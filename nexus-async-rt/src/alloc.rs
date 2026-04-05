//! Slab task allocator — optional, power-user feature.
//!
//! By default, tasks are Box-allocated. For zero-alloc hot-path spawning,
//! configure a slab via [`RuntimeBuilder::slab`] and use [`spawn_pinned`].
//!
//! The slab is accessed via thread-local pointers — no dynamic dispatch,
//! no Option checks. Default pointers panic with a clear message if
//! `spawn_pinned` is called without a slab configured.

use std::cell::Cell;
use std::future::Future;

use crate::task::Task;

// =============================================================================
// TLS slots
// =============================================================================

type FreeFn = unsafe fn(*const u8, *mut u8);

thread_local! {
    /// Raw pointer to the slab instance. Type-erased.
    static SLAB_PTR: Cell<*const u8> = const { Cell::new(std::ptr::null()) };

    /// Function pointer: free a task from the slab.
    /// Args: (slab_ptr, task_ptr)
    static SLAB_FREE: Cell<FreeFn> = const { Cell::new(no_slab_free) };
}

/// Panic stub — should never be reached if alloc was never called.
unsafe fn no_slab_free(_slab: *const u8, _ptr: *mut u8) {
    panic!("slab free called without a slab configured")
}

// =============================================================================
// TLS install/guard
// =============================================================================

/// Install slab pointer and free fn into TLS.
/// Returns an RAII guard that restores the previous values on drop.
pub(crate) fn install_slab(slab_ptr: *const u8, free: FreeFn) -> SlabGuard {
    let prev_ptr = SLAB_PTR.with(|c| c.replace(slab_ptr));
    let prev_free = SLAB_FREE.with(|c| c.replace(free));
    SlabGuard {
        prev_ptr,
        prev_free,
    }
}

/// RAII guard that restores previous slab TLS values on drop.
pub(crate) struct SlabGuard {
    prev_ptr: *const u8,
    prev_free: FreeFn,
}

impl Drop for SlabGuard {
    fn drop(&mut self) {
        SLAB_PTR.with(|c| c.set(self.prev_ptr));
        SLAB_FREE.with(|c| c.set(self.prev_free));
    }
}

// =============================================================================
// Slab-based spawn
// =============================================================================

/// Allocate a task in the slab and return its raw pointer.
///
/// The task is constructed with a slab-aware `free_fn` and placed
/// directly into a slab slot via the TLS slab pointer.
///
/// # Panics
///
/// Panics if no slab is configured.
pub(crate) fn slab_spawn<F: Future<Output = ()> + 'static, const S: usize>(
    future: F,
    tracker_key: u32,
) -> *mut u8 {
    let slab_ptr = SLAB_PTR.with(Cell::get);
    assert!(
        !slab_ptr.is_null(),
        "spawn_pinned() called without a slab configured — \
         use Runtime::builder().slab::<S>(capacity) to enable slab allocation"
    );

    let task = Task::new_with_free(future, tracker_key, slab_free_task);

    // SAFETY: slab_ptr points to a valid Slab<S> installed by the runtime.
    let slab = unsafe {
        &*(slab_ptr as *const nexus_slab::byte::unbounded::Slab<S>)
    };
    let slot = slab.alloc(task);
    slot.into_raw()
}

/// Free function stored in slab-allocated task headers.
///
/// Reads the slab pointer and free fn from TLS, calls through.
///
/// # Safety
///
/// `ptr` must point to a slab-allocated task.
unsafe fn slab_free_task(ptr: *mut u8) {
    let slab_ptr = SLAB_PTR.with(Cell::get);
    let free_fn = SLAB_FREE.with(Cell::get);
    // SAFETY: slab_ptr and free_fn were installed by the runtime.
    unsafe { free_fn(slab_ptr, ptr) };
}

// =============================================================================
// SlabAlloc — owned by Runtime, provides TLS pointers
// =============================================================================

/// Slab allocator instance. Owned by the runtime.
///
/// The const generic `S` is the slot size (use [`slot_size`](crate::slot_size)).
pub struct SlabAlloc<const S: usize> {
    slab: nexus_slab::byte::unbounded::Slab<S>,
}

impl<const S: usize> SlabAlloc<S> {
    /// Create a slab allocator with `capacity` initial slots.
    pub fn new(capacity: usize) -> Self {
        // SAFETY: single-threaded use. Slot lifetimes managed by Executor.
        let slab = unsafe {
            nexus_slab::byte::unbounded::Slab::<S>::with_chunk_capacity(capacity)
        };
        Self { slab }
    }

    /// Get the raw pointer for TLS installation.
    pub(crate) fn as_ptr(&self) -> *const u8 {
        std::ptr::from_ref(&self.slab).cast::<u8>()
    }

    /// The free function pointer for TLS.
    pub(crate) fn free_fn() -> FreeFn {
        free_impl::<S>
    }
}

/// Slab free implementation. Releases the slot back to the slab.
///
/// # Safety
///
/// `slab_ptr` must point to a valid `Slab<S>`.
/// `ptr` must have been returned by `slab.alloc()` on the same slab.
unsafe fn free_impl<const S: usize>(slab_ptr: *const u8, ptr: *mut u8) {
    let slab = unsafe {
        &*(slab_ptr as *const nexus_slab::byte::unbounded::Slab<S>)
    };
    let slot = unsafe { nexus_slab::byte::Slot::<u8>::from_raw(ptr) };
    slab.free(slot);
}
