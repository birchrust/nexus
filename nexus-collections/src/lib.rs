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
// RcFree trait — unifies bounded and unbounded slab free
// =============================================================================

/// Trait for slab types that can free an [`RcSlot`] handle.
///
/// Implemented by both `rc::bounded::Slab<T>` and `rc::unbounded::Slab<T>`.
/// Collections use this to accept either slab type in methods that release
/// references (unlink, clear).
///
/// Choose one slab type per collection and stick with it — don't mix
/// bounded and unbounded on the same collection.
pub trait RcFree<T> {
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
// SlabFree trait — unifies bounded and unbounded raw slab free
// =============================================================================

/// Trait for raw slab types that can free a [`nexus_slab::Slot`] handle.
///
/// Implemented by both `bounded::Slab<T>` and `unbounded::Slab<T>`.
/// Used by tree collections (RbTree, BTree) which own nodes directly.
pub trait SlabFree<T> {
    /// Free a slot, dropping the value and returning storage to the freelist.
    fn free_slot(&self, slot: nexus_slab::Slot<T>);

    /// Take the value out of a slot, then free the storage.
    fn take_slot(&self, slot: nexus_slab::Slot<T>) -> T;
}

impl<T> SlabFree<T> for nexus_slab::bounded::Slab<T> {
    #[inline]
    fn free_slot(&self, slot: nexus_slab::Slot<T>) {
        self.free(slot);
    }

    #[inline]
    fn take_slot(&self, slot: nexus_slab::Slot<T>) -> T {
        self.take(slot)
    }
}

impl<T> SlabFree<T> for nexus_slab::unbounded::Slab<T> {
    #[inline]
    fn free_slot(&self, slot: nexus_slab::Slot<T>) {
        self.free(slot);
    }

    #[inline]
    fn take_slot(&self, slot: nexus_slab::Slot<T>) -> T {
        self.take(slot)
    }
}

// =============================================================================
// Collection identity
// =============================================================================

thread_local! {
    static NEXT_COLLECTION_ID: Cell<usize> = const { Cell::new(1) };
}

/// Returns a unique (per-thread) collection ID for ownership checking.
fn next_collection_id() -> usize {
    NEXT_COLLECTION_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}
