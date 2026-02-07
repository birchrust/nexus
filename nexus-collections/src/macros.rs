//! Macros for creating typed list and heap allocators.

/// Creates a list allocator for a specific type.
///
/// This macro generates all the types and functions needed to work with
/// slab-backed linked lists. Invoke it inside a module — either a
/// file-based module or an inline `mod` block.
///
/// The macro generates the allocator definition, but **you must initialize
/// the allocator at runtime** before creating any nodes. Initialization
/// configures the backing slab (capacity for bounded, chunk size for
/// unbounded) and must happen once per thread.
///
/// # Usage
///
/// **File-based module (preferred):**
///
/// ```ignore
/// // lib.rs
/// mod orders;
///
/// // orders.rs
/// use crate::Order;
/// nexus_collections::list_allocator!(Order, bounded);
/// ```
///
/// **Inline module:**
///
/// ```ignore
/// mod orders {
///     use super::*;
///     nexus_collections::list_allocator!(Order, bounded);
/// }
/// ```
///
/// **Initialization (required before first allocation):**
///
/// ```ignore
/// // bounded — set fixed capacity
/// orders::Allocator::builder().capacity(1000).build().unwrap();
///
/// // unbounded — optionally configure chunk size (default 4096)
/// orders::Allocator::builder().chunk_size(512).build().unwrap();
/// ```
///
/// # Variants
///
/// - `bounded` — Fixed capacity. `create_node` returns `Result<Handle, Full<T>>`.
/// - `unbounded` — Grows as needed. `create_node` returns `Handle` directly.
///
/// # Generated API
///
/// - `Allocator` — call `Allocator::builder()` to configure and initialize
/// - `Builder` — configuration builder (`capacity` for bounded, `chunk_size`
///   for unbounded)
/// - `Handle` — strong reference to a list node
///   (`RcSlot<ListNode<T>, Allocator>`)
/// - `WeakHandle` — weak reference (`WeakSlot<ListNode<T>, Allocator>`)
/// - `List` — the linked list type
/// - `Cursor` — cursor for positional traversal
/// - `create_node(value)` — allocate a detached node
/// - `create_node_or_panic(value)` — allocate or panic (bounded only)
///
/// # Example
///
/// ```ignore
/// use nexus_collections::list_allocator;
///
/// struct Order { id: u64, price: f64 }
///
/// mod orders {
///     use super::*;
///     list_allocator!(Order, bounded);
/// }
///
/// // Initialize the allocator before creating nodes
/// orders::Allocator::builder().capacity(1000).build().unwrap();
///
/// let mut list = orders::List::new();
/// let handle = orders::create_node(Order { id: 1, price: 100.0 }).unwrap();
/// list.link_back(&handle);
/// assert_eq!(handle.exclusive().price, 100.0);
/// ```
#[macro_export]
macro_rules! list_allocator {
    ($T:ty, bounded) => {
        type __T = $T;

        mod __alloc {
            nexus_slab::bounded_rc_allocator!($crate::list::ListNode<super::__T>);
        }

        pub use __alloc::{Allocator, Builder};

        /// Strong reference handle to a list node.
        pub type Handle = __alloc::RcSlot;
        /// Weak reference to a list node.
        pub type WeakHandle = __alloc::WeakSlot;
        /// The list type for this allocator.
        pub type List = $crate::list::List<__T, __alloc::Allocator>;
        /// Cursor for list traversal.
        pub type Cursor<'a> = $crate::list::Cursor<'a, __T, __alloc::Allocator>;

        /// Creates a new detached list node.
        ///
        /// Returns `Err(Full(value))` if the allocator is at capacity.
        #[inline]
        pub fn create_node(value: __T) -> Result<Handle, nexus_slab::Full<__T>> {
            Handle::try_new($crate::list::ListNode::new(value))
                .map_err(|full| nexus_slab::Full(full.into_inner().into_data()))
        }

        /// Creates a new detached list node, panicking if full.
        ///
        /// # Panics
        ///
        /// Panics if the allocator is at capacity.
        #[inline]
        pub fn create_node_or_panic(value: __T) -> Handle {
            create_node(value).expect("allocator is full")
        }
    };
    ($T:ty, unbounded) => {
        type __T = $T;

        mod __alloc {
            nexus_slab::unbounded_rc_allocator!($crate::list::ListNode<super::__T>);
        }

        pub use __alloc::{Allocator, Builder};

        /// Strong reference handle to a list node.
        pub type Handle = __alloc::RcSlot;
        /// Weak reference to a list node.
        pub type WeakHandle = __alloc::WeakSlot;
        /// The list type for this allocator.
        pub type List = $crate::list::List<__T, __alloc::Allocator>;
        /// Cursor for list traversal.
        pub type Cursor<'a> = $crate::list::Cursor<'a, __T, __alloc::Allocator>;

        /// Creates a new detached list node. Always succeeds.
        #[inline]
        pub fn create_node(value: __T) -> Handle {
            Handle::new($crate::list::ListNode::new(value))
        }
    };
}

