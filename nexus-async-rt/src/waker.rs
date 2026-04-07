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
    let prev_ready = READY_QUEUE.with(|cell| cell.replace(ready as *mut Vec<*mut u8>));
    let prev_free = DEFERRED_FREE.with(|cell| cell.replace(deferred_free as *mut Vec<*mut u8>));
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

/// Extract the task pointer from a local waker.
///
/// Returns the task `*mut u8` if the waker uses our local vtable.
/// Returns `None` if it's a different waker (cross-thread, root, etc.).
///
/// # Safety
///
/// The waker must have been created by this runtime's `ReusableWaker`.
pub(crate) fn task_ptr_from_local_waker(waker: &Waker) -> Option<*mut u8> {
    // Waker layout: [vtable_ptr, data_ptr] — two pointers at offset 0 and 8.
    // SAFETY: Waker is repr(transparent) over RawWaker which is [*const (), *const ()].
    let raw: &[*const (); 2] = unsafe { &*(waker as *const Waker).cast::<[*const (); 2]>() };
    let vtable_ptr = raw[0];
    let data_ptr = raw[1];

    if vtable_ptr == (&raw const VTABLE).cast::<()>() {
        Some(data_ptr as *mut u8)
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
    use std::task::{Poll, RawWaker, Waker};

    /// Validate Waker layout assumptions used by `task_ptr_from_local_waker`.
    /// If Rust changes the layout of Waker, this test fails.
    #[test]
    fn waker_layout_matches_assumptions() {
        // Waker must be [vtable, data] = 16 bytes
        assert_eq!(std::mem::size_of::<Waker>(), 16);
        assert_eq!(std::mem::align_of::<Waker>(), 8);

        // Verify field order: construct a Waker and check byte layout
        let sentinel = 0xDEAD_BEEF_u64 as *const ();
        let raw = RawWaker::new(sentinel, &VTABLE);
        let waker = std::mem::ManuallyDrop::new(unsafe { Waker::from_raw(raw) });

        let bytes: &[u64; 2] = unsafe { &*(&*waker as *const Waker as *const [u64; 2]) };

        // First field should be vtable pointer, second should be data
        assert_eq!(
            bytes[0],
            (&raw const VTABLE) as u64,
            "Waker layout changed: vtable not at offset 0"
        );
        assert_eq!(
            bytes[1], sentinel as u64,
            "Waker layout changed: data not at offset 8"
        );
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
