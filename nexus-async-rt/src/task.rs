//! Task storage: header + future in a contiguous allocation.
//!
//! Each task is a `Task<F>` struct. The raw pointer to the allocation
//! IS the task handle — no index layer, no separate metadata store.
//!
//! The waker holds the raw pointer directly. `wake()` sets `is_queued`
//! and pushes the pointer to the ready queue. Zero allocations.
//!
//! Tasks can be allocated via Box (default) or slab (power user).
//! The `free_fn` in the header knows how to deallocate regardless
//! of which allocator was used.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicPtr;
use std::task::{Context, Poll};

// =============================================================================
// Task layout
// =============================================================================

/// Header size in bytes. Must match the layout of `Task<F>` before the
/// `future` field.
pub const TASK_HEADER_SIZE: usize = 40;

/// Task header + future in a contiguous allocation. `repr(C)` for
/// deterministic layout.
///
/// The raw pointer to this struct is the task handle, the waker data,
/// and the ready queue entry — all the same pointer.
///
/// Layout (64-bit):
/// ```text
/// offset  0: poll_fn      (8 bytes, fn pointer — polls the future)
/// offset  8: drop_fn      (8 bytes, fn pointer — drops the future in place)
/// offset 16: free_fn      (8 bytes, fn pointer — deallocates the task storage)
/// offset 24: is_queued    (1 byte, bool)
/// offset 25: is_completed (1 byte, bool — future dropped, awaiting refcount drain)
/// offset 26: ref_count    (2 bytes, u16 — number of live Waker clones)
/// offset 28: tracker_key  (4 bytes, u32 — index in Executor::all_tasks slab)
/// offset 32: cross_next   (8 bytes, AtomicPtr — intrusive cross-thread wake queue)
/// offset 40: future       (F bytes, the actual future)
/// ```
#[repr(C)]
pub(crate) struct Task<F> {
    poll_fn: unsafe fn(*mut u8, &mut Context<'_>) -> Poll<()>,
    drop_fn: unsafe fn(*mut u8),
    free_fn: unsafe fn(*mut u8),
    is_queued: bool,
    /// Set when the future is dropped (completion/cancel). The slot
    /// stays alive until ref_count also hits 0.
    is_completed: bool,
    /// Number of live Waker clones. Incremented on waker clone,
    /// decremented on waker wake (by value) or drop. When this
    /// reaches 0 and is_completed is true, the slot is freed.
    ref_count: u16,
    /// Index into the Executor's `all_tasks` slab. Set at spawn time.
    tracker_key: u32,
    /// Intrusive next pointer for the cross-thread wake queue.
    /// Null when the task is not in the cross-thread inbox.
    cross_next: AtomicPtr<u8>,
    future: F,
}

// Static assertion: header layout matches TASK_HEADER_SIZE.
const _: () = {
    assert!(std::mem::size_of::<Task<()>>() == TASK_HEADER_SIZE);
};

impl<F: Future<Output = ()> + 'static> Task<F> {
    /// Construct a task with a Box-based free function.
    #[inline]
    pub(crate) fn new_boxed(future: F, tracker_key: u32) -> Self {
        Self {
            poll_fn: poll_fn::<F>,
            drop_fn: drop_fn::<F>,
            free_fn: box_free::<F>,
            is_queued: false,
            is_completed: false,
            ref_count: 1, // executor holds one reference
            tracker_key,
            cross_next: AtomicPtr::new(std::ptr::null_mut()),
            future,
        }
    }

    /// Construct a task with a custom free function (for slab allocation).
    #[inline]
    pub(crate) fn new_with_free(
        future: F,
        tracker_key: u32,
        free_fn: unsafe fn(*mut u8),
    ) -> Self {
        Self {
            poll_fn: poll_fn::<F>,
            drop_fn: drop_fn::<F>,
            free_fn,
            is_queued: false,
            is_completed: false,
            ref_count: 1,
            tracker_key,
            cross_next: AtomicPtr::new(std::ptr::null_mut()),
            future,
        }
    }
}

