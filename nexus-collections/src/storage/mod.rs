//! Storage backends for slab-like containers with stable keys.
//!
//! This module provides storage implementations for the collection types.
//! Storage owns the data and provides stable keys for access.
//!
//! # Storage Types
//!
//! Each collection has dedicated storage types that hide internal node structure:
//!
//! | Collection | Bounded | Growable |
//! |------------|---------|----------|
//! | [`List`](crate::List) | [`ListStorage<T>`] | [`GrowableListStorage<T>`] |
//! | [`Heap`](crate::Heap) | [`HeapStorage<T>`] | [`GrowableHeapStorage<T>`] |
//!
//! # Example
//!
//! ```
//! use nexus_collections::{List, ListStorage};
//!
//! // Create storage - capacity is the only parameter
//! let mut storage: ListStorage<u64> = ListStorage::with_capacity(1000);
//!
//! // Use with List
//! let mut list: List<u64, ListStorage<u64>> = List::new();
//! let key = list.try_push_back(&mut storage, 42).unwrap();
//! assert_eq!(list.get(&storage, key), Some(&42));
//! ```

mod heap;
mod list;
// Re-export specialized storage types
pub use heap::{
    BoundedHeapStorageOps, GrowableHeapStorage, GrowableHeapStorageOps, GrowableHeapVacant,
    HeapEntry, HeapNode, HeapRef, HeapRefMut, HeapStorage, HeapStorageOps, HeapVacant,
};
pub use list::{
    BoundedListStorageOps, GrowableListStorage, GrowableListStorageOps, GrowableListVacant,
    ListEntry, ListNode, ListRef, ListRefMut, ListStorage, ListStorageOps, ListVacant,
};
// Internal exports for crate use
pub(crate) use heap::HEAP_POS_NONE;

// =============================================================================
// Error Type
// =============================================================================

/// Error returned when fixed-capacity storage is full.
///
/// Contains the value that could not be inserted, allowing recovery.
/// Modeled after `std::sync::mpsc::SendError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full<T>(pub T);

impl<T> Full<T> {
    /// Returns the value that could not be inserted.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> core::fmt::Display for Full<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "storage is full")
    }
}

impl<T: core::fmt::Debug> std::error::Error for Full<T> {}
