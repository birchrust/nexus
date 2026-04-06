#![allow(dead_code)] // Wired into executor + channel in subsequent commits.
//! Cross-thread wake infrastructure.
//!
//! An intrusive MPSC queue (Vyukov style) for waking tasks from other
//! threads. Each task's header contains an `AtomicPtr<u8>` (`cross_next`)
//! used as the intrusive link — zero allocation per wake.
//!
//! The queue is paired with a `mio::Waker` (eventfd) to interrupt the
//! runtime's epoll when a cross-thread wake arrives.
//!
//! Local wakes (same thread) continue using the fast TLS Vec path.
//! Cross-thread wakes use this queue + eventfd. The executor drains
//! both on each poll cycle.

use std::cell::{Cell, UnsafeCell};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

use crate::task;

// =============================================================================
// TLS — cross-wake context accessible during block_on
// =============================================================================

thread_local! {
    static CTX_CROSS_WAKE: Cell<*const Arc<CrossWakeContext>> =
        const { Cell::new(std::ptr::null()) };
}

/// Install the cross-wake context in TLS. Returns a guard that clears
/// it on drop.
pub(crate) fn install_cross_wake(ctx: &Arc<CrossWakeContext>) -> CrossWakeGuard {
    let prev = CTX_CROSS_WAKE.with(|c| c.replace(ctx as *const Arc<CrossWakeContext>));
    CrossWakeGuard { prev }
}

pub(crate) struct CrossWakeGuard {
    prev: *const Arc<CrossWakeContext>,
}

impl Drop for CrossWakeGuard {
    fn drop(&mut self) {
        CTX_CROSS_WAKE.with(|c| c.set(self.prev));
    }
}

/// Get the current runtime's cross-wake context. Returns None if
/// called outside `block_on`.
pub(crate) fn cross_wake_context() -> Option<Arc<CrossWakeContext>> {
    CTX_CROSS_WAKE.with(|c| {
        let ptr = c.get();
        if ptr.is_null() {
            None
        } else {
            // SAFETY: ptr was set by install_cross_wake and is valid
            // for the duration of block_on.
            Some(unsafe { (*ptr).clone() })
        }
    })
}

// =============================================================================
// Intrusive MPSC queue (Vyukov)
// =============================================================================

/// Lock-free MPSC queue for cross-thread task wake notifications.
///
/// Producers (any thread) push task pointers via atomic swap on the tail.
/// The single consumer (runtime thread) drains via the head.
///
/// Each task's `cross_next` field (offset 32 in the header) serves as
/// the intrusive link pointer. No heap allocation per push.
///
/// Uses a stub node to avoid the empty-queue edge case. The stub is
/// just an `AtomicPtr` — not a real task.
pub(crate) struct CrossWakeQueue {
    /// Consumer reads from here. Only touched by the runtime thread.
    /// Wrapped in UnsafeCell so pop() can take &self (interior mutability).
    head: UnsafeCell<*mut u8>,
    /// Producers CAS here. Shared across threads.
    tail: AtomicPtr<u8>,
    /// Heap-allocated stub node. Stable address across moves.
    /// The stub is just an `AtomicPtr<u8>` (the "next" pointer).
    stub: *mut AtomicPtr<u8>,
}

// SAFETY: The queue is designed for cross-thread use.
// Producers push from any thread (atomic tail swap).
// Consumer pops from one thread (head is non-atomic).
unsafe impl Send for CrossWakeQueue {}
unsafe impl Sync for CrossWakeQueue {}

impl CrossWakeQueue {
    /// Create a new empty queue.
    pub(crate) fn new() -> Self {
        // Heap-allocate the stub so its address is stable after moves.
        let stub = Box::into_raw(Box::new(AtomicPtr::new(std::ptr::null_mut())));
        let stub_as_node = stub.cast::<u8>();
        Self {
            head: UnsafeCell::new(stub_as_node),
            tail: AtomicPtr::new(stub_as_node),
            stub,
        }
    }

