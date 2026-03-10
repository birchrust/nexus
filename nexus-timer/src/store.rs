//! Slab storage traits for instance-based (non-ZST) slabs.
//!
//! Single trait hierarchy: [`SlabStore`] provides allocation and deallocation.
//! [`BoundedStore`] extends it with fallible allocation for callers who want
//! graceful error handling.
//!
//! # Allocation and OOM
//!
//! [`SlabStore::alloc`] always returns a valid slot. For unbounded slabs this
//! is guaranteed by growth. For bounded slabs, exceeding capacity **panics**.
//!
//! This is a deliberate design choice: bounded capacity is a deployment
//! constraint, not a runtime negotiation. If you hit the limit, your capacity
//! planning is wrong and the system should fail loudly — the same way a
//! process panics on OOM. Silently dropping timers or events is worse than
//! crashing.
//!
//! Use [`BoundedStore::try_alloc`] if you need graceful error handling at
//! specific call sites.

use nexus_slab::Full;
use nexus_slab::shared::{Slot, SlotCell};
use nexus_slab::{bounded, unbounded};

// Re-export concrete slab types so downstream crates (nexus-rt) can name
// them in type defaults without adding nexus-slab as a direct dependency.
pub use bounded::Slab as BoundedSlab;
pub use unbounded::Slab as UnboundedSlab;

// =============================================================================
// Traits
// =============================================================================

/// Base trait for slab storage — allocation, deallocation, and value extraction.
///
/// # Allocation
///
/// [`alloc`](Self::alloc) always returns a valid slot:
///
/// - **Unbounded slabs** grow as needed — allocation never fails.
/// - **Bounded slabs** panic if capacity is exceeded. This is intentional:
///   running out of pre-allocated capacity is a capacity planning error,
///   equivalent to OOM. The system should crash loudly rather than silently
///   drop work.
///
/// For fallible allocation on bounded slabs, use [`BoundedStore::try_alloc`].
///
/// # Safety
///
/// Implementors must uphold:
///
/// - `free` must drop the value and return the slot to the freelist.
/// - `take` must move the value out and return the slot to the freelist.
/// - The slot must have been allocated from `self`.
pub unsafe trait SlabStore {
    /// The type stored in each slot.
    type Item;

    /// Allocates a slot with the given value.
    ///
    /// # Panics
    ///
    /// Panics if the store is at capacity (bounded slabs only). This is a
    /// capacity planning error — size your slabs for peak load.
    fn alloc(&self, value: Self::Item) -> Slot<Self::Item>;

    /// Drops the value and returns the slot to the freelist.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from this store.
    /// - No references to the slot's value may exist.
    unsafe fn free(&self, slot: Slot<Self::Item>);

    /// Moves the value out and returns the slot to the freelist.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from this store.
    /// - No references to the slot's value may exist.
    unsafe fn take(&self, slot: Slot<Self::Item>) -> Self::Item;

    /// Returns a slot to the freelist by raw pointer.
    ///
    /// Does NOT drop the value — caller must have already dropped or moved it.
    ///
    /// # Safety
    ///
    /// - `ptr` must point to a slot within this store.
    /// - The value must already be dropped or moved out.
    unsafe fn free_ptr(&self, ptr: *mut SlotCell<Self::Item>);
}

/// Bounded (fixed-capacity) storage — provides fallible allocation.
///
/// Use [`try_alloc`](Self::try_alloc) when you need graceful error handling.
/// For the common case where capacity exhaustion is a fatal error, use
/// [`SlabStore::alloc`] directly (it panics on bounded-full).
pub trait BoundedStore: SlabStore {
    /// Attempts to allocate a slot with the given value.
    ///
    /// Returns `Err(Full(value))` if storage is at capacity.
    fn try_alloc(&self, value: Self::Item) -> Result<Slot<Self::Item>, Full<Self::Item>>;
}

// =============================================================================
// Impls for bounded::Slab
// =============================================================================

