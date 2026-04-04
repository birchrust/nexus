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

use std::task::{Context, RawWaker, RawWakerVTable, Waker};

use crate::task;

// =============================================================================
// Thread-local ready queue for wakers
// =============================================================================

std::thread_local! {
    /// Raw pointer to the executor's ready queue. Set before polling,
    /// cleared after. Wakers read this to push their task pointer.
    static READY_QUEUE: std::cell::Cell<*mut Vec<*mut u8>> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Install the ready queue pointer for the duration of a poll cycle.
/// Returns an RAII guard that clears it on drop.
#[inline]
pub(crate) fn set_ready_queue(queue: &mut Vec<*mut u8>) -> ReadyQueueGuard {
    let prev = READY_QUEUE.with(|cell| cell.replace(queue as *mut Vec<*mut u8>));
    ReadyQueueGuard { prev }
}

pub(crate) struct ReadyQueueGuard {
    prev: *mut Vec<*mut u8>,
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
#[allow(dead_code)]
pub(crate) fn task_waker(task_ptr: *mut u8) -> Waker {
    let raw = RawWaker::new(task_ptr.cast::<()>(), &VTABLE);
    // SAFETY: The vtable is correct for our task pointer convention.
    // Single-threaded — no Send/Sync concern.
    unsafe { Waker::from_raw(raw) }
}

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
            queue.push(ptr);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{RawWaker, Waker};

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
        let mut reusable = ReusableWaker::new();
        reusable.init();

        let sentinel_a = 0x1111_u64 as *mut u8;
        let sentinel_b = 0x2222_u64 as *mut u8;

        // First task
        let cx = unsafe { reusable.set_task(sentinel_a) };
        let cloned = cx.waker().clone();
        let raw_a: &[u64; 2] =
            unsafe { &*(&cloned as *const Waker as *const [u64; 2]) };
        assert_eq!(raw_a[1], sentinel_a as u64);
        // drop cloned waker (no-op drop, but keeps LLVM from optimizing out)
        drop(cloned);

        // Second task — same ReusableWaker, different pointer
        let cx = unsafe { reusable.set_task(sentinel_b) };
        let cloned = cx.waker().clone();
        let raw_b: &[u64; 2] =
            unsafe { &*(&cloned as *const Waker as *const [u64; 2]) };
        assert_eq!(raw_b[1], sentinel_b as u64);
        drop(cloned);
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
            queue.push(task_ptr);
        }
        // If null, we're outside a poll cycle. The task will be picked
        // up on the next poll since is_queued is set.
    });
}
