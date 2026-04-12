//! Refcounted single-threaded waker.
//!
//! The waker stores the task's raw pointer directly in the `RawWaker` data
//! field. No `Box`, no `Arc` — the waker is two pointers (vtable + data).
//!
//! **Clone** increments the task's `ref_count` (AtomicU16) and copies the pointer.
//! **Drop** decrements `ref_count`; if it hits 0 on a completed task, the slot
//! is pushed to the deferred free list. **Wake** pushes the task pointer to the
//! TLS ready queue (with `is_queued` dedup) and then decrements `ref_count`
//! (consuming the waker).
//!
//! # Safety
//!
//! Single-threaded only. The ready queue and deferred free list pointers in TLS
//! must be valid during the entire poll cycle.
//!
//! # Waker Lifetime
//!
//! Wakers hold a ref to the task via `ref_count`. The task slot stays alive as
//! long as any waker, JoinHandle, or the executor holds a reference. When the
//! last ref drops (refcount hits 0), the slot is deferred for freeing.

use std::task::{RawWaker, RawWakerVTable, Waker};

use crate::task;

// =============================================================================
// Thread-local ready queue for wakers
// =============================================================================

std::thread_local! {
    /// Raw pointer to the executor's ready queue. Set before polling,
    /// cleared after. Wakers read this to push their task pointer.
    static READY_QUEUE: std::cell::Cell<*mut Vec<*mut u8>> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };

    /// Deferred free list: task pointers whose refcount hit 0 after
    /// completion. The executor drains this on each poll cycle.
    static DEFERRED_FREE: std::cell::Cell<*mut Vec<*mut u8>> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Install TLS pointers for the duration of a poll cycle.
/// Returns an RAII guard that restores previous values on drop.
#[inline]
pub(crate) fn set_poll_context(
    ready: &mut Vec<*mut u8>,
    deferred_free: &mut Vec<*mut u8>,
) -> PollContextGuard {
    let prev_ready = READY_QUEUE.with(|cell| cell.replace(std::ptr::from_mut(ready)));
    let prev_free = DEFERRED_FREE.with(|cell| cell.replace(std::ptr::from_mut(deferred_free)));
    PollContextGuard {
        prev_ready,
        prev_free,
    }
}

pub(crate) struct PollContextGuard {
    prev_ready: *mut Vec<*mut u8>,
    prev_free: *mut Vec<*mut u8>,
}

impl Drop for PollContextGuard {
    #[inline]
    fn drop(&mut self) {
        READY_QUEUE.with(|cell| cell.set(self.prev_ready));
        DEFERRED_FREE.with(|cell| cell.set(self.prev_free));
    }
}

// =============================================================================
// RawWaker vtable
// =============================================================================

pub(crate) static VTABLE: RawWakerVTable =
    RawWakerVTable::new(clone_fn, wake_fn, wake_by_ref_fn, drop_fn);

/// Create a `Waker` for a task. Increments `ref_count` to account for
/// the waker's reference. The waker's `drop_fn` will decrement it.
///
/// # Safety
///
/// `ptr` must point to a live task with `ref_count >= 1`.
#[inline]
pub(crate) unsafe fn task_waker(ptr: *mut u8) -> Waker {
    unsafe { task::ref_inc(ptr) };
    let raw = RawWaker::new(ptr.cast(), &VTABLE);
    unsafe { Waker::from_raw(raw) }
}

/// Extract the task pointer from a waker if it belongs to this runtime.
///
/// Returns the task `*mut u8` if the waker uses our local vtable.
/// Returns `None` if it's a different waker (cross-thread, root, etc.).
pub(crate) fn task_ptr_from_local_waker(waker: &Waker) -> Option<*mut u8> {
    if waker.vtable() == &VTABLE {
        Some(waker.data() as *mut u8)
    } else {
        None
    }
}

/// Clone: increment refcount, copy the pointer.
unsafe fn clone_fn(data: *const ()) -> RawWaker {
    // SAFETY: data points to a live (or completed) task.
    unsafe { task::ref_inc(data as *mut u8) };
    RawWaker::new(data, &VTABLE)
}

