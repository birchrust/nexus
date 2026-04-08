//! Task storage: header + future/output union in a contiguous allocation.
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
//!
//! ## Union storage
//!
//! The slot at `storage_offset` holds either `F` (the future) or `T` (the output),
//! never both. While running, `F` is live. When the future completes,
//! `poll_join` drops `F` in place and writes `T` to the same bytes.
//! `drop_fn` is overwritten from `drop_fn::<F>` to `drop_output::<T>`
//! so subsequent cleanup targets the correct type.

use std::cell::UnsafeCell;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU16, Ordering};
use std::task::{Context, Poll, Waker};

// =============================================================================
// Task flags
// =============================================================================

/// JoinHandle exists for this task.
const HAS_JOIN: u8 = 0b001;
/// JoinHandle consumed the output via poll.
const OUTPUT_TAKEN: u8 = 0b010;
/// abort() was called.
const ABORTED: u8 = 0b100;

// =============================================================================
// Task layout
// =============================================================================

/// Header size in bytes. Must match the layout of `Task<F>` before the
/// `future` field.
pub const TASK_HEADER_SIZE: usize = 64;

/// Task header + storage in a contiguous allocation. `repr(C)` for
/// deterministic layout.
///
/// `S` is the storage type — either just `F` (fire-and-forget) or a union
/// of `F` and `T` (joinable). The header is always 64 bytes regardless of `S`.
///
/// Layout (64-bit):
/// ```text
/// offset  0: poll_fn       (8B, fn pointer — polls the future)
/// offset  8: drop_fn       (8B, fn pointer — drops F or T in place)
/// offset 16: free_fn       (8B, fn pointer — deallocates the task storage)
/// offset 24: is_queued      (1B, AtomicBool — cross-thread wakers CAS this)
/// offset 25: is_completed   (1B, AtomicBool — cross-thread reads with Acquire)
/// offset 26: ref_count      (2B, AtomicU16 — number of live references)
/// offset 28: tracker_key    (4B, u32 — index in Executor::all_tasks slab)
/// offset 32: cross_next     (8B, AtomicPtr — intrusive cross-thread wake queue)
/// offset 40: join_waker     (16B, UnsafeCell<Option<Waker>>)
/// offset 56: storage_offset (2B, u16 — byte offset to storage field)
/// offset 58: flags          (1B, Cell<u8> — HAS_JOIN | OUTPUT_TAKEN | ABORTED)
/// offset 59: _pad           (5B)
/// offset 64: storage        (S bytes — future F or union { F, T })
/// ```
#[repr(C)]
pub(crate) struct Task<S> {
    /// Polls the future. Receives the task base pointer.
    poll_fn: unsafe fn(*mut u8, &mut Context<'_>) -> Poll<()>,
    /// Drops the value at `storage_offset` (future F or output T). Receives base pointer.
    drop_fn: unsafe fn(*mut u8),
    /// Deallocates the task storage.
    free_fn: unsafe fn(*mut u8),
    is_queued: AtomicBool,
    /// Set when the future completes or is aborted.
    is_completed: AtomicBool,
    /// Number of live references (executor + waker clones + JoinHandle).
    ref_count: AtomicU16,
    /// Index into the Executor's `all_tasks` slab.
    tracker_key: u32,
    /// Intrusive next pointer for the cross-thread wake queue.
    cross_next: AtomicPtr<u8>,
    /// Waker for the task awaiting this JoinHandle.
    join_waker: UnsafeCell<Option<Waker>>,
    /// Byte offset from task base to the storage field.
    /// Set at construction from `offset_of!(Task<S>, storage)`.
    storage_offset: u16,
    /// Packed flags: HAS_JOIN | OUTPUT_TAKEN | ABORTED.
    /// Single-threaded — no atomics needed.
    flags: std::cell::Cell<u8>,
    /// Padding to reach 64 bytes.
    _pad: [u8; 5],
    storage: S,
}

/// Union storage for joinable tasks. Sized to fit both the future F
/// and the output T in the same allocation.
#[repr(C)]
pub(crate) union FutureOrOutput<F, T> {
    pub(crate) future: std::mem::ManuallyDrop<F>,
    pub(crate) output: std::mem::ManuallyDrop<T>,
}

// Static assertion: header layout matches TASK_HEADER_SIZE.
const _: () = {
    assert!(std::mem::size_of::<Task<()>>() == TASK_HEADER_SIZE);
};

impl<F: Future<Output = ()> + 'static> Task<F> {
    /// Construct a fire-and-forget task (no JoinHandle) with Box-based free.
    ///
    /// Used internally for tests and low-level task construction.
    /// `ref_count = 1` (executor only), `HAS_JOIN` not set.
    ///
    /// # Why `Output = ()` is required
    ///
    /// This uses `poll_join::<F>` which writes T at offset 64 after dropping F.
    /// The storage is `F` (not `FutureOrOutput<F, T>`), so it's only sized for F.
    /// With `T = ()` (ZST), the write is zero-size and the `drop_fn` overwrite
    /// to `drop_output::<()>` is a no-op. Relaxing this bound to non-ZST T
    /// would write T into storage not sized for it — UB.
    #[cfg(test)]
    #[inline]
    pub(crate) fn new_boxed(future: F, tracker_key: u32) -> Self {
        Self {
            poll_fn: poll_join::<F>,
            drop_fn: drop_future::<F>,
            free_fn: box_free::<F>,
            is_queued: AtomicBool::new(false),
            is_completed: AtomicBool::new(false),
            ref_count: AtomicU16::new(1),
            tracker_key,
            cross_next: AtomicPtr::new(std::ptr::null_mut()),
            join_waker: UnsafeCell::new(None),
            flags: std::cell::Cell::new(0),
            storage_offset: std::mem::offset_of!(Task<F>, storage) as u16,
            _pad: [0; 5],
            storage: future,
        }
    }
}

