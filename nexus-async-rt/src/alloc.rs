//! Slab task allocator — optional, power-user feature.
//!
//! By default, tasks are Box-allocated. For zero-alloc hot-path spawning,
//! configure a slab via [`RuntimeBuilder::slab_unbounded`] or
//! [`RuntimeBuilder::slab_bounded`] and use [`spawn_slab`].
//!
//! The slab is accessed via thread-local function pointers. Each fn pointer
//! is monomorphized at slab install time with the correct slot size `S`.
//! `spawn_slab` is not generic over `S` — the caller doesn't need to know it.

use std::cell::Cell;
use std::future::Future;

use crate::task::Task;

// =============================================================================
// TLS slots
// =============================================================================

/// Claim a slab slot, copy `size` bytes from `src`, return raw pointer.
/// Returns null if the slab is full (bounded only).
type ClaimFn = unsafe fn(src: *const u8, size: usize) -> *mut u8;

/// Free a slab slot.
type FreeFn = unsafe fn(ptr: *mut u8);

thread_local! {
    /// Raw pointer to the slab instance.
    static SLAB_PTR: Cell<*const u8> = const { Cell::new(std::ptr::null()) };

    /// Fn pointer: claim a slot and copy task bytes into it.
    static SLAB_CLAIM: Cell<ClaimFn> = const { Cell::new(no_slab_claim) };

    /// Fn pointer: free a slab slot.
    static SLAB_FREE: Cell<FreeFn> = const { Cell::new(no_slab_free) };
}

/// Panic stub — called when `spawn_slab` is used without a slab.
unsafe fn no_slab_claim(_src: *const u8, _size: usize) -> *mut u8 {
    panic!(
        "spawn_slab() called without a slab configured — \
         use Runtime::builder().slab_unbounded(slab) or .slab_bounded(slab)"
    )
}

/// Panic stub.
unsafe fn no_slab_free(_ptr: *mut u8) {
    panic!("slab free called without a slab configured")
}

// =============================================================================
// TLS install/guard
// =============================================================================

/// Configuration for slab TLS installation.
pub(crate) struct SlabTlsConfig {
    pub(crate) slab_ptr: *const u8,
    pub(crate) claim_fn: ClaimFn,
    pub(crate) free_fn: FreeFn,
}

/// Install slab TLS from a config. Returns RAII guard.
pub(crate) fn install_slab(config: &SlabTlsConfig) -> SlabGuard {
    let prev_ptr = SLAB_PTR.with(|c| c.replace(config.slab_ptr));
    let prev_claim = SLAB_CLAIM.with(|c| c.replace(config.claim_fn));
    let prev_free = SLAB_FREE.with(|c| c.replace(config.free_fn));
    SlabGuard {
        prev_ptr,
        prev_claim,
        prev_free,
    }
}

#[allow(clippy::struct_field_names)]
pub(crate) struct SlabGuard {
    prev_ptr: *const u8,
    prev_claim: ClaimFn,
    prev_free: FreeFn,
}

impl Drop for SlabGuard {
    fn drop(&mut self) {
        SLAB_PTR.with(|c| c.set(self.prev_ptr));
        SLAB_CLAIM.with(|c| c.set(self.prev_claim));
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
/// - If no slab is configured.
/// - If the slab is full (bounded slab).
/// - If the task exceeds the slab's slot size.
pub(crate) fn slab_spawn<F: Future<Output = ()> + 'static>(
    future: F,
    tracker_key: u32,
) -> *mut u8 {
    let task = Task::new_with_free(future, tracker_key, slab_free_task);
    let size = std::mem::size_of_val(&task);
    let src = std::ptr::from_ref(&task).cast::<u8>();

    let claim = SLAB_CLAIM.with(Cell::get);
    // SAFETY: claim copies `size` bytes from `src` into a slab slot.
    let ptr = unsafe { claim(src, size) };
    assert!(!ptr.is_null(), "slab full — spawn_slab failed");

    // Task was copied into the slab. Prevent stack drop.
    std::mem::forget(task);

    ptr
}

/// Free function stored in slab-allocated task headers.
unsafe fn slab_free_task(ptr: *mut u8) {
    let free = SLAB_FREE.with(Cell::get);
    unsafe { free(ptr) };
}

// =============================================================================
// Monomorphized fn pointers (created at builder time)
// =============================================================================

/// Build TLS config for an unbounded slab.
pub(crate) fn make_unbounded_config<const S: usize>(slab_ptr: *const u8) -> SlabTlsConfig {
    SlabTlsConfig {
        slab_ptr,
        claim_fn: unbounded_claim::<S>,
        free_fn: unbounded_free::<S>,
    }
}

/// Build TLS config for a bounded slab.
pub(crate) fn make_bounded_config<const S: usize>(slab_ptr: *const u8) -> SlabTlsConfig {
    SlabTlsConfig {
        slab_ptr,
        claim_fn: bounded_claim::<S>,
        free_fn: bounded_free::<S>,
    }
}

// -- Unbounded --

unsafe fn unbounded_claim<const S: usize>(src: *const u8, size: usize) -> *mut u8 {
    let slab_ptr = SLAB_PTR.with(Cell::get);
    let slab = unsafe {
        &*(slab_ptr as *const nexus_slab::byte::unbounded::Slab<S>)
    };
    unsafe { slab.alloc_raw(src, size) }
}

unsafe fn unbounded_free<const S: usize>(ptr: *mut u8) {
    let slab_ptr = SLAB_PTR.with(Cell::get);
    let slab = unsafe {
        &*(slab_ptr as *const nexus_slab::byte::unbounded::Slab<S>)
    };
    let slot = unsafe { nexus_slab::byte::Slot::<u8>::from_raw(ptr) };
    slab.free(slot);
}

// -- Bounded --

unsafe fn bounded_claim<const S: usize>(src: *const u8, size: usize) -> *mut u8 {
    let slab_ptr = SLAB_PTR.with(Cell::get);
    let slab = unsafe {
        &*(slab_ptr as *const nexus_slab::byte::bounded::Slab<S>)
    };
    unsafe { slab.alloc_raw(src, size) }
}

unsafe fn bounded_free<const S: usize>(ptr: *mut u8) {
    let slab_ptr = SLAB_PTR.with(Cell::get);
    let slab = unsafe {
        &*(slab_ptr as *const nexus_slab::byte::bounded::Slab<S>)
    };
    let slot = unsafe { nexus_slab::byte::Slot::<u8>::from_raw(ptr) };
    slab.free(slot);
}
