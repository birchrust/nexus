//! Macros for creating typed list, heap, and tree allocators.

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
/// // Primary: collection allocates internally
/// let mut list = orders::List::new(orders::Allocator);
/// let handle = list.try_push_back(Order { id: 1, price: 100.0 }).unwrap();
/// assert_eq!(handle.exclusive().price, 100.0);
///
/// // Re-linking: move between collections
/// list.unlink(&handle);
/// other_list.link_back(&handle);
///
/// // Detached node (for deferred linking)
/// let handle = orders::create_node(Order { id: 2, price: 200.0 }).unwrap();
/// list.link_back(&handle);
/// ```
#[macro_export]
macro_rules! list_allocator {
    ($T:ty, bounded) => {
        type __T = $T;

        mod __alloc {
            $crate::bounded_rc_allocator!($crate::list::ListNode<super::__T>);
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
        pub fn create_node(value: __T) -> Result<Handle, $crate::Full<__T>> {
            Handle::try_new($crate::list::ListNode::new(value))
                .map_err(|full| $crate::Full(full.into_inner().into_data()))
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
            $crate::unbounded_rc_allocator!($crate::list::ListNode<super::__T>);
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
/// // Primary: collection allocates internally
/// let mut heap = timers::Heap::new(timers::Allocator);
/// let handle = heap.try_push(Timer { deadline: 42 }).unwrap();
///
/// // Re-linking: move between collections
/// heap.unlink(&handle);
/// other_heap.link(&handle);
///
/// // Detached node (for deferred linking)
/// let handle = timers::create_node(Timer { deadline: 42 }).unwrap();
/// heap.link(&handle);
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
            $crate::bounded_rc_allocator!($crate::heap::HeapNode<super::__T>);
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
        pub fn create_node(value: __T) -> Result<Handle, $crate::Full<__T>> {
            Handle::try_new($crate::heap::HeapNode::new(value))
                .map_err(|full| $crate::Full(full.into_inner().into_data()))
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
            $crate::unbounded_rc_allocator!($crate::heap::HeapNode<super::__T>);
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

/// Creates a B-tree allocator for a specific key-value pair.
///
/// This macro generates all the types needed to work with slab-backed
/// B-trees. Invoke it inside a module — either a file-based module or
/// an inline `mod` block.
///
/// # Usage
///
/// ```ignore
/// // Default B=8
/// mod levels {
///     nexus_collections::btree_allocator!(u64, String, bounded);
/// }
///
/// // Custom B=12
/// mod wide {
///     nexus_collections::btree_allocator!(u64, String, bounded, 12);
/// }
///
/// levels::Allocator::builder().capacity(1000).build().unwrap();
/// let mut map = levels::BTree::new(levels::Allocator);
/// map.try_insert(100, "hello".into()).unwrap();
/// ```
///
/// # Parameters
///
/// | Form | B (branching) | Max keys/node |
/// |------|---------------|---------------|
/// | `(K, V, bounded)` | 8 | 7 |
/// | `(K, V, bounded, 12)` | 12 | 11 |
///
/// B must be even and >= 4. Max keys per node is B-1.
///
/// # Variants
///
/// - `bounded` — Fixed capacity. `try_insert` returns `Result<Option<V>, Full<(K, V)>>`.
/// - `unbounded` — Grows as needed. `insert` always succeeds.
///
/// # Generated API
///
/// - `Allocator` — call `Allocator::builder()` to configure and initialize
/// - `Builder` — configuration builder
/// - `BTree` — the B-tree sorted map type
/// - `Cursor` — cursor for positional traversal
/// - `Entry` — entry enum for the entry API
#[macro_export]
macro_rules! btree_allocator {
    // 3-arg: defaults (B=8)
    ($K:ty, $V:ty, bounded) => {
        $crate::btree_allocator!($K, $V, bounded, 8);
    };
    ($K:ty, $V:ty, unbounded) => {
        $crate::btree_allocator!($K, $V, unbounded, 8);
    };
    // 4-arg: custom B
    ($K:ty, $V:ty, bounded, $B:expr) => {
        type __K = $K;
        type __V = $V;

        mod __alloc {
            $crate::bounded_allocator!(
                $crate::btree::BTreeNode<super::__K, super::__V, $B>
            );
        }

        pub use __alloc::{Allocator, Builder};

        /// The B-tree sorted map type.
        pub type BTree<C = $crate::compare::Natural> =
            $crate::btree::BTree<__K, __V, __alloc::Allocator, $B, C>;
        /// Cursor for positional traversal.
        pub type Cursor<'a, C = $crate::compare::Natural> =
            $crate::btree::Cursor<'a, __K, __V, __alloc::Allocator, $B, C>;
        /// Entry for the entry API.
        pub type Entry<'a, C = $crate::compare::Natural> =
            $crate::btree::Entry<'a, __K, __V, __alloc::Allocator, $B, C>;
    };
    ($K:ty, $V:ty, unbounded, $B:expr) => {
        type __K = $K;
        type __V = $V;

        mod __alloc {
            $crate::unbounded_allocator!(
                $crate::btree::BTreeNode<super::__K, super::__V, $B>
            );
        }

        pub use __alloc::{Allocator, Builder};

        /// The B-tree sorted map type.
        pub type BTree<C = $crate::compare::Natural> =
            $crate::btree::BTree<__K, __V, __alloc::Allocator, $B, C>;
        /// Cursor for positional traversal.
        pub type Cursor<'a, C = $crate::compare::Natural> =
            $crate::btree::Cursor<'a, __K, __V, __alloc::Allocator, $B, C>;
        /// Entry for the entry API.
        pub type Entry<'a, C = $crate::compare::Natural> =
            $crate::btree::Entry<'a, __K, __V, __alloc::Allocator, $B, C>;
    };
}

/// Creates a red-black tree allocator for a specific key-value pair.
///
/// This macro generates all the types needed to work with slab-backed
/// red-black trees. Invoke it inside a module — either a file-based
/// module or an inline `mod` block.
///
/// # Usage
///
/// ```ignore
/// mod levels {
///     nexus_collections::rbtree_allocator!(u64, String, bounded);
/// }
///
/// levels::Allocator::builder().capacity(1000).build().unwrap();
/// let mut map = levels::RbTree::new(levels::Allocator);
/// map.try_insert(100, "hello".into()).unwrap();
/// ```
///
/// # Variants
///
/// - `bounded` — Fixed capacity. `try_insert` returns `Result<Option<V>, Full<(K, V)>>`.
/// - `unbounded` — Grows as needed. `insert` always succeeds.
///
/// # Generated API
///
/// - `Allocator` — call `Allocator::builder()` to configure and initialize
/// - `Builder` — configuration builder
/// - `RbTree` — the sorted map type
/// - `Cursor` — cursor for positional traversal
/// - `Entry` — entry enum for the entry API
#[macro_export]
macro_rules! rbtree_allocator {
    ($K:ty, $V:ty, bounded) => {
        type __K = $K;
        type __V = $V;

        mod __alloc {
            $crate::bounded_allocator!($crate::rbtree::RbNode<super::__K, super::__V>);
        }

        pub use __alloc::{Allocator, Builder};

        /// The red-black tree sorted map type.
        pub type RbTree<C = $crate::compare::Natural> =
            $crate::rbtree::RbTree<__K, __V, __alloc::Allocator, C>;
        /// Cursor for positional traversal.
        pub type Cursor<'a, C = $crate::compare::Natural> =
            $crate::rbtree::Cursor<'a, __K, __V, __alloc::Allocator, C>;
        /// Entry for the entry API.
        pub type Entry<'a, C = $crate::compare::Natural> =
            $crate::rbtree::Entry<'a, __K, __V, __alloc::Allocator, C>;
    };
    ($K:ty, $V:ty, unbounded) => {
        type __K = $K;
        type __V = $V;

        mod __alloc {
            $crate::unbounded_allocator!($crate::rbtree::RbNode<super::__K, super::__V>);
        }

        pub use __alloc::{Allocator, Builder};

        /// The red-black tree sorted map type.
        pub type RbTree<C = $crate::compare::Natural> =
            $crate::rbtree::RbTree<__K, __V, __alloc::Allocator, C>;
        /// Cursor for positional traversal.
        pub type Cursor<'a, C = $crate::compare::Natural> =
            $crate::rbtree::Cursor<'a, __K, __V, __alloc::Allocator, C>;
        /// Entry for the entry API.
        pub type Entry<'a, C = $crate::compare::Natural> =
            $crate::rbtree::Entry<'a, __K, __V, __alloc::Allocator, C>;
    };
}