/// Allocate a joinable Box task and return the raw pointer.
///
/// The task has `ref_count = 2` (executor + JoinHandle) and `HAS_JOIN` set.
/// The allocation is sized for `max(size_of::<F>(), size_of::<T>())` via
/// the `FutureOrOutput<F, T>` union.
pub(crate) fn box_spawn_joinable<F>(future: F, tracker_key: u32) -> *mut u8
where
    F: Future + 'static,
    F::Output: 'static,
{
    type Storage<F> = FutureOrOutput<F, <F as Future>::Output>;

    let task: Task<Storage<F>> = Task {
        poll_fn: poll_join::<F>,
        drop_fn: drop_future_in_union::<F>,
        free_fn: box_free::<Storage<F>>,
        is_queued: AtomicBool::new(false),
        is_completed: AtomicBool::new(false),
        ref_count: AtomicU16::new(2), // executor + JoinHandle
        tracker_key,
        cross_next: AtomicPtr::new(std::ptr::null_mut()),
        join_waker: UnsafeCell::new(None),
        flags: std::cell::Cell::new(HAS_JOIN),
        storage_offset: std::mem::offset_of!(Task<Storage<F>>, storage) as u16,
        _pad: [0; 5],
        storage: FutureOrOutput {
            future: std::mem::ManuallyDrop::new(future),
        },
    };
    Box::into_raw(Box::new(task)) as *mut u8
}

/// Construct a joinable task for slab allocation.
///
/// Returns the task struct to be copied into a slab slot. Uses the
/// `FutureOrOutput<F, T>` union so the allocation fits both.
pub(crate) fn new_joinable_slab<F>(
    future: F,
    tracker_key: u32,
    free_fn: unsafe fn(*mut u8),
) -> Task<FutureOrOutput<F, F::Output>>
where
    F: Future + 'static,
    F::Output: 'static,
{
    type Storage<F> = FutureOrOutput<F, <F as Future>::Output>;

    Task {
        poll_fn: poll_join::<F>,
        drop_fn: drop_future_in_union::<F>,
        free_fn,
        is_queued: AtomicBool::new(false),
        is_completed: AtomicBool::new(false),
        ref_count: AtomicU16::new(2), // executor + JoinHandle
        tracker_key,
        cross_next: AtomicPtr::new(std::ptr::null_mut()),
        join_waker: UnsafeCell::new(None),
        flags: std::cell::Cell::new(HAS_JOIN),
        storage_offset: std::mem::offset_of!(Task<Storage<F>>, storage) as u16,
        _pad: [0; 5],
        storage: FutureOrOutput {
            future: std::mem::ManuallyDrop::new(future),
        },
    }
}

