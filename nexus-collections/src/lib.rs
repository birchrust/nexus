//! High-performance collections with slab-backed storage.
//!
//! Collections use thread-local slab allocators for O(1) insert/remove with
//! stable handles and no allocation on the hot path.
//!
//! # Collections
//!
//! - **List** — Doubly-linked list with `RcSlot` handles and external allocation
//! - **Heap** — Pairing heap with `RcSlot` handles and external allocation
//! - **SkipList** — Sorted map with internal allocation (user sees only K/V)
//!
//! # Quick Start (SkipList)
//!
//! ```ignore
//! mod levels {
//!     nexus_collections::skip_allocator!(u64, String, bounded);
//! }
//!
//! fn main() {
//!     levels::Allocator::builder().capacity(1000).build().unwrap();
//!
//!     let mut map = levels::SkipList::new(levels::Allocator);
//!     map.try_insert(100, "hello".into()).unwrap();
//!     assert_eq!(map.get(&100), Some(&"hello".into()));
//! }
//! ```

#![warn(missing_docs)]

pub mod exclusive;
pub mod heap;
pub mod list;
mod macros;
pub mod skiplist;

// Re-export ExclusiveCell types at crate root
pub use exclusive::{ExMut, ExRef, ExclusiveCell};