// =============================================================================
// Task handle — raw pointer operations
// =============================================================================

/// Opaque task identifier. Wraps the raw pointer to the task.
/// The pointer is stable for the task's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub(crate) *mut u8);

impl TaskId {
    /// Returns the raw pointer to the task. Internal use only.
    #[allow(dead_code)]
    pub(crate) fn as_ptr(&self) -> *mut u8 {
        self.0
    }
}

/// Read the `tracker_key` from a task pointer.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
pub(crate) unsafe fn tracker_key(ptr: *mut u8) -> u32 {
    // SAFETY: tracker_key is at offset 28 in repr(C) Task.
    unsafe { *(ptr.add(28).cast::<u32>()) }
}

/// Increment the waker refcount. Called on waker clone.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
pub(crate) unsafe fn ref_inc(ptr: *mut u8) {
    // SAFETY: ref_count is at offset 26 in repr(C) Task.
    let rc = unsafe { &mut *ptr.add(26).cast::<u16>() };
    *rc = rc.checked_add(1).expect("waker refcount overflow");
}

/// Decrement the refcount. Returns true if refcount hit 0 (slot can be freed).
///
/// # Safety
///
/// `ptr` must point to a live (or completed) `Task<F>`.
#[inline]
pub(crate) unsafe fn ref_dec(ptr: *mut u8) -> bool {
    // SAFETY: ref_count at offset 26.
    let rc = unsafe { &mut *ptr.add(26).cast::<u16>() };
    debug_assert!(*rc > 0, "waker refcount underflow");
    *rc -= 1;
    *rc == 0
}

/// Read the refcount.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[allow(dead_code)]
#[inline]
pub(crate) unsafe fn ref_count(ptr: *mut u8) -> u16 {
    unsafe { *ptr.add(26).cast::<u16>() }
}

/// Set the is_completed flag.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
pub(crate) unsafe fn set_completed(ptr: *mut u8) {
    // SAFETY: is_completed is at offset 25 in repr(C) Task.
    unsafe { *ptr.add(25) = 1 }
}

/// Read the is_completed flag.
///
/// # Safety
///
/// `ptr` must point to a (possibly completed) `Task<F>`.
#[inline]
pub(crate) unsafe fn is_completed(ptr: *mut u8) -> bool {
    unsafe { *ptr.add(25) != 0 }
}

/// Get a reference to the `cross_next` atomic pointer.
///
/// Used by the intrusive cross-thread wake queue. The pointer lives
/// at offset 32 in the task header.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
#[allow(dead_code)] // Used by cross_wake module (coming next)
pub(crate) unsafe fn cross_next(ptr: *mut u8) -> &'static AtomicPtr<u8> {
    // SAFETY: cross_next is at offset 32 in repr(C) Task.
    unsafe { &*ptr.add(32).cast::<AtomicPtr<u8>>() }
}

/// Read the `is_queued` flag from a task pointer.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
pub(crate) unsafe fn is_queued(ptr: *mut u8) -> bool {
    // SAFETY: is_queued is at offset 24 in repr(C) Task.
    unsafe { *ptr.add(24) != 0 }
}

/// Set the `is_queued` flag on a task.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
pub(crate) unsafe fn set_queued(ptr: *mut u8, queued: bool) {
    unsafe { *ptr.add(24) = queued as u8 }
}

/// Poll the task's future.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
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

/// Call the task's free function to deallocate its storage.
///
/// # Safety
///
/// `ptr` must point to a `Task<F>` whose future has already been dropped.
/// Must only be called once (after refcount reaches 0).
#[inline]
pub(crate) unsafe fn free_task(ptr: *mut u8) {
    // SAFETY: free_fn is at offset 16 in repr(C) Task.
    let free_fn: unsafe fn(*mut u8) =
        unsafe { *(ptr.add(16) as *const unsafe fn(*mut u8)) };
    unsafe { free_fn(ptr) }
}

// =============================================================================
// Type-erased vtable functions
// =============================================================================