// =============================================================================
// Task handle — raw pointer operations
// =============================================================================

/// Opaque task identifier. Wraps the raw pointer to the task.
/// The pointer is stable for the task's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TaskId(pub(crate) *mut u8);

impl TaskId {
    /// Returns the raw pointer to the task.
    #[allow(dead_code)]
    pub(crate) fn as_ptr(&self) -> *mut u8 {
        self.0
    }
}

// =============================================================================
// JoinHandle
// =============================================================================

/// Handle to a spawned task. Await to get the result.
///
/// Dropping the handle detaches the task — it continues running but the
/// output is dropped when the task completes. Use [`abort()`](Self::abort)
/// to cancel the task.
///
/// `JoinHandle` is `!Send` and `!Sync` — it must stay on the executor thread.
#[must_use = "dropping a JoinHandle detaches the task — await it or call .abort()"]
pub struct JoinHandle<T> {
    ptr: *mut u8,
    _marker: PhantomData<T>,
    _not_send: PhantomData<*const ()>, // !Send + !Sync
}

impl<T: 'static> Future for JoinHandle<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let ptr = self.ptr;

        // SAFETY: ptr is valid — JoinHandle holds a ref (refcount >= 1).
        if unsafe { is_completed(ptr) } {
            let flags = unsafe { task_flags(ptr) };
            assert!(
                flags & ABORTED == 0,
                "polled JoinHandle after task was aborted"
            );
            // SAFETY: Task completed, so poll_join already transitioned the union
            // from F to T. The output is live at storage_offset. ptr::read moves
            // it out (bitwise copy). OUTPUT_TAKEN prevents double-read.
            let output_ptr = unsafe { ptr.add(storage_offset(ptr)) };
            let value = unsafe { std::ptr::read(output_ptr.cast::<T>()) };
            unsafe { set_flag(ptr, OUTPUT_TAKEN) };
            Poll::Ready(value)
        } else {
            // SAFETY: Task still running, single-threaded — safe to write waker.
            unsafe { set_join_waker(ptr, cx.waker().clone()) };
            Poll::Pending
        }
    }
}

impl<T> JoinHandle<T> {
    pub(crate) fn new(ptr: *mut u8) -> Self {
        Self {
            ptr,
            _marker: PhantomData,
            _not_send: PhantomData,
        }
    }

    /// Returns `true` if the task has completed (output is ready).
    pub fn is_finished(&self) -> bool {
        unsafe { is_completed(self.ptr) }
    }

    /// Abort the task and consume the handle.
    ///
    /// The future is dropped on the next poll cycle. Consumes the handle
    /// so it cannot be awaited after abort — this is enforced at the type
    /// level rather than via a runtime panic.
    ///
    /// Returns `true` if the task was still running, `false` if it had
    /// already completed (output is dropped by `JoinHandle::drop`).
    #[must_use = "returns whether the task was still running"]
    pub fn abort(self) -> bool {
        let ptr = self.ptr;
        let was_running = !unsafe { is_completed(ptr) };
        if was_running {
            unsafe { set_flag(ptr, ABORTED) };
        }
        // self is consumed — Drop runs, which clears HAS_JOIN,
        // takes the join waker, and decrements refcount.
        was_running
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        let ptr = self.ptr;
        // SAFETY: ptr is valid — JoinHandle holds a ref (refcount >= 1).
        // All accessor calls below are single-threaded and target valid
        // header fields at known offsets.
        let flags = unsafe { task_flags(ptr) };

        if unsafe { is_completed(ptr) } && (flags & OUTPUT_TAKEN == 0) && (flags & ABORTED == 0) {
            // Task completed but output was never read — drop it.
            // SAFETY: poll_join overwrote drop_fn to drop_output::<T>,
            // so this drops the output T (not the future F).
            unsafe { drop_task_future(ptr) };
        }

        // Clear HAS_JOIN so complete_task knows nobody is waiting.
        unsafe { clear_flag(ptr, HAS_JOIN) };

        // If we previously polled to Pending, a cloned waker is stored in the
        // task. Clear it so the parent task's refcount isn't kept alive until
        // the child completes. take() returns None if no waker was stored.
        let _ = unsafe { take_join_waker(ptr) };

        // Release our reference. If refcount hits 0, the task is complete and
        // all other refs (executor, wakers) are gone — defer the free.
        let should_free = unsafe { ref_dec(ptr) };
        if should_free {
            // SAFETY: refcount is 0, task is completed. Can't free directly
            // because we may be outside the poll cycle. defer_free pushes to
            // TLS deferred list (or leaks if TLS unavailable — Executor::drop
            // catches those).
            unsafe { defer_free_slot(ptr) };
        }
    }
}

