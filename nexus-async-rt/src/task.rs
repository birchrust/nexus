//! Task storage: header + future bytes in a single byte slab slot.
//!
//! Each task is a `Task<F>` struct allocated via placement new into the
//! byte slab. The raw pointer to the slab slot IS the task handle —
//! no index layer, no separate metadata store.
//!
//! The waker holds the raw pointer directly. `wake()` sets `is_queued`
//! and pushes the pointer to the ready queue. Zero allocations.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

// =============================================================================
// Task layout
// =============================================================================

/// Header size in bytes. Must match the layout of `Task<F>` before the
/// `future` field. Used to compute internal slab slot size.
pub const TASK_HEADER_SIZE: usize = 24;

/// Task stored in the byte slab. `repr(C)` for deterministic layout.
///
/// The raw pointer to this struct is the task handle, the waker data,
/// and the ready queue entry — all the same pointer.
///
/// Layout (64-bit):
/// ```text
/// offset  0: poll_fn    (8 bytes, fn pointer)
/// offset  8: drop_fn    (8 bytes, fn pointer)
/// offset 16: is_queued  (1 byte, bool)
/// offset 17: _pad       (7 bytes, alignment padding)
/// offset 24: future     (F bytes, the actual future)
/// ```
#[repr(C)]
pub(crate) struct Task<F> {
    poll_fn: unsafe fn(*mut u8, &mut Context<'_>) -> Poll<()>,
    drop_fn: unsafe fn(*mut u8),
    is_queued: bool,
    /// Explicit padding ensures TASK_HEADER_SIZE is always 24 regardless
    /// of F's alignment. Without this, repr(C) would pad to F's alignment
    /// — a future with align(1) would put `future` at offset 17, not 24.
    /// The hardcoded offsets in `is_queued()`/`set_queued()`/`poll_task()`
    /// depend on this being exactly 24.
    _pad: [u8; 7],
    future: F,
}

// Static assertion: header layout matches TASK_HEADER_SIZE.
const _: () = {
    // Task with ZST future — size is just the header.
    assert!(std::mem::size_of::<Task<()>>() == TASK_HEADER_SIZE);
};

impl<F: Future<Output = ()> + 'static> Task<F> {
    /// Construct a task. Called once at spawn time.
    #[inline]
    pub(crate) fn new(future: F) -> Self {
        Self {
            poll_fn: poll_fn::<F>,
            drop_fn: drop_fn::<F>,
            is_queued: false,
            _pad: [0; 7],
            future,
        }
    }
}

// =============================================================================
// Task handle — raw pointer operations
// =============================================================================

/// Opaque task identifier. Wraps the raw pointer to the task in the
/// byte slab. The pointer is stable for the task's lifetime (slab
/// memory never moves).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub(crate) *mut u8);

impl TaskId {
    /// Returns the raw pointer to the task.
    pub fn as_ptr(&self) -> *mut u8 {
        self.0
    }
}

/// Read the `is_queued` flag from a task pointer.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>` in the byte slab.
#[inline]
pub(crate) unsafe fn is_queued(ptr: *mut u8) -> bool {
    // SAFETY: is_queued is at offset 16 in repr(C) Task.
    // Caller guarantees ptr is valid.
    unsafe { *ptr.add(16) != 0 }
}

/// Set the `is_queued` flag on a task.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>` in the byte slab.
#[inline]
pub(crate) unsafe fn set_queued(ptr: *mut u8, queued: bool) {
    // SAFETY: is_queued is at offset 16 in repr(C) Task.
    unsafe { *ptr.add(16) = queued as u8 }
}

/// Poll the task's future.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>` in the byte slab.
/// The future must not have been dropped.
#[inline]
pub(crate) unsafe fn poll_task(ptr: *mut u8, cx: &mut Context<'_>) -> Poll<()> {
    // SAFETY: poll_fn is at offset 0 in repr(C) Task.
    let poll_fn: unsafe fn(*mut u8, &mut Context<'_>) -> Poll<()> =
        unsafe { *(ptr as *const unsafe fn(*mut u8, &mut Context<'_>) -> Poll<()>) };
    // Future bytes start at TASK_HEADER_SIZE offset.
    let future_ptr = unsafe { ptr.add(TASK_HEADER_SIZE) };
    unsafe { poll_fn(future_ptr, cx) }
}

/// Drop the task's future in place.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`. Must only be called once.
#[inline]
pub(crate) unsafe fn drop_task_future(ptr: *mut u8) {
    // SAFETY: drop_fn is at offset 8 in repr(C) Task.
    let drop_fn: unsafe fn(*mut u8) =
        unsafe { *(ptr.add(8) as *const unsafe fn(*mut u8)) };
    let future_ptr = unsafe { ptr.add(TASK_HEADER_SIZE) };
    unsafe { drop_fn(future_ptr) }
}

// =============================================================================
// Type-erased vtable functions
// =============================================================================

/// Type-erased poll: cast back to `Pin<&mut F>` and poll.
///
/// # Safety
///
/// `ptr` must point to a live `F` at the future offset within a Task.
/// Address is stable (slab guarantee) so Pin is sound.
unsafe fn poll_fn<F: Future<Output = ()>>(
    ptr: *mut u8,
    cx: &mut Context<'_>,
) -> Poll<()> {
    // SAFETY: ptr points to a live F. Address is stable (slab).
    let future = unsafe { Pin::new_unchecked(&mut *ptr.cast::<F>()) };
    future.poll(cx)
}

/// Type-erased drop: cast back to `*mut F` and drop in place.
///
/// # Safety
///
/// `ptr` must point to a live `F`. Must only be called once.
unsafe fn drop_fn<F: Future<Output = ()>>(ptr: *mut u8) {
    // SAFETY: ptr points to a live F. Caller guarantees single drop.
    unsafe { std::ptr::drop_in_place(ptr.cast::<F>()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_header_size() {
        assert_eq!(TASK_HEADER_SIZE, 24);
        assert_eq!(std::mem::size_of::<Task<()>>(), 24);
    }

    #[test]
    fn task_layout_offsets() {
        // Verify repr(C) layout matches our offset assumptions.
        assert_eq!(std::mem::offset_of!(Task<()>, poll_fn), 0);
        assert_eq!(std::mem::offset_of!(Task<()>, drop_fn), 8);
        assert_eq!(std::mem::offset_of!(Task<()>, is_queued), 16);
        assert_eq!(std::mem::offset_of!(Task<()>, future), 24);
    }

    #[test]
    fn task_size_with_future() {
        #[allow(dead_code)]
        struct SmallFuture([u8; 64]);
        impl Future for SmallFuture {
            type Output = ();
            fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                Poll::Ready(())
            }
        }

        assert_eq!(
            std::mem::size_of::<Task<SmallFuture>>(),
            TASK_HEADER_SIZE + 64
        );
    }

    #[test]
    fn queued_flag_via_pointer() {
        struct Noop;
        impl Future for Noop {
            type Output = ();
            fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                Poll::Ready(())
            }
        }

        let task = Box::new(Task::new(Noop));
        let ptr = Box::into_raw(task) as *mut u8;

        unsafe {
            assert!(!is_queued(ptr));
            set_queued(ptr, true);
            assert!(is_queued(ptr));
            set_queued(ptr, false);
            assert!(!is_queued(ptr));

            // Clean up
            drop(Box::from_raw(ptr as *mut Task<Noop>));
        }
    }
}