    /// The stub's "task pointer" — the heap-allocated AtomicPtr.
    #[inline]
    fn stub_ptr(&self) -> *mut u8 {
        self.stub.cast::<u8>()
    }

    /// Get the `cross_next` pointer for a node. For real tasks this is
    /// the AtomicPtr at offset 32. For the stub it IS the stub allocation.
    #[inline]
    unsafe fn next_of(&self, node: *mut u8) -> &AtomicPtr<u8> {
        if node == self.stub_ptr() {
            // SAFETY: stub is a valid heap-allocated AtomicPtr.
            unsafe { &*self.stub }
        } else {
            // SAFETY: caller guarantees `node` is a valid task pointer.
            unsafe { task::cross_next(node) }
        }
    }

}

impl Drop for CrossWakeQueue {
    fn drop(&mut self) {
        // SAFETY: stub was allocated via Box::into_raw in new().
        unsafe { drop(Box::from_raw(self.stub)) };
    }
}

impl CrossWakeQueue {
    /// Push a task pointer into the queue. Thread-safe (any thread).
    ///
    /// # Safety
    ///
    /// `task_ptr` must point to a live task with a valid `cross_next` field,
    /// OR must be the stub pointer (internal re-insertion).
    /// The task must not already be in this queue.
    pub(crate) unsafe fn push(&self, task_ptr: *mut u8) {
        // Clear next pointer on the node we're pushing.
        // SAFETY: task_ptr is either a valid task or the stub.
        unsafe { self.next_of(task_ptr) }.store(std::ptr::null_mut(), Ordering::Relaxed);

        // Atomically swap ourselves into the tail position.
        let prev = self.tail.swap(task_ptr, Ordering::AcqRel);

        // Link the previous tail to us. The consumer will see this
        // once the Release from our swap is visible.
        // SAFETY: prev is either the stub or a previously pushed task.
        unsafe { self.next_of(prev) }.store(task_ptr, Ordering::Release);
    }

    /// Pop a task pointer from the queue. Single-consumer only.
    ///
    /// Takes `&self` using interior mutability for `head` (UnsafeCell).
    /// The single-consumer guarantee ensures no concurrent access to `head`.
    ///
    /// Returns `None` if the queue is empty (or a producer hasn't
    /// finished linking yet — transient inconsistency).
    pub(crate) fn pop(&self) -> Option<*mut u8> {
        // SAFETY: single-consumer guarantee — only the runtime thread
        // calls pop(), never concurrently.
        let head_ref = unsafe { &mut *self.head.get() };
        let mut head = *head_ref;
        // SAFETY: head is either the stub or a previously pushed task.
        let mut next = unsafe { self.next_of(head) }.load(Ordering::Acquire);

        let stub = self.stub_ptr();

        // Skip the stub node.
        if head == stub {
            if next.is_null() {
                return None; // Queue is empty.
            }
            *head_ref = next;
            head = next;
            next = unsafe { self.next_of(head) }.load(Ordering::Acquire);
        }

        // Normal case: head has a next -> pop head, advance.
        if !next.is_null() {
            *head_ref = next;
            return Some(head);
        }

        // head is the last node. Check if tail == head.
        let tail = self.tail.load(Ordering::Acquire);
        if head != tail {
            // A producer swapped tail but hasn't linked next yet.
            // Transient inconsistency — return None, retry later.
            return None;
        }

        // Re-insert stub so we don't lose the tail reference.
        // SAFETY: stub is always valid.
        unsafe { self.push(stub) };

        // Now check if head got a next pointer (the stub push linked it).
        next = unsafe { self.next_of(head) }.load(Ordering::Acquire);
        if !next.is_null() {
            *head_ref = next;
            return Some(head);
        }

        None
    }
}

// =============================================================================
// Cross-thread waker data
// =============================================================================

