//! Macros for creating typed list allocators.

/// Creates a bounded list allocator module for a specific type.
///
/// Generates a module with TLS-backed slab storage for list nodes,
/// providing a clean API without visible generics.
///
/// # Generated API
///
/// - `Allocator` — unit struct, call `Allocator::builder().capacity(N).build()`
/// - `Builder` — configuration builder
/// - `Handle` — `RcSlot<ListNode<T>, Allocator>` (strong handle)
/// - `WeakHandle` — `WeakSlot<ListNode<T>, Allocator>` (weak handle)
/// - `List` — `list::List<T, Allocator>`
/// - `create_node(value) -> Result<Handle, Full<T>>` — allocate a detached node
/// - `create_node_or_panic(value) -> Handle` — allocate or panic
///
/// # Example
///
/// ```ignore
/// use nexus_collections::list_allocator;
///
/// struct Order { id: u64, price: f64 }
///
/// list_allocator!(orders, Order);
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
    ($name:ident, $T:ty) => {
        #[allow(dead_code)]
        pub mod $name {
            #[allow(unused_imports)]
            use super::*;

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
        }
    };
}

/// Creates an unbounded list allocator module for a specific type.
///
/// Same as [`list_allocator!`] but the allocator grows as needed — allocation
/// never fails.
///
/// # Generated API
///
/// Same as `list_allocator!` except `create_node` always succeeds (returns
/// `Handle` directly, not `Result`).
#[macro_export]
macro_rules! unbounded_list_allocator {
    ($name:ident, $T:ty) => {
        #[allow(dead_code)]
        pub mod $name {
            #[allow(unused_imports)]
            use super::*;

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
        }
    };
}
