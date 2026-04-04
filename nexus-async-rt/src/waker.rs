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

use std::collections::VecDeque;
use std::task::{RawWaker, RawWakerVTable, Waker};

use crate::task;

// =============================================================================
// Thread-local ready queue for wakers
// =============================================================================

std::thread_local! {
    /// Raw pointer to the executor's ready queue. Set before polling,
    /// cleared after. Wakers read this to push their task pointer.
    static READY_QUEUE: std::cell::Cell<*mut VecDeque<*mut u8>> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Install the ready queue pointer for the duration of a poll cycle.
/// Returns an RAII guard that clears it on drop.
#[inline]
pub(crate) fn set_ready_queue(queue: &mut VecDeque<*mut u8>) -> ReadyQueueGuard {
    let prev = READY_QUEUE.with(|cell| cell.replace(queue as *mut VecDeque<*mut u8>));
    ReadyQueueGuard { prev }
}

pub(crate) struct ReadyQueueGuard {
    prev: *mut VecDeque<*mut u8>,
}

impl Drop for ReadyQueueGuard {
    #[inline]
    fn drop(&mut self) {
        READY_QUEUE.with(|cell| cell.set(self.prev));
    }
}

// =============================================================================
// Waker construction
// =============================================================================

/// Create a waker for the given task pointer.
///
/// The task pointer is stored directly in the `RawWaker` data field.
/// Zero allocation.
#[inline]
pub(crate) fn task_waker(task_ptr: *mut u8) -> Waker {
    let raw = RawWaker::new(task_ptr.cast::<()>(), &VTABLE);
    // SAFETY: The vtable is correct for our task pointer convention.
    // Single-threaded — no Send/Sync concern.
    unsafe { Waker::from_raw(raw) }
}

// =============================================================================
// RawWaker vtable
// =============================================================================

static VTABLE: RawWakerVTable = RawWakerVTable::new(clone_fn, wake_fn, wake_by_ref_fn, drop_fn);

/// Clone: just copy the pointer. No allocation.
unsafe fn clone_fn(data: *const ()) -> RawWaker {
    RawWaker::new(data, &VTABLE)
}

/// Wake (by value): set is_queued, push to ready queue, consume waker.
unsafe fn wake_fn(data: *const ()) {
    // SAFETY: data is a valid task pointer. Caller (waker contract)
    // guarantees it was produced by task_waker.
    unsafe { wake_impl(data) };
}

/// Wake (by ref): set is_queued, push to ready queue.
unsafe fn wake_by_ref_fn(data: *const ()) {
    // SAFETY: same as wake_fn.
    unsafe { wake_impl(data) };
}

/// Drop: no-op. The waker doesn't own any resources.
unsafe fn drop_fn(_data: *const ()) {}

/// Push a task pointer to the ready queue. Used by the IO driver
/// to wake tasks directly (bypasses the waker vtable path).
///
/// # Safety
///
/// `ptr` must point to a live task. The ready queue TLS must be set.
#[inline]
pub(crate) unsafe fn push_ready(ptr: *mut u8) {
    READY_QUEUE.with(|cell| {
        let queue_ptr = cell.get();
        if !queue_ptr.is_null() {
            // SAFETY: queue_ptr valid — set by set_ready_queue.
            let queue = unsafe { &mut *queue_ptr };
            queue.push_back(ptr);
        }
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

    // Check dedup flag — don't queue twice.
    // SAFETY: task_ptr points to a live Task in the slab.
    if unsafe { task::is_queued(task_ptr) } {
        return;
    }
    unsafe { task::set_queued(task_ptr, true) };

    // Push to ready queue.
    READY_QUEUE.with(|cell| {
        let queue_ptr = cell.get();
        if !queue_ptr.is_null() {
            // SAFETY: queue_ptr is valid — set by set_ready_queue before
            // polling. Single-threaded, no concurrent access.
            let queue = unsafe { &mut *queue_ptr };
            queue.push_back(task_ptr);
        }
        // If null, we're outside a poll cycle. The task will be picked
        // up on the next poll since is_queued is set.
    });
}