/// Shared context for all cross-thread wakers in a runtime instance.
/// Created once per runtime, `Arc`-shared across all cross-thread wakers.
pub(crate) struct CrossWakeContext {
    /// The intrusive MPSC queue for cross-thread wake pushes.
    pub(crate) queue: CrossWakeQueue,
    /// The mio waker to interrupt epoll after pushing.
    pub(crate) mio_waker: Arc<mio::Waker>,
    /// Whether the runtime is currently parked in epoll_wait.
    /// Cross-thread senders read this to decide whether to poke
    /// the eventfd — skip the syscall when the runtime is actively
    /// polling (it will drain the inbox on the next iteration).
    pub(crate) parked: std::sync::atomic::AtomicBool,
}

// SAFETY: CrossWakeQueue is Send + Sync, Arc<mio::Waker> is Send + Sync.
unsafe impl Send for CrossWakeContext {}
unsafe impl Sync for CrossWakeContext {}

/// Wake a task via the cross-thread path: push to intrusive inbox,
/// conditionally poke eventfd. Zero allocation.
///
/// # Safety
///
/// `task_ptr` must point to a live task. `ctx` must be a valid
/// `CrossWakeContext` (guaranteed by channel lifetime).
pub(crate) unsafe fn wake_task_cross_thread(
    task_ptr: *mut u8,
    ctx: &CrossWakeContext,
) {
    // Don't wake completed tasks.
    if unsafe { task::is_completed(task_ptr) } {
        return;
    }

    // Dedup: atomic CAS on is_queued for thread safety.
    // SAFETY: task_ptr is a valid task.
    if !unsafe { task::try_set_queued(task_ptr) } {
        return;
    }

    // SAFETY: task_ptr valid, not already queued.
    unsafe { ctx.queue.push(task_ptr) };

    if ctx.parked.load(Ordering::Acquire) {
        let _ = ctx.mio_waker.wake();
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Task;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    struct Noop;
    impl Future for Noop {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }

    fn make_task() -> *mut u8 {
        let task = Box::new(Task::new_boxed(Noop, 0));
        Box::into_raw(task) as *mut u8
    }

    unsafe fn free(ptr: *mut u8) {
        unsafe { task::free_task(ptr) };
    }

    #[test]
    fn queue_push_pop_single() {
        let q = CrossWakeQueue::new();
        let t1 = make_task();

        unsafe { q.push(t1) };
        assert_eq!(q.pop(), Some(t1));
        assert_eq!(q.pop(), None);

        unsafe { free(t1) };
    }

    #[test]
    fn queue_push_pop_multiple() {
        let q = CrossWakeQueue::new();
        let t1 = make_task();
        let t2 = make_task();
        let t3 = make_task();

        unsafe { q.push(t1) };
        unsafe { q.push(t2) };
        unsafe { q.push(t3) };

        assert_eq!(q.pop(), Some(t1));
        assert_eq!(q.pop(), Some(t2));
        assert_eq!(q.pop(), Some(t3));
        assert_eq!(q.pop(), None);

        unsafe { free(t1) };
        unsafe { free(t2) };
        unsafe { free(t3) };
    }

    #[test]
    fn queue_interleaved_push_pop() {
        let q = CrossWakeQueue::new();
        let t1 = make_task();
        let t2 = make_task();

        unsafe { q.push(t1) };
        assert_eq!(q.pop(), Some(t1));

        unsafe { q.push(t2) };
        assert_eq!(q.pop(), Some(t2));
        assert_eq!(q.pop(), None);

        unsafe { free(t1) };
        unsafe { free(t2) };
    }

    #[test]
    fn queue_empty() {
        let q = CrossWakeQueue::new();
        assert_eq!(q.pop(), None);
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn queue_reuse_after_drain() {
        let q = CrossWakeQueue::new();
        let t1 = make_task();

        for _ in 0..100 {
            unsafe { q.push(t1) };
            assert_eq!(q.pop(), Some(t1));
        }
        assert_eq!(q.pop(), None);

        unsafe { free(t1) };
    }
}