/// Wake (by value): push to ready queue, decrement refcount (consumes waker).
/// If the task is completed and refcount hits 0, signals slot can be freed.
unsafe fn wake_fn(data: *const ()) {
    // SAFETY: data is a valid task pointer.
    unsafe { wake_impl(data) };
    // Consume: decrement refcount. If slot should be freed, call the
    // free callback via TLS.
    let should_free = unsafe { task::ref_dec(data as *mut u8) };
    if should_free {
        unsafe { free_completed_slot(data as *mut u8) };
    }
}

/// Wake (by ref): push to ready queue. Does NOT decrement refcount
/// (the waker is borrowed, not consumed).
unsafe fn wake_by_ref_fn(data: *const ()) {
    // SAFETY: data is a valid task pointer.
    unsafe { wake_impl(data) };
}

/// Drop (without waking): decrement refcount. If the task is completed
/// and refcount hits 0, free the slot.
unsafe fn drop_fn(data: *const ()) {
    let should_free = unsafe { task::ref_dec(data as *mut u8) };
    if should_free {
        unsafe { free_completed_slot(data as *mut u8) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{RawWaker, Waker};

    #[test]
    fn task_ptr_from_local_waker_roundtrip() {
        let sentinel = 0xDEAD_BEEF_usize as *mut u8;
        let waker = unsafe { Waker::from_raw(RawWaker::new(sentinel.cast(), &VTABLE)) };
        let waker = std::mem::ManuallyDrop::new(waker);

        let ptr = task_ptr_from_local_waker(&waker);
        assert_eq!(ptr, Some(sentinel));
    }

    #[test]
    fn task_ptr_from_foreign_waker_returns_none() {
        static OTHER: RawWakerVTable =
            RawWakerVTable::new(|p| RawWaker::new(p, &OTHER), |_| {}, |_| {}, |_| {});
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &OTHER)) };
        let waker = std::mem::ManuallyDrop::new(waker);

        assert!(task_ptr_from_local_waker(&waker).is_none());
    }
}

/// Queue a completed task slot for deferred freeing.
///
/// Called when the last reference drops (refcount hits 0) — either from
/// a waker drop or from JoinHandle::Drop.
///
/// If called outside a poll cycle (DEFERRED_FREE TLS is null), the slot
/// is **not freed** — it will be reclaimed by `Executor::drop`. This is
/// acceptable for correctness but means tasks whose last ref drops outside
/// `block_on` are cleaned up lazily.
///
/// # Safety
///
/// `ptr` must point to a completed task slot.
#[cold]
#[inline(never)]
pub(crate) unsafe fn defer_free(ptr: *mut u8) {
    unsafe { free_completed_slot(ptr) };
}

/// Queue a completed task slot for deferred freeing. Called when the
/// last waker clone drops (refcount hits 0).
///
/// # Safety
///
/// `ptr` must point to a completed task slot.
#[cold]
#[inline(never)]
unsafe fn free_completed_slot(ptr: *mut u8) {
    DEFERRED_FREE.with(|cell| {
        let list_ptr = cell.get();
        if !list_ptr.is_null() {
            // SAFETY: list_ptr valid — set by set_poll_context.
            let list = unsafe { &mut *list_ptr };
            list.push(ptr);
        }
        // If null (outside poll cycle), the slot leaks. This is
        // acceptable — it will be cleaned up on Executor::drop.
    });
}

/// Shared wake implementation.
///
/// # Safety
///
/// `data` must be a valid task pointer from the byte slab.
/// The ready queue TLS must be set (we're inside a poll cycle).
unsafe fn wake_impl(data: *const ()) {
    let task_ptr = data as *mut u8;

    // Don't wake completed tasks — the future is already dropped.
    // SAFETY: task_ptr points to a (possibly completed) task.
    if unsafe { task::is_completed(task_ptr) } {
        return;
    }

    // Check dedup flag — don't queue twice.
    if unsafe { task::is_queued(task_ptr) } {
        return;
    }
    unsafe { task::set_queued(task_ptr, true) };

    // Push to ready queue.
    READY_QUEUE.with(|cell| {
        let queue_ptr = cell.get();
        debug_assert!(
            !queue_ptr.is_null(),
            "waker fired outside poll cycle — task will be lost. \
             Ensure wakers are only used within Runtime::block_on or \
             Executor::poll scope."
        );
        if !queue_ptr.is_null() {
            // SAFETY: queue_ptr is valid — set by set_poll_context before
            // polling. Single-threaded, no concurrent access.
            let queue = unsafe { &mut *queue_ptr };
            queue.push(task_ptr);
        }
    });
}
