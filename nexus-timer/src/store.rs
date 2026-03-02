//! Slab storage traits for instance-based (non-ZST) slabs.
//!
//! These mirror the `Alloc`/`BoundedAlloc`/`UnboundedAlloc` hierarchy from
//! nexus-slab but work with instance-based slabs rather than ZST/TLS-backed
//! allocators. We own the traits here (orphan rules satisfied).

use nexus_slab::Full;
use nexus_slab::shared::{Slot, SlotCell};
use nexus_slab::{bounded, unbounded};

// =============================================================================
// Traits
// =============================================================================

/// Base trait for slab storage — deallocation and value extraction.
///
/// # Safety
///
/// - `free` must drop the value and return the slot to the freelist.
/// - `take` must move the value out and return the slot to the freelist.
/// - The slot must have been allocated from `self`.
pub unsafe trait SlabStore {
    /// The type stored in each slot.
    type Item;

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

/// Bounded (fixed-capacity) storage — allocation can fail.
pub trait BoundedStore: SlabStore {
    /// Attempts to allocate a slot with the given value.
    ///
    /// Returns `Err(Full(value))` if storage is at capacity.
    fn try_alloc(&self, value: Self::Item) -> Result<Slot<Self::Item>, Full<Self::Item>>;
}

/// Unbounded (growable) storage — allocation always succeeds.
pub trait UnboundedStore: SlabStore {
    /// Allocates a slot with the given value. Always succeeds (grows if needed).
    fn alloc(&self, value: Self::Item) -> Slot<Self::Item>;
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

impl<T> UnboundedStore for unbounded::Slab<T> {
    #[inline]
    fn alloc(&self, value: T) -> Slot<T> {
        unbounded::Slab::alloc(self, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_store_roundtrip() {
        let slab = bounded::Slab::<u64>::with_capacity(16);
        let slot = BoundedStore::try_alloc(&slab, 42).unwrap();
        assert_eq!(*slot, 42);
        // SAFETY: slot was allocated from this slab
        let val = unsafe { SlabStore::take(&slab, slot) };
        assert_eq!(val, 42);
    }

    #[test]
    fn unbounded_store_roundtrip() {
        let slab = unbounded::Slab::<u64>::with_chunk_capacity(16);
        let slot = UnboundedStore::alloc(&slab, 99);
        assert_eq!(*slot, 99);
        // SAFETY: slot was allocated from this slab
        unsafe { SlabStore::free(&slab, slot) };
    }

    #[test]
    fn bounded_store_full() {
        let slab = bounded::Slab::<u64>::with_capacity(1);
        let _s1 = BoundedStore::try_alloc(&slab, 1).unwrap();
        let err = BoundedStore::try_alloc(&slab, 2).unwrap_err();
        assert_eq!(err.into_inner(), 2);
    }
}
