//! Macros for creating typed list allocators.

/// Creates a list allocator for a specific type.
///
/// Generates TLS-backed slab storage for list nodes with a clean API.
/// Call this inside a module — the macro generates the module contents.
///
/// # Variants
///
/// - `bounded` — Fixed capacity, `create_node` returns `Result<Handle, Full<T>>`
/// - `unbounded` — Grows as needed, `create_node` always succeeds
///
/// # Generated API
///
/// - `Allocator` — unit struct, call `Allocator::builder().capacity(N).build()`
/// - `Builder` — configuration builder
/// - `Handle` — `RcSlot<ListNode<T>, Allocator>` (strong handle)
/// - `WeakHandle` — `WeakSlot<ListNode<T>, Allocator>` (weak handle)
/// - `List` — `list::List<T, Allocator>`
/// - `Cursor` — `list::Cursor<T, Allocator>`
/// - `create_node(value)` — allocate a detached node
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