/// Push a task to the deferred free list, or free immediately if outside poll.
///
/// # Safety
///
/// `ptr` must point to a completed task with ref_count 0.
unsafe fn defer_free_slot(ptr: *mut u8) {
    unsafe { crate::waker::defer_free(ptr) };
}

// =============================================================================
// Task header accessor functions
// =============================================================================

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
    // SAFETY: ref_count is AtomicU16 at offset 26 in repr(C) Task.
    let rc = unsafe { &*ptr.add(26).cast::<AtomicU16>() };
    let prev = rc.fetch_add(1, Ordering::Relaxed);
    assert!(prev < u16::MAX, "waker refcount overflow");
}

/// Decrement the refcount. Returns true if refcount hit 0 (slot can be freed).
///
/// # Safety
///
/// `ptr` must point to a live (or completed) `Task<F>`.
#[inline]
pub(crate) unsafe fn ref_dec(ptr: *mut u8) -> bool {
    // SAFETY: ref_count is AtomicU16 at offset 26.
    let rc = unsafe { &*ptr.add(26).cast::<AtomicU16>() };
    let prev = rc.fetch_sub(1, Ordering::AcqRel);
    debug_assert!(prev > 0, "waker refcount underflow");
    prev == 1
}

/// Read the refcount.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[allow(dead_code)]
#[inline]
pub(crate) unsafe fn ref_count(ptr: *mut u8) -> u16 {
    // SAFETY: ref_count is AtomicU16 at offset 26.
    unsafe { &*ptr.add(26).cast::<AtomicU16>() }.load(Ordering::Relaxed)
}

/// Set the is_completed flag.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
pub(crate) unsafe fn set_completed(ptr: *mut u8) {
    // SAFETY: is_completed is AtomicBool at offset 25 in repr(C) Task.
    unsafe { &*ptr.add(25).cast::<AtomicBool>() }.store(true, Ordering::Release);
}

/// Read the is_completed flag.
///
/// # Safety
///
/// `ptr` must point to a (possibly completed) `Task<F>`.
#[inline]
pub(crate) unsafe fn is_completed(ptr: *mut u8) -> bool {
    // SAFETY: is_completed is AtomicBool at offset 25.
    unsafe { &*ptr.add(25).cast::<AtomicBool>() }.load(Ordering::Acquire)
}

/// Get a reference to the `cross_next` atomic pointer.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
#[allow(dead_code)]
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
    // SAFETY: is_queued is AtomicBool at offset 24 in repr(C) Task.
    unsafe { &*ptr.add(24).cast::<AtomicBool>() }.load(Ordering::Relaxed)
}

/// Set the `is_queued` flag on a task.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
pub(crate) unsafe fn set_queued(ptr: *mut u8, queued: bool) {
    // SAFETY: is_queued is AtomicBool at offset 24 in repr(C) Task.
    unsafe { &*ptr.add(24).cast::<AtomicBool>() }.store(queued, Ordering::Relaxed);
}

/// Atomically try to set `is_queued` from false to true. Returns true if
/// successful (was not queued). Used by cross-thread wakers.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
pub(crate) unsafe fn try_set_queued(ptr: *mut u8) -> bool {
    // SAFETY: is_queued is AtomicBool at offset 24.
    let queued = unsafe { &*ptr.add(24).cast::<AtomicBool>() };
    queued
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
}