/// Type-erased poll: cast back to `Pin<&mut F>` and poll.
///
/// # Safety
///
/// `ptr` must point to a live `F` at the future offset within a Task.
/// Address is stable (Box or slab guarantee) so Pin is sound.
unsafe fn poll_fn<F: Future<Output = ()>>(
    ptr: *mut u8,
    cx: &mut Context<'_>,
) -> Poll<()> {
    let future = unsafe { Pin::new_unchecked(&mut *ptr.cast::<F>()) };
    future.poll(cx)
}

/// Type-erased drop: cast back to `*mut F` and drop in place.
///
/// # Safety
///
/// `ptr` must point to a live `F`. Must only be called once.
unsafe fn drop_fn<F: Future<Output = ()>>(ptr: *mut u8) {
    unsafe { std::ptr::drop_in_place(ptr.cast::<F>()) }
}

/// Free function for Box-allocated tasks.
///
/// Deallocates the memory without running destructors — the future
/// was already dropped via `drop_task_future`, and the header fields
/// are all Copy. Only the heap allocation needs to be freed.
///
/// # Safety
///
/// `ptr` must have been produced by `Box::into_raw(Box::new(Task<F>))`.
/// The future at `ptr + TASK_HEADER_SIZE` must already be dropped.
unsafe fn box_free<F>(ptr: *mut u8) {
    // SAFETY: Layout matches what Box::new(Task<F>) allocated.
    let layout = std::alloc::Layout::new::<Task<F>>();
    unsafe { std::alloc::dealloc(ptr, layout) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_header_size() {
        assert_eq!(TASK_HEADER_SIZE, 40);
        assert_eq!(std::mem::size_of::<Task<()>>(), 40);
    }

    #[test]
    fn task_layout_offsets() {
        assert_eq!(std::mem::offset_of!(Task<()>, poll_fn), 0);
        assert_eq!(std::mem::offset_of!(Task<()>, drop_fn), 8);
        assert_eq!(std::mem::offset_of!(Task<()>, free_fn), 16);
        assert_eq!(std::mem::offset_of!(Task<()>, is_queued), 24);
        assert_eq!(std::mem::offset_of!(Task<()>, is_completed), 25);
        assert_eq!(std::mem::offset_of!(Task<()>, ref_count), 26);
        assert_eq!(std::mem::offset_of!(Task<()>, tracker_key), 28);
        assert_eq!(std::mem::offset_of!(Task<()>, cross_next), 32);
        assert_eq!(std::mem::offset_of!(Task<()>, future), 40);
    }

    #[test]
    fn task_size_with_future() {
        #[allow(dead_code)]
        struct SmallFuture([u8; 24]);
        impl Future for SmallFuture {
            type Output = ();
            fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                Poll::Ready(())
            }
        }

        // 40 byte header + 24 byte future = 64 bytes (fits in min slab slot)
        assert_eq!(
            std::mem::size_of::<Task<SmallFuture>>(),
            TASK_HEADER_SIZE + 24
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

        let task = Box::new(Task::new_boxed(Noop, 0));
        let ptr = Box::into_raw(task) as *mut u8;

        unsafe {
            assert!(!is_queued(ptr));
            set_queued(ptr, true);
            assert!(is_queued(ptr));
            set_queued(ptr, false);
            assert!(!is_queued(ptr));

            // Drop future, then free storage (matches executor lifecycle).
            drop_task_future(ptr);
            free_task(ptr);
        }
    }

    #[test]
    fn box_free_works() {
        struct Noop;
        impl Future for Noop {
            type Output = ();
            fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                Poll::Ready(())
            }
        }

        let task = Box::new(Task::new_boxed(Noop, 42));
        let ptr = Box::into_raw(task) as *mut u8;

        unsafe {
            assert_eq!(tracker_key(ptr), 42);
            assert_eq!(ref_count(ptr), 1);
            // Drop future, then free storage.
            drop_task_future(ptr);
            free_task(ptr);
        }
    }
}
