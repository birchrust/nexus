//! Macros for creating typed collection allocators.
//!
//! The `list_allocator!` macro generates a module with TLS-based storage
//! for list nodes, providing a clean API without visible generics.

/// Creates a TLS-based list allocator module for a specific type.
///
/// # Syntax
///
/// ```ignore
/// list_allocator!(module_name, Type);
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
/// - `List` - Wrapper type for creating lists
/// - `ListSlot` - Handle to a linked node (type alias)
/// - `DetachedNode` - Handle to an unlinked node (type alias)
///
/// # Example
///
/// ```ignore
/// use nexus_collections::list_allocator;
///
/// #[derive(Debug)]
/// struct Order { id: u64, price: f64 }
///
/// // Create the allocator module
/// list_allocator!(orders, Order);
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
macro_rules! list_allocator {
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

            // Re-export types for internal use
            #[doc(hidden)]
            pub use $crate::__private::Key as __Key;
            #[doc(hidden)]
            pub use $crate::__private::VTable as __VTable;
            #[doc(hidden)]
            pub use $crate::list::Node;

            // Create the underlying slab allocator for Node<T>
            // Note: The slab module will use super::__T to access the type
            $crate::__private::create_allocator!(__slab, $crate::list::Node<super::__T>);

            // Re-export initialization and lifecycle functions
            pub use __slab::{capacity, init, is_initialized, len, shutdown};

            // =================================================================
            // Type aliases (no generics visible to users)
            // =================================================================

            /// Handle to a linked node in the list.
            pub type ListSlot = $crate::list::ListSlot<__T>;

            /// Handle to an unlinked node (not in any list).
            pub type DetachedNode = $crate::list::DetachedListNode<__T>;

            /// Transitionary guard for pop operations.
            pub type Detached = $crate::list::Detached<__T>;

            /// Cursor for list traversal.
            pub type Cursor<'a> = $crate::list::Cursor<'a, __T>;

            /// Guard for cursor position.
            pub type CursorGuard<'cursor, 'list> = $crate::list::CursorGuard<'cursor, 'list, __T>;

            // =================================================================
            // List wrapper (newtype for orphan rule compliance)
            // =================================================================

            /// A doubly-linked list for this type.
            ///
            /// This is a thin wrapper around `nexus_collections::List<T>` that
            /// provides module-local construction via `new()`.
            #[repr(transparent)]
            pub struct List {
                inner: $crate::list::List<__T>,
            }

            impl List {
                /// Creates a new empty list.
                ///
                /// # Panics
                ///
                /// Panics in debug builds if the allocator is not initialized.
                #[inline]
                pub fn new() -> Self {
                    let vtable = __slab::vtable_ptr();
                    // SAFETY: vtable_ptr returns a valid pointer when initialized
                    Self {
                        inner: unsafe { $crate::list::List::with_vtable(vtable) },
                    }
                }
            }

            impl Default for List {
                fn default() -> Self {
                    Self::new()
                }
            }

            impl core::ops::Deref for List {
                type Target = $crate::list::List<__T>;

                #[inline]
                fn deref(&self) -> &Self::Target {
                    &self.inner
                }
            }

            impl core::ops::DerefMut for List {
                #[inline]
                fn deref_mut(&mut self) -> &mut Self::Target {
                    &mut self.inner
                }
            }

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
                let vtable = __slab::vtable_ptr();
                let key = slot.leak();
                // SAFETY: We just created this node, it's valid and detached
                Some(unsafe { $crate::list::DetachedListNode::from_key_vtable(key, vtable) })
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