/// Read the storage offset from the task header.
///
/// # Safety
///
/// `ptr` must point to a live `Task<S>`.
#[inline]
pub(crate) unsafe fn storage_offset(ptr: *mut u8) -> usize {
    // SAFETY: storage_offset is u16 at offset 56 in repr(C) Task.
    unsafe { *(ptr.add(56).cast::<u16>()) as usize }
}

/// Read task_flags.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`. Single-threaded access only.
#[inline]
unsafe fn task_flags(ptr: *mut u8) -> u8 {
    // SAFETY: flags is Cell<u8> at offset 58.
    unsafe { &*ptr.add(58).cast::<std::cell::Cell<u8>>() }.get()
}

/// Set a flag bit in task_flags.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`. Single-threaded access only.
#[inline]
unsafe fn set_flag(ptr: *mut u8, flag: u8) {
    let cell = unsafe { &*ptr.add(58).cast::<std::cell::Cell<u8>>() };
    cell.set(cell.get() | flag);
}

/// Clear a flag bit in task_flags.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`. Single-threaded access only.
#[inline]
unsafe fn clear_flag(ptr: *mut u8, flag: u8) {
    let cell = unsafe { &*ptr.add(58).cast::<std::cell::Cell<u8>>() };
    cell.set(cell.get() & !flag);
}

/// Check if HAS_JOIN flag is set.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
pub(crate) unsafe fn has_join(ptr: *mut u8) -> bool {
    (unsafe { task_flags(ptr) }) & HAS_JOIN != 0
}

/// Check if ABORTED flag is set.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`.
#[inline]
pub(crate) unsafe fn is_aborted(ptr: *mut u8) -> bool {
    (unsafe { task_flags(ptr) }) & ABORTED != 0
}

/// Store a waker for the JoinHandle awaiter.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`. Single-threaded access only.
#[inline]
unsafe fn set_join_waker(ptr: *mut u8, waker: Waker) {
    // SAFETY: join_waker is UnsafeCell<Option<Waker>> at offset 40.
    let cell = unsafe { &*ptr.add(40).cast::<UnsafeCell<Option<Waker>>>() };
    unsafe { *cell.get() = Some(waker) };
}

/// Take the join waker (if any).
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`. Single-threaded access only.
#[inline]
pub(crate) unsafe fn take_join_waker(ptr: *mut u8) -> Option<Waker> {
    let cell = unsafe { &*ptr.add(40).cast::<UnsafeCell<Option<Waker>>>() };
    unsafe { (*cell.get()).take() }
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
    // Pass the task base pointer — the trampoline reads storage_offset.
    unsafe { poll_fn(ptr, cx) }
}

/// Drop the task's future (or output) in place.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`. Must only be called once.
#[inline]
pub(crate) unsafe fn drop_task_future(ptr: *mut u8) {
    // SAFETY: drop_fn is at offset 8 in repr(C) Task.
    let drop_fn: unsafe fn(*mut u8) = unsafe { *(ptr.add(8) as *const unsafe fn(*mut u8)) };
    // Pass base pointer — the trampoline reads storage_offset.
    unsafe { drop_fn(ptr) }
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
    let free_fn: unsafe fn(*mut u8) = unsafe { *(ptr.add(16) as *const unsafe fn(*mut u8)) };
    unsafe { free_fn(ptr) }
}

// =============================================================================
// Type-erased vtable functions
// =============================================================================

