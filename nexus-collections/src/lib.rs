//! High-performance collections with slab-backed storage.
//!
//! Collections use external slab allocators passed by reference. The user
//! creates the slab, the collection wires pointers.
//!
//! # Collections
//!
//! - **List** — Doubly-linked list with `RcSlot` handles and external allocation
//! - **Heap** — Pairing heap with `RcSlot` handles and external allocation
//! - **RbTree** — Red-black tree sorted map with deterministic O(log n) worst case
//! - **BTree** — B-tree sorted map with cache-friendly, tunable node layout
//!
//! # Quick Start (List)
//!
//! ```ignore
//! use nexus_slab::rc::bounded::Slab;
//! use nexus_collections::list::{List, ListNode};
//!
//! let slab = unsafe { Slab::<ListNode<u64>>::with_capacity(1000) };
//! let mut list = List::new();
//! let handle = slab.alloc(ListNode::new(42));
//! list.link_back(&handle);
//! ```

#![warn(missing_docs)]

use std::cell::Cell;

pub mod btree;
pub mod compare;
pub mod heap;
pub mod list;
pub mod rbtree;

// Re-export comparison types at crate root
pub use compare::{Compare, Natural, Reverse};
// Re-export slab handle and error types for convenience
pub use nexus_slab::rc::{RcSlot, Ref, RefMut};
pub use nexus_slab::shared::Full;

// =============================================================================
// Sealed — prevents external trait implementations
// =============================================================================

mod sealed {
    /// Sealed marker for Rc slab types.
    pub trait RcSealed {}
    /// Sealed marker for raw slab types.
    pub trait SlabSealed {}

    impl<T> RcSealed for nexus_slab::rc::bounded::Slab<T> {}
    impl<T> RcSealed for nexus_slab::rc::unbounded::Slab<T> {}
    impl<T> SlabSealed for nexus_slab::bounded::Slab<T> {}
    impl<T> SlabSealed for nexus_slab::unbounded::Slab<T> {}
}

// =============================================================================
// RcFree trait — sealed, unifies bounded and unbounded Rc slab free
// =============================================================================

/// Sealed trait for Rc slab types that can free an [`RcSlot`] handle.
///
/// Implemented by `rc::bounded::Slab<T>` and `rc::unbounded::Slab<T>`.
/// Used by list and heap for `unlink` and `clear`.
///
/// This trait is sealed — it cannot be implemented outside this crate.
pub trait RcFree<T>: sealed::RcSealed {
    /// Free a handle, decrementing the refcount.
    /// Deallocates the slot when the last handle is freed.
    fn free_rc(&self, handle: RcSlot<T>);
}

impl<T> RcFree<T> for nexus_slab::rc::bounded::Slab<T> {
    #[inline]
    fn free_rc(&self, handle: RcSlot<T>) {
        self.free(handle);
    }
}

impl<T> RcFree<T> for nexus_slab::rc::unbounded::Slab<T> {
    #[inline]
    fn free_rc(&self, handle: RcSlot<T>) {
        self.free(handle);
    }
}

// =============================================================================
// SlabOps trait — unifies bounded and unbounded raw slab free
// =============================================================================

/// Sealed trait for raw slab operations.
///
/// Implemented by `bounded::Slab<T>` and `unbounded::Slab<T>`.
/// Used by tree collections for `remove`, `clear`, cursor, entry, and drain.
///
/// Methods that allocate (insert) are typed to the specific slab variant;
/// this trait covers the shared operations (free, take, contains).
///
/// This trait is sealed — it cannot be implemented outside this crate.
pub trait SlabOps<T>: sealed::SlabSealed {
    /// Free a slot, dropping the value and returning storage to the freelist.
    fn free_slot(&self, slot: nexus_slab::Slot<T>);

    /// Take the value out of a slot, then free the storage.
    fn take_slot(&self, slot: nexus_slab::Slot<T>) -> T;

    /// Returns true if the pointer falls within this slab's storage.
    fn contains_ptr(&self, ptr: *const ()) -> bool;
}

impl<T> SlabOps<T> for nexus_slab::bounded::Slab<T> {
    #[inline]
    fn free_slot(&self, slot: nexus_slab::Slot<T>) {
        self.free(slot);
    }

    #[inline]
    fn take_slot(&self, slot: nexus_slab::Slot<T>) -> T {
        self.take(slot)
    }

    #[inline]
    fn contains_ptr(&self, ptr: *const ()) -> bool {
        self.contains_ptr(ptr)
    }
}

impl<T> SlabOps<T> for nexus_slab::unbounded::Slab<T> {
    #[inline]
    fn free_slot(&self, slot: nexus_slab::Slot<T>) {
        self.free(slot);
    }

    #[inline]
    fn take_slot(&self, slot: nexus_slab::Slot<T>) -> T {
        self.take(slot)
    }

    #[inline]
    fn contains_ptr(&self, ptr: *const ()) -> bool {
        self.contains_ptr(ptr)
    }
}

// =============================================================================
// Collection identity
// =============================================================================

thread_local! {
    static NEXT_COLLECTION_ID: Cell<usize> = const { Cell::new(1) };
}

/// Returns a unique (per-thread) collection ID for ownership checking.
///
/// IDs are per-thread and wrap after `usize::MAX` allocations. On 64-bit,
/// this is ~18 quintillion — effectively impossible. On 32-bit, wrapping
/// after ~4B creates is theoretically possible but practically unreachable
/// for a slab-backed collection (slab capacity would be exhausted first).
/// We skip ID 0 to avoid colliding with potential sentinel values.
fn next_collection_id() -> usize {
    NEXT_COLLECTION_ID.with(|c| {
        let id = c.get();
        let next = id.wrapping_add(1);
        c.set(if next == 0 { 1 } else { next });
        id
    })
}
