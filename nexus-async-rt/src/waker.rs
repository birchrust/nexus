//! Zero-allocation single-threaded waker.
//!
//! The waker stores the task's raw pointer (from the byte slab) directly
//! in the `RawWaker` data field. `wake()` sets the task's `is_queued`
//! flag and pushes the pointer to the ready queue (obtained from TLS).
//!
//! No `Box`, no `Arc`, no atomics. Clone is a pointer copy. Drop is a no-op.
//!
//! # Safety
//!
//! Single-threaded only. The ready queue pointer in TLS must be valid
//! during the entire poll cycle.
//!
//! # Waker Lifetime Invariant
//!
//! Wakers are **non-owning** raw pointers to task slab slots. They do
//! not prevent the slot from being freed. This means:
//!
//! - A waker must not be used after its task is cancelled or completed.
//! - IO registrations and timer entries that hold wakers must be cleaned
//!   up before the task slot is freed.
//! - In practice, the single-threaded executor enforces this: timers fire
//!   before cancel, IO sources are deregistered before cancel, and task
//!   completion happens synchronously during poll (no concurrent free).
//!
//! A future improvement could add a lightweight in-task refcount
//! (incremented on clone, decremented on drop/wake) so the slot stays
//! valid until all wakers are dropped. For now, the single-threaded
//! invariant is sufficient.

use std::task::{Context, RawWaker, RawWakerVTable};

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
    let prev_ready = READY_QUEUE.with(|cell| cell.replace(ready as *mut Vec<*mut u8>));
    let prev_free = DEFERRED_FREE.with(|cell| cell.replace(deferred_free as *mut Vec<*mut u8>));
    PollContextGuard { prev_ready, prev_free }
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
// Waker construction
// =============================================================================

// =============================================================================
// Reusable waker for poll loops
// =============================================================================

/// Pre-built waker + context for the poll loop. The vtable and Context
/// layout are set once; only the data pointer (task pointer) is updated
/// per iteration.
///
/// # Layout
///
/// `raw[0..2]` = Waker: `[vtable_ptr, data_ptr]`
/// `raw[2..6]` = Context: `[&Waker, &Waker, 0, 0]` (32 bytes)
///
/// The Context fields are self-referential pointers to `raw[0]`.
/// Call `init()` after construction to set them — must not be moved after.
pub(crate) struct ReusableWaker {
    raw: [*const (); 6],
}

impl ReusableWaker {
    /// Build the waker skeleton. Vtable is set; self-refs are NOT set.
    /// Call `init()` before first `set_task()`.
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            raw: [
                (&raw const VTABLE).cast::<()>(), // vtable (constant)
                std::ptr::null(),                   // data (updated per task)
                std::ptr::null(),                   // &Waker (set by init)
                std::ptr::null(),                   // &Waker duplicate
                std::ptr::null(),                   // _ExtendedContext pad
                std::ptr::null(),                   // _ExtendedContext pad
            ],
        }
    }

    /// Set self-referential pointers. Must be called exactly once,
    /// after the struct is at its final stack location (no moves after).
    #[inline]
    pub(crate) fn init(&mut self) {
        let waker_ptr = self.raw.as_ptr().cast::<()>();
        self.raw[2] = waker_ptr;
        self.raw[3] = waker_ptr;
    }

    /// Update the data pointer and return a `&mut Context` ready for
    /// polling.
    ///
    /// # Safety
    ///
    /// `task_ptr` must point to a live task in the slab.
    /// `init()` must have been called. `self` must not have been moved
    /// since `init()`.
    #[inline]
    pub(crate) unsafe fn set_task(&mut self, task_ptr: *mut u8) -> &mut Context<'_> {
        self.raw[1] = task_ptr.cast::<()>();
        // SAFETY: raw[0..2] has Waker layout [vtable, data].
        // raw[2..5] has Context layout [&Waker, &Waker, null].
        // Self-ref pointers set by init() point to raw[0]. Caller
        // guarantees no move since init().
        unsafe { &mut *(self.raw.as_mut_ptr().add(2).cast::<Context<'_>>()) }
    }
}

// =============================================================================
// RawWaker vtable
// =============================================================================

