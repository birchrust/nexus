//! High-performance collections with slab-backed storage.
//!
//! Collections use thread-local slab allocators for O(1) insert/remove with
//! stable handles and no allocation on the hot path.
//!
//! # Collections
//!
//! - **List** — Doubly-linked list with `RcSlot` handles and external allocation
//! - **Heap** — Pairing heap with `RcSlot` handles and external allocation
//! - **RbTree** — Red-black tree sorted map with deterministic O(log n) worst case
//! - **BTree** — B-tree sorted map with cache-friendly, tunable node layout
//!
//! # Quick Start (RbTree)
//!
//! ```ignore
//! mod levels {
//!     nexus_collections::rbtree_allocator!(u64, String, bounded);
//! }
//!
//! fn main() {
//!     levels::Allocator::builder().capacity(1000).build().unwrap();
//!
//!     let mut map = levels::RbTree::new(levels::Allocator);
//!     map.try_insert(100, "hello".into()).unwrap();
//!     assert_eq!(map.get(&100), Some(&"hello".into()));
//! }
//! ```

#![warn(missing_docs)]

use std::cell::Cell;

pub mod btree;
pub mod compare;
pub mod exclusive;
pub mod heap;
pub mod list;
mod macros;
pub mod rbtree;

// Re-export comparison types at crate root
pub use compare::{Compare, Natural, Reverse};
// Re-export ExclusiveCell types at crate root
pub use exclusive::{ExMut, ExRef, ExclusiveCell};
// Re-export slab traits so users can call Allocator::is_initialized() etc.
// without depending on nexus-slab directly
pub use nexus_slab::{Alloc, BoundedAlloc, Full, UnboundedAlloc};

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
