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
use nexus_slab::Slot;
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
    fn free(&self, slot: Slot<Self::Item>);

    /// Moves the value out and returns the slot to the freelist.
    fn take(&self, slot: Slot<Self::Item>) -> Self::Item;
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
    fn free(&self, slot: Slot<T>) {
        bounded::Slab::free(self, slot)
    }

    #[inline]
    fn take(&self, slot: Slot<T>) -> T {
        bounded::Slab::take(self, slot)
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
unsafe impl<T> SlabStore for unbounded::Slab<T> {
    type Item = T;

    #[inline]
    fn alloc(&self, value: T) -> Slot<T> {
        unbounded::Slab::alloc(self, value)
    }

    #[inline]
    fn free(&self, slot: Slot<T>) {
        unbounded::Slab::free(self, slot)
    }

    #[inline]
    fn take(&self, slot: Slot<T>) -> T {
        unbounded::Slab::take(self, slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_store_roundtrip() {
        // SAFETY: single-threaded test, slab contract upheld
        let slab = unsafe { bounded::Slab::<u64>::with_capacity(16) };
        let slot = SlabStore::alloc(&slab, 42);
        assert_eq!(*slot, 42);
        let val = SlabStore::take(&slab, slot);
        assert_eq!(val, 42);
    }

    #[test]
    fn unbounded_store_roundtrip() {
        // SAFETY: single-threaded test, slab contract upheld
        let slab = unsafe { unbounded::Slab::<u64>::with_chunk_capacity(16) };
        let slot = SlabStore::alloc(&slab, 99);
        assert_eq!(*slot, 99);
        SlabStore::free(&slab, slot);
    }

    #[test]
    fn bounded_try_alloc_graceful() {
        // SAFETY: single-threaded test, slab contract upheld
        let slab = unsafe { bounded::Slab::<u64>::with_capacity(1) };
        let s1 = BoundedStore::try_alloc(&slab, 1).unwrap();
        let err = BoundedStore::try_alloc(&slab, 2).unwrap_err();
        assert_eq!(err.into_inner(), 2);
        SlabStore::free(&slab, s1);
    }

    #[test]
    #[should_panic(expected = "capacity exceeded")]
    fn bounded_alloc_panics_on_full() {
        // SAFETY: single-threaded test, slab contract upheld
        let slab = unsafe { bounded::Slab::<u64>::with_capacity(1) };
        let _s1 = SlabStore::alloc(&slab, 1);
        let _s2 = SlabStore::alloc(&slab, 2); // panics
    }
}