static VTABLE: RawWakerVTable = RawWakerVTable::new(clone_fn, wake_fn, wake_by_ref_fn, drop_fn);

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
    use std::task::{Poll, RawWaker, Waker};

    /// Validate that our ReusableWaker layout matches Waker + Context.
    /// If Rust changes the layout of Waker or Context, this test fails
    /// and we know the ReusableWaker is unsound.
    #[test]
    fn reusable_waker_layout_matches_std() {
        // Waker must be [vtable, data] = 16 bytes
        assert_eq!(std::mem::size_of::<Waker>(), 16);
        assert_eq!(std::mem::align_of::<Waker>(), 8);

        // Verify field order: construct a Waker and check byte layout
        let sentinel = 0xDEAD_BEEF_u64 as *const ();
        let raw = RawWaker::new(sentinel, &VTABLE);
        let waker = std::mem::ManuallyDrop::new(unsafe { Waker::from_raw(raw) });

        let bytes: &[u64; 2] =
            unsafe { &*(&*waker as *const Waker as *const [u64; 2]) };

        // First field should be vtable pointer, second should be data
        assert_eq!(
            bytes[0],
            (&raw const VTABLE) as u64,
            "Waker layout changed: vtable not at offset 0"
        );
        assert_eq!(
            bytes[1],
            sentinel as u64,
            "Waker layout changed: data not at offset 8"
        );
    }

    /// Validate that ReusableWaker produces a functional Context.
    /// The future receives the correct task pointer through the waker.
    #[test]
    fn reusable_waker_delivers_correct_task_ptr() {
        use crate::task::Task;
        use std::future::Future;
        use std::pin::Pin;

        struct Noop;
        impl Future for Noop {
            type Output = ();
            fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                Poll::Ready(())
            }
        }

        // Allocate real task headers so clone/drop can touch refcount.
        let task_a = Box::new(Task::new_boxed(Noop, 0));
        let task_b = Box::new(Task::new_boxed(Noop, 0));
        let ptr_a = Box::into_raw(task_a) as *mut u8;
        let ptr_b = Box::into_raw(task_b) as *mut u8;

        let mut reusable = ReusableWaker::new();
        reusable.init();

        // First task
        let cx = unsafe { reusable.set_task(ptr_a) };
        // refcount starts at 1 (executor ref). Clone adds 1 → 2.
        assert_eq!(unsafe { crate::task::ref_count(ptr_a) }, 1);
        let cloned = cx.waker().clone();
        let raw_a: &[u64; 2] =
            unsafe { &*(&cloned as *const Waker as *const [u64; 2]) };
        assert_eq!(raw_a[1], ptr_a as u64);
        assert_eq!(unsafe { crate::task::ref_count(ptr_a) }, 2);
        drop(cloned); // decrements → 1 (executor ref remains)
        assert_eq!(unsafe { crate::task::ref_count(ptr_a) }, 1);

        // Second task — same ReusableWaker, different pointer
        let cx = unsafe { reusable.set_task(ptr_b) };
        assert_eq!(unsafe { crate::task::ref_count(ptr_b) }, 1);
        let cloned = cx.waker().clone();
        let raw_b: &[u64; 2] =
            unsafe { &*(&cloned as *const Waker as *const [u64; 2]) };
        assert_eq!(raw_b[1], ptr_b as u64);
        assert_eq!(unsafe { crate::task::ref_count(ptr_b) }, 2);
        drop(cloned);
        assert_eq!(unsafe { crate::task::ref_count(ptr_b) }, 1);

        // Clean up.
        unsafe {
            drop(Box::from_raw(ptr_a as *mut Task<Noop>));
            drop(Box::from_raw(ptr_b as *mut Task<Noop>));
        }
    }

    /// Validate Context layout: Context wraps &Waker.
    /// Our ReusableWaker stores the Context fields at raw[2..5].
    #[test]
    fn context_layout_matches_assumption() {
        // Build a normal Context from a real Waker and inspect bytes.
        let raw = RawWaker::new(std::ptr::null(), &VTABLE);
        let waker = std::mem::ManuallyDrop::new(unsafe { Waker::from_raw(raw) });
        let cx = Context::from_waker(&waker);

        let cx_size = std::mem::size_of::<Context<'_>>();
        // Context should be 4 pointers: [&Waker, &Waker, 0, 0] = 32 bytes.
        assert!(
            cx_size <= 32,
            "Context size {} exceeds our 32-byte allocation",
            cx_size
        );

        // The first field should be &Waker pointing to our waker
        let cx_bytes: &[u64] =
            unsafe { std::slice::from_raw_parts(&cx as *const _ as *const u64, cx_size / 8) };
        assert_eq!(
            cx_bytes[0],
            &*waker as *const Waker as u64,
            "Context first field is not &Waker"
        );
    }
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
