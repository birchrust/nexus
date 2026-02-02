//! Macros for creating typed collection allocators.
//!
//! The `create_list!` macro generates a module with TLS-based storage
//! for list nodes, providing a clean API without visible generics.

/// Creates a TLS-based list allocator module for a specific type.
///
/// # Syntax
///
/// ```ignore
/// create_list!(module_name, Type);
/// ```
///
/// The type must be visible from the scope where the macro is invoked.
/// For types defined in the same module, just use the type name.
/// For types from other modules, use a path (e.g., `crate::Order`).
///
/// # Generated API
///
/// The macro generates a module with:
///
/// - `init() -> UnconfiguredBuilder` - Start configuring the allocator
/// - `shutdown()` - Shutdown the allocator
/// - `is_initialized() -> bool` - Check if initialized
/// - `len() -> usize` - Number of nodes in the slab
/// - `capacity() -> usize` - Total capacity
/// - `create_node(value: T) -> Option<DetachedNode>` - Create a detached node
/// - `List` - The list type (type alias, no generics visible)
/// - `ListSlot` - Handle to a linked node (type alias)
/// - `DetachedNode` - Handle to an unlinked node (type alias)
///
/// # Example
///
/// ```ignore
/// use nexus_collections::create_list;
///
/// #[derive(Debug)]
/// struct Order { id: u64, price: f64 }
///
/// // Create the allocator module
/// create_list!(orders, Order);
///
/// fn main() {
///     // Initialize at startup
///     orders::init().bounded(1000).build();
///
///     // Create and use the list
///     let mut list = orders::List::new();
///
///     if let Some(detached) = orders::create_node(Order { id: 1, price: 100.0 }) {
///         let slot = list.link_back(detached);
///         let price = list.read(&slot, |o| o.price);
///         println!("Price: {}", price);
///     }
/// }
/// ```
#[macro_export]
macro_rules! create_list {
    ($name:ident, $T:ty) => {
        /// TLS-based list storage for the specified type.
        #[allow(dead_code)]
        pub mod $name {
            // Bring parent scope items into this module so $T resolves
            #[allow(unused_imports)]
            use super::*;

            // Re-export the user type for internal use.
            // We use a public type alias so it's accessible from nested modules.
            pub type __T = $T;

            // Re-export Node for internal use
            #[doc(hidden)]
            pub use $crate::__private::Key as __Key;
            #[doc(hidden)]
            pub use $crate::list::Node;

            // Create the underlying slab allocator for Node<T>
            // Note: The slab module will use super::__T to access the type
            $crate::__private::create_allocator!(__slab, $crate::list::Node<super::__T>);

            // Re-export initialization and lifecycle functions
            pub use __slab::{capacity, init, is_initialized, len, shutdown};

            // =================================================================
            // Storage marker type
            // =================================================================

            /// Marker type that implements ListStorage for this allocator.
            #[derive(Copy, Clone)]
            pub struct Storage;

            impl $crate::internal::ListStorage<__T> for Storage {
                #[inline]
                fn contains_key(key: __Key) -> bool {
                    __slab::contains_key(key)
                }

                #[inline]
                unsafe fn get(key: __Key) -> Option<&'static Node<__T>> {
                    unsafe { __slab::get(key) }
                }

                #[inline]
                unsafe fn get_mut(key: __Key) -> Option<&'static mut Node<__T>> {
                    unsafe { __slab::get_mut(key) }
                }

                #[inline]
                unsafe fn get_unchecked(key: __Key) -> &'static Node<__T> {
                    unsafe { __slab::get_unchecked(key) }
                }

                #[inline]
                unsafe fn get_unchecked_mut(key: __Key) -> &'static mut Node<__T> {
                    unsafe { __slab::get_unchecked_mut(key) }
                }

                #[inline]
                fn try_remove(key: __Key) -> Option<Node<__T>> {
                    unsafe { __slab::try_remove_by_key(key) }
                }

                #[inline]
                unsafe fn remove_unchecked(key: __Key) -> Node<__T> {
                    unsafe { __slab::remove_by_key(key) }
                }
            }

            // =================================================================
            // Type aliases (hide generics from users)
            // =================================================================

            /// A doubly-linked list for this type.
            pub type List = $crate::list::List<__T, Storage>;

            /// Handle to a linked node in the list.
            pub type ListSlot = $crate::list::ListSlot<__T, Storage>;

            /// Handle to an unlinked node (not in any list).
            pub type DetachedNode = $crate::list::DetachedListNode<__T, Storage>;

            /// Transitionary guard for pop operations.
            pub type Detached<'a> = $crate::list::Detached<'a, __T, Storage>;

            // =================================================================
            // Node creation
            // =================================================================

            /// Creates a new detached list node.
            ///
            /// The node is allocated in the slab but not linked to any list.
            /// Returns `None` if the slab is full (bounded) or allocation fails.
            #[inline]
            pub fn create_node(data: __T) -> Option<DetachedNode> {
                let slot = __slab::try_insert(Node::detached(data))?;
                let key = slot.leak();
                // SAFETY: We just created this node, it's valid and detached
                Some(unsafe { $crate::list::DetachedListNode::from_key(key) })
            }

            /// Creates a new detached list node, panicking if full.
            ///
            /// # Panics
            ///
            /// Panics if the slab is full.
            #[inline]
            pub fn create_node_or_panic(data: __T) -> DetachedNode {
                create_node(data).expect("slab is full")
            }
        }
    };
}
