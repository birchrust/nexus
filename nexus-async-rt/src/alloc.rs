//! Slab task allocator — optional, power-user feature.
//!
//! By default, tasks are Box-allocated. For zero-alloc hot-path spawning,
//! configure a slab via [`RuntimeBuilder::slab`] and use [`spawn_slab`].
//!
//! The slab is accessed via thread-local pointers — no dynamic dispatch,
//! no Option checks. Default pointers panic with a clear message if
//! `spawn_slab` is called without a slab configured.

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

/// Panic stub.
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
        "spawn_slab() called without a slab configured — \
         use Runtime::builder().slab(slab) to enable slab allocation"
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
/// # Safety
///
/// `ptr` must point to a slab-allocated task.
unsafe fn slab_free_task(ptr: *mut u8) {
    let slab_ptr = SLAB_PTR.with(Cell::get);
    let free_fn = SLAB_FREE.with(Cell::get);
    unsafe { free_fn(slab_ptr, ptr) };
}

// =============================================================================
// Free fn (monomorphized per S)
// =============================================================================

/// Returns the free function pointer for a slab with slot size `S`.
pub(crate) fn slab_free_fn<const S: usize>() -> FreeFn {
    free_impl::<S>
}

unsafe fn free_impl<const S: usize>(slab_ptr: *const u8, ptr: *mut u8) {
    let slab = unsafe {
        &*(slab_ptr as *const nexus_slab::byte::unbounded::Slab<S>)
    };
    let slot = unsafe { nexus_slab::byte::Slot::<u8>::from_raw(ptr) };
    slab.free(slot);
}
