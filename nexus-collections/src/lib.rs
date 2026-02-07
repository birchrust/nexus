//! High-performance collections with slab-backed storage.
//!
//! Collections use thread-local `RcSlot`-based slab allocators for O(1)
//! insert/remove with stable handles and no allocation on the hot path.
//!
//! # Design Philosophy
//!
//! - **RcSlot handles** — User holds ownership token, data accessible via guard
//! - **ExclusiveCell** — Interior mutability with exclusive-borrow semantics
//! - **List manages links** — Raw pointer bookkeeping with refcount safety
//! - **No closures** — Guard-based access replaces closure-based API
//!
//! # Quick Start
//!
//! ```ignore
//! use nexus_collections::list_allocator;
//!
//! struct Order { id: u64, price: f64 }
//!
//! list_allocator!(orders, Order);
//!
//! fn main() {
//!     orders::Allocator::builder().capacity(1000).build().unwrap();
//!
//!     let mut list = orders::List::new();
//!     let handle = orders::create_node(Order { id: 1, price: 100.0 }).unwrap();
//!     list.link_back(&handle);
//!
//!     // Guard-based access via auto-deref
//!     let price = handle.exclusive().price;
//!     handle.exclusive_mut().price = 200.0;
//! }
//! ```

#![warn(missing_docs)]

pub mod exclusive;
pub mod list;
mod macros;

// Re-export ExclusiveCell types at crate root
pub use exclusive::{ExMut, ExRef, ExclusiveCell};