/// Poll trampoline for joinable tasks (Output = T).
///
/// On completion: drops F, writes T into the same location, overwrites
/// drop_fn to target T instead of F.
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>`. The future must not have been dropped.
unsafe fn poll_join<F: Future>(ptr: *mut u8, cx: &mut Context<'_>) -> Poll<()>
where
    F::Output: 'static,
{
    // Check if aborted
    if unsafe { is_aborted(ptr) } {
        return Poll::Ready(());
    }

    let future_ptr = unsafe { ptr.add(storage_offset(ptr)) };
    let future = unsafe { Pin::new_unchecked(&mut *future_ptr.cast::<F>()) };
    match future.poll(cx) {
        Poll::Pending => Poll::Pending,
        Poll::Ready(value) => {
            let drop_fn_slot = unsafe { ptr.add(8).cast::<unsafe fn(*mut u8)>() };
            // 1. Overwrite drop_fn to no-op BEFORE dropping F.
            //    If F::drop() panics, this prevents double-drop —
            //    subsequent cleanup calls the no-op instead of
            //    drop_future_in_union on a partially-dropped F.
            //    The output (value) is dropped during unwind (stack-owned).
            unsafe { *drop_fn_slot = drop_noop };
            // 2. Drop the future in place (panic-safe now)
            unsafe { std::ptr::drop_in_place(future_ptr.cast::<F>()) };
            // 3. Write output T into the same location
            unsafe { std::ptr::write(future_ptr.cast::<F::Output>(), value) };
            // 4. Overwrite drop_fn: now drops T instead of F
            unsafe { *drop_fn_slot = drop_output::<F::Output> };
            Poll::Ready(())
        }
    }
}

/// Drop trampoline for futures stored directly (fire-and-forget tasks).
///
/// # Safety
///
/// `ptr` must point to a live `Task<F>` with a live future at `storage_offset`.
#[cfg(test)]
unsafe fn drop_future<F>(ptr: *mut u8) {
    let future_ptr = unsafe { ptr.add(storage_offset(ptr)) };
    unsafe { std::ptr::drop_in_place(future_ptr.cast::<F>()) }
}

/// Drop trampoline for futures stored in FutureOrOutput union.
///
/// # Safety
///
/// `ptr` must point to a `Task<FutureOrOutput<F, T>>` with a live future.
unsafe fn drop_future_in_union<F: Future>(ptr: *mut u8) {
    let storage_ptr = unsafe { ptr.add(storage_offset(ptr)) };
    // The future is at the start of the union (same offset as the union itself).
    unsafe { std::ptr::drop_in_place(storage_ptr.cast::<F>()) }
}

/// No-op drop trampoline. Installed temporarily during the F→T transition
/// in `poll_join` to prevent double-drop if `F::drop()` panics.
///
/// # Safety
///
/// Always safe — does nothing.
unsafe fn drop_noop(_ptr: *mut u8) {}

/// Drop trampoline for output values. Receives the task base pointer.
///
/// Installed by `poll_join` after the future completes, replacing `drop_future`.
///
/// # Safety
///
/// `ptr` must point to a `Task` with a live `T` at `storage_offset`.
unsafe fn drop_output<T>(ptr: *mut u8) {
    let output_ptr = unsafe { ptr.add(storage_offset(ptr)) };
    unsafe { std::ptr::drop_in_place(output_ptr.cast::<T>()) }
}

/// Free function for Box-allocated tasks.
///
/// Deallocates the memory without running destructors — the future/output
/// was already dropped via `drop_task_future`, and the header fields
/// are all trivial. Only the heap allocation needs to be freed.
///
/// # Safety
///
/// `ptr` must have been produced by `Box::into_raw(Box::new(Task<F>))`.
/// The value at offset 64 must already be dropped.
unsafe fn box_free<F>(ptr: *mut u8) {
    // SAFETY: Layout matches what Box::new(Task<F>) allocated.
    let layout = std::alloc::Layout::new::<Task<F>>();
    unsafe { std::alloc::dealloc(ptr, layout) }
}

// Remove the dead new_joinable_boxed function that had a bad API.
// box_spawn_joinable and new_joinable_slab are the correct APIs.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_header_size() {
        assert_eq!(TASK_HEADER_SIZE, 64);
        assert_eq!(std::mem::size_of::<Task<()>>(), 64);
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
        assert_eq!(std::mem::offset_of!(Task<()>, join_waker), 40);
        assert_eq!(std::mem::offset_of!(Task<()>, storage_offset), 56);
        assert_eq!(std::mem::offset_of!(Task<()>, flags), 58);
        assert_eq!(std::mem::offset_of!(Task<()>, _pad), 59);
        assert_eq!(std::mem::offset_of!(Task<()>, storage), 64);
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

        // 64 byte header + 24 byte future = 88 bytes
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

    #[test]
    fn joinable_task_flags() {
        struct Noop;
        impl Future for Noop {
            type Output = u64;
            fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<u64> {
                Poll::Ready(42)
            }
        }

        let ptr = box_spawn_joinable(Noop, 0);
        unsafe {
            assert!(has_join(ptr));
            assert!(!is_aborted(ptr));
            assert_eq!(ref_count(ptr), 2); // executor + JoinHandle

            // Clean up
            drop_task_future(ptr);
            ref_dec(ptr); // JoinHandle ref
            ref_dec(ptr); // executor ref
            free_task(ptr);
        }
    }

    // =========================================================================
    // Panic safety — drop_fn transitions
    // =========================================================================

    /// Future whose Drop impl panics. Used to verify the drop_noop guard
    /// in poll_join prevents double-drop.
    struct PanickingDrop {
        drop_count: *mut u32,
    }

    impl Future for PanickingDrop {
        type Output = u64;
        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<u64> {
            Poll::Ready(42)
        }
    }

    impl Drop for PanickingDrop {
        fn drop(&mut self) {
            unsafe { *self.drop_count += 1 };
            panic!("intentional drop panic");
        }
    }

    #[test]
    fn poll_join_panic_in_drop_prevents_double_drop() {
        use std::task::{RawWaker, RawWakerVTable, Waker};

        let noop_vtable = RawWakerVTable::new(
            |p| RawWaker::new(p, &NOOP_VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        // Need a named static for the clone fn to reference.
        static NOOP_VTABLE: RawWakerVTable = RawWakerVTable::new(
            |p| RawWaker::new(p, &NOOP_VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &NOOP_VTABLE)) };
        let mut cx = Context::from_waker(&waker);

        let mut drop_count: u32 = 0;
        let ptr = box_spawn_joinable(
            PanickingDrop {
                drop_count: &raw mut drop_count,
            },
            0,
        );

        // poll_join completes the future, then drops F — which panics.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            poll_task(ptr, &mut cx)
        }));

        // The panic should have been caught.
        assert!(result.is_err(), "expected panic from PanickingDrop");
        // F was dropped exactly once (by poll_join, before the panic propagated).
        assert_eq!(drop_count, 1, "future should be dropped exactly once");

        // drop_fn should now be drop_noop — calling it must NOT double-drop F.
        unsafe { drop_task_future(ptr) };
        assert_eq!(
            drop_count, 1,
            "drop_task_future after panic must be a no-op (drop_noop)"
        );

        // Clean up: dec both refs (executor + JoinHandle), then free.
        unsafe {
            ref_dec(ptr);
            ref_dec(ptr);
            free_task(ptr);
        }
    }

    #[test]
    fn drop_fn_transitions_correctly_on_normal_completion() {
        use std::task::{RawWaker, RawWakerVTable, Waker};

        static NOOP_VTABLE: RawWakerVTable = RawWakerVTable::new(
            |p| RawWaker::new(p, &NOOP_VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &NOOP_VTABLE)) };
        let mut cx = Context::from_waker(&waker);

        static mut OUTPUT_DROP_COUNT: u32 = 0;
        struct TrackedOutput;
        impl Drop for TrackedOutput {
            fn drop(&mut self) {
                unsafe { OUTPUT_DROP_COUNT += 1 };
            }
        }

        struct ProduceTracked;
        impl Future for ProduceTracked {
            type Output = TrackedOutput;
            fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<TrackedOutput> {
                Poll::Ready(TrackedOutput)
            }
        }

        let ptr = box_spawn_joinable(ProduceTracked, 0);

        // Poll to completion — F dropped, T written, drop_fn → drop_output.
        let result = unsafe { poll_task(ptr, &mut cx) };
        assert!(result.is_ready());

        // drop_fn should now target T (TrackedOutput).
        unsafe { OUTPUT_DROP_COUNT = 0 };
        unsafe { drop_task_future(ptr) };
        assert_eq!(
            unsafe { OUTPUT_DROP_COUNT },
            1,
            "drop_fn should drop the output exactly once"
        );

        // Clean up.
        unsafe {
            ref_dec(ptr);
            ref_dec(ptr);
            free_task(ptr);
        }
    }
}