// SAFETY: bounded::Slab::free drops the value and returns to freelist.
// bounded::Slab::take moves value out and returns to freelist.
// bounded::Slab::free_ptr returns slot to freelist without dropping.
unsafe impl<T> SlabStore for bounded::Slab<T> {
    type Item = T;

    #[inline]
    fn alloc(&self, value: T) -> Slot<T> {
        self.try_alloc(value).unwrap_or_else(|full| {
            // Drop the value inside Full, then panic.
            drop(full);
            panic!(
                "bounded slab: capacity exceeded (type: {})",
                std::any::type_name::<T>(),
            );
        })
    }

    #[inline]
    unsafe fn free(&self, slot: Slot<T>) {
        // SAFETY: caller guarantees slot was allocated from this slab
        unsafe { bounded::Slab::free(self, slot) }
    }

    #[inline]
    unsafe fn take(&self, slot: Slot<T>) -> T {
        // SAFETY: caller guarantees slot was allocated from this slab
        unsafe { bounded::Slab::take(self, slot) }
    }

    #[inline]
    unsafe fn free_ptr(&self, ptr: *mut SlotCell<T>) {
        // SAFETY: caller guarantees ptr is within this slab
        unsafe { bounded::Slab::free_ptr(self, ptr) }
    }
}

impl<T> BoundedStore for bounded::Slab<T> {
    #[inline]
    fn try_alloc(&self, value: T) -> Result<Slot<T>, Full<T>> {
        bounded::Slab::try_alloc(self, value)
    }
}

// =============================================================================
// Impls for unbounded::Slab
// =============================================================================

// SAFETY: unbounded::Slab::free drops the value and returns to freelist.
// unbounded::Slab::take moves value out and returns to freelist.
// unbounded::Slab::free_ptr returns slot to freelist without dropping.
unsafe impl<T> SlabStore for unbounded::Slab<T> {
    type Item = T;

    #[inline]
    fn alloc(&self, value: T) -> Slot<T> {
        unbounded::Slab::alloc(self, value)
    }

    #[inline]
    unsafe fn free(&self, slot: Slot<T>) {
        // SAFETY: caller guarantees slot was allocated from this slab
        unsafe { unbounded::Slab::free(self, slot) }
    }

    #[inline]
    unsafe fn take(&self, slot: Slot<T>) -> T {
        // SAFETY: caller guarantees slot was allocated from this slab
        unsafe { unbounded::Slab::take(self, slot) }
    }

    #[inline]
    unsafe fn free_ptr(&self, ptr: *mut SlotCell<T>) {
        // SAFETY: caller guarantees ptr is within this slab
        unsafe { unbounded::Slab::free_ptr(self, ptr) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_store_roundtrip() {
        let slab = bounded::Slab::<u64>::with_capacity(16);
        let slot = SlabStore::alloc(&slab, 42);
        assert_eq!(*slot, 42);
        // SAFETY: slot was allocated from this slab
        let val = unsafe { SlabStore::take(&slab, slot) };
        assert_eq!(val, 42);
    }

    #[test]
    fn unbounded_store_roundtrip() {
        let slab = unbounded::Slab::<u64>::with_chunk_capacity(16);
        let slot = SlabStore::alloc(&slab, 99);
        assert_eq!(*slot, 99);
        // SAFETY: slot was allocated from this slab
        unsafe { SlabStore::free(&slab, slot) };
    }

    #[test]
    fn bounded_try_alloc_graceful() {
        let slab = bounded::Slab::<u64>::with_capacity(1);
        let _s1 = BoundedStore::try_alloc(&slab, 1).unwrap();
        let err = BoundedStore::try_alloc(&slab, 2).unwrap_err();
        assert_eq!(err.into_inner(), 2);
    }

    #[test]
    #[should_panic(expected = "capacity exceeded")]
    fn bounded_alloc_panics_on_full() {
        let slab = bounded::Slab::<u64>::with_capacity(1);
        let _s1 = SlabStore::alloc(&slab, 1);
        let _s2 = SlabStore::alloc(&slab, 2); // panics
    }
}