/// Creates a heap allocator for a specific type.
///
/// This macro generates all the types and functions needed to work with
/// slab-backed pairing heaps. Invoke it inside a module — either a
/// file-based module or an inline `mod` block.
///
/// The macro generates the allocator definition, but **you must initialize
/// the allocator at runtime** before creating any nodes.
///
/// # Usage
///
/// ```ignore
/// mod timers {
///     use super::*;
///     nexus_collections::heap_allocator!(Timer, bounded);
/// }
///
/// timers::Allocator::builder().capacity(1000).build().unwrap();
///
/// let mut heap = timers::Heap::new();
/// let handle = timers::create_node(Timer { deadline: 42 }).unwrap();
/// heap.push(&handle);
/// ```
///
/// # Variants
///
/// - `bounded` — Fixed capacity. `create_node` returns `Result<Handle, Full<T>>`.
/// - `unbounded` — Grows as needed. `create_node` returns `Handle` directly.
///
/// # Generated API
///
/// - `Allocator` — call `Allocator::builder()` to configure and initialize
/// - `Builder` — configuration builder
/// - `Handle` — strong reference to a heap node
/// - `WeakHandle` — weak reference
/// - `Heap` — the min-heap type
/// - `create_node(value)` — allocate a detached node
/// - `create_node_or_panic(value)` — allocate or panic (bounded only)
#[macro_export]
macro_rules! heap_allocator {
    ($T:ty, bounded) => {
        type __T = $T;

        mod __alloc {
            nexus_slab::bounded_rc_allocator!($crate::heap::HeapNode<super::__T>);
        }

        pub use __alloc::{Allocator, Builder};

        /// Strong reference handle to a heap node.
        pub type Handle = __alloc::RcSlot;
        /// Weak reference to a heap node.
        pub type WeakHandle = __alloc::WeakSlot;
        /// The heap type for this allocator.
        pub type Heap = $crate::heap::Heap<__T, __alloc::Allocator>;

        /// Creates a new detached heap node.
        ///
        /// Returns `Err(Full(value))` if the allocator is at capacity.
        #[inline]
        pub fn create_node(value: __T) -> Result<Handle, nexus_slab::Full<__T>> {
            Handle::try_new($crate::heap::HeapNode::new(value))
                .map_err(|full| nexus_slab::Full(full.into_inner().into_data()))
        }

        /// Creates a new detached heap node, panicking if full.
        ///
        /// # Panics
        ///
        /// Panics if the allocator is at capacity.
        #[inline]
        pub fn create_node_or_panic(value: __T) -> Handle {
            create_node(value).expect("allocator is full")
        }
    };
    ($T:ty, unbounded) => {
        type __T = $T;

        mod __alloc {
            nexus_slab::unbounded_rc_allocator!($crate::heap::HeapNode<super::__T>);
        }

        pub use __alloc::{Allocator, Builder};

        /// Strong reference handle to a heap node.
        pub type Handle = __alloc::RcSlot;
        /// Weak reference to a heap node.
        pub type WeakHandle = __alloc::WeakSlot;
        /// The heap type for this allocator.
        pub type Heap = $crate::heap::Heap<__T, __alloc::Allocator>;

        /// Creates a new detached heap node. Always succeeds.
        #[inline]
        pub fn create_node(value: __T) -> Handle {
            Handle::new($crate::heap::HeapNode::new(value))
        }
    };
}
