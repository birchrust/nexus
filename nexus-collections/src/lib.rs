//! High-performance collections with TLS-based storage.
//!
//! Collections use thread-local slab allocators for O(1) insert/remove
//! with stable keys and no allocation on the hot path.
//!
//! # Design Philosophy
//!
//! - **Slab owns data** - All data lives in TLS slab storage
//! - **Collections manage structure** - Lists manage prev/next links
//! - **Slots are opaque handles** - No direct data access, use closures
//! - **Borrow checker enforces safety** - Closure-based access prevents aliasing
//!
//! # Quick Start
//!
//! ```ignore
//! use nexus_collections::list_allocator;
//!
//! #[derive(Debug)]
//! struct Order { id: u64, price: f64 }
//!
//! // Create a typed list allocator
//! list_allocator!(orders, Order);
//!
//! fn main() {
//!     // Initialize at startup
//!     orders::init().bounded(1000).build();
//!
//!     // Create and use
//!     let mut list = orders::List::new();
//!     let node = orders::create_node(Order { id: 1, price: 100.0 }).unwrap();
//!     let slot = list.link_back(node);
//!
//!     // Access via closures
//!     let price = list.read(&slot, |o| o.price);
//!     println!("Price: {}", price);
//! }
//! ```
//!
//! # User Invariants
//!
//! 1. **Consume all guards** - `Detached` must call `take()` or `try_take()`
//! 2. **Unlink slots before dropping** - Don't drop `ListSlot` while linked
//! 3. **Keep your index in sync** - Track slots in your HashMap

#![warn(missing_docs)]

mod macros;

pub mod list;

// The list_allocator! macro is automatically exported via #[macro_export]

// Re-export list types for use in macro-generated code and direct use
pub use list::{
    Cursor, CursorGuard, Detached, DetachedListNode, Id as ListId, List, ListSlot, Node as ListNode,
};

/// Private module for macro implementation details.
///
/// These are not part of the public API and may change without notice.
#[doc(hidden)]
pub mod __private {
    // Re-export nexus_slab items needed by the macro
    pub use nexus_slab::{Key, SlotCell, VTable, create_allocator};
}
