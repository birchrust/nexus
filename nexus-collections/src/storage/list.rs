//! Specialized storage types for [`List`](crate::List).
//!
//! These storage types hide the internal [`ListNode`] structure from users,
//! providing a cleaner API where you only specify the element type.
//!
//! # Storage Types
//!
//! | Type | Capacity | Use Case |
//! |------|----------|----------|
//! | [`ListStorage<T>`] | Fixed | Known max size, no allocation after init |
//! | [`GrowableListStorage<T>`] | Grows | Unknown size, may allocate on insert |
//!
//! # Example
//!
//! ```
//! use nexus_collections::{List, ListStorage};
//!
//! let mut storage: ListStorage<u64> = ListStorage::with_capacity(100);
//! let mut list: List<u64, ListStorage<u64>> = List::new();
//!
//! let key = list.try_push_back(&mut storage, 42).unwrap();
//! assert_eq!(list.get(&storage, key), Some(&42));
//! ```

// SlabOps uses interior mutability (&self for mutations), but &mut self is
// semantically correct for our API - we are mutating the storage.
#![allow(clippy::needless_pass_by_ref_mut)]

use crate::internal::SlabOps;

use super::Full;
use nexus_slab::{BoundedSlab, Key as NexusKey, Slab};

// =============================================================================
// ListNode
// =============================================================================

/// A node in the linked list.
///
/// This wraps user data with prev/next links. Users interact with `&T` and `&mut T`
/// through the list's accessor methods; the node structure is an implementation detail.
#[derive(Debug)]
pub struct ListNode<T> {
    pub(crate) data: T,
    pub(crate) prev: NexusKey,
    pub(crate) next: NexusKey,
}

impl<T> ListNode<T> {
    /// Creates a new unlinked node.
    #[inline]
    pub(crate) fn new(data: T) -> Self {
        Self {
            data,
            prev: NexusKey::NONE,
            next: NexusKey::NONE,
        }
    }
}

// =============================================================================
// ListStorageOps trait
// =============================================================================

/// Operations required for list storage.
///
/// This is a sealed trait implemented by [`ListStorage`] and [`GrowableListStorage`].
/// It enables [`List`](crate::List) to work with either bounded or growable storage.
pub trait ListStorageOps<T>: list_sealed::Sealed {
    /// Returns the number of elements stored.
    fn len(&self) -> usize;

    /// Returns `true` if no elements are stored.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the key is valid.
    fn contains(&self, key: NexusKey) -> bool;

    /// Returns a reference to the node at `key`.
    fn get_node(&self, key: NexusKey) -> Option<&ListNode<T>>;

    /// Returns a mutable reference to the node at `key`.
    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut ListNode<T>>;

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &ListNode<T>;

    /// Returns a mutable reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut ListNode<T>;

    /// Removes and returns the node at `key`.
    fn remove_node(&mut self, key: NexusKey) -> Option<ListNode<T>>;

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> ListNode<T>;
}

/// Operations for bounded list storage (fallible insertion).
pub trait BoundedListStorageOps<T>: ListStorageOps<T> {
    /// Returns the total capacity.
    fn capacity(&self) -> usize;

    /// Returns `true` if storage is at capacity.
    fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }

    /// Attempts to insert a node, returning its key.
    fn try_insert_node(&mut self, node: ListNode<T>) -> Result<NexusKey, Full<T>>;
}

/// Operations for growable list storage (infallible insertion).
pub trait GrowableListStorageOps<T>: ListStorageOps<T> {
    /// Inserts a node, returning its key. May allocate.
    fn insert_node(&mut self, node: ListNode<T>) -> NexusKey;
}

mod list_sealed {
    use super::{GrowableListStorage, ListStorage};

    pub trait Sealed {}
    impl<T> Sealed for ListStorage<T> {}
    impl<T> Sealed for GrowableListStorage<T> {}
}

// =============================================================================
// ListStorage - Bounded
// =============================================================================

/// Fixed-capacity storage for [`List`](crate::List).
///
/// Backed by [`BoundedSlab`], this storage type has a fixed capacity set at
/// creation time. Insertions fail with [`Full`] when capacity is reached.
///
/// # Example
///
/// ```
/// use nexus_collections::{List, ListStorage};
///
/// let mut storage: ListStorage<u64> = ListStorage::with_capacity(100);
/// let mut list: List<u64, ListStorage<u64>> = List::new();
///
/// for i in 0..100 {
///     list.try_push_back(&mut storage, i).unwrap();
/// }
/// assert!(list.try_push_back(&mut storage, 100).is_err());
/// ```
#[derive(Debug)]
pub struct ListStorage<T> {
    inner: BoundedSlab<ListNode<T>>,
}

impl<T> ListStorage<T> {
    /// Creates storage with the specified capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is 0.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: BoundedSlab::with_capacity(capacity),
        }
    }
}

impl<T> ListStorageOps<T> for ListStorage<T> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.slab_len()
    }

    #[inline]
    fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    #[inline]
    fn get_node(&self, key: NexusKey) -> Option<&ListNode<T>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    #[inline]
    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut ListNode<T>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &ListNode<T> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut ListNode<T> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    #[inline]
    fn remove_node(&mut self, key: NexusKey) -> Option<ListNode<T>> {
        self.inner.slab_try_remove(key)
    }

    #[inline]
    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> ListNode<T> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

impl<T> BoundedListStorageOps<T> for ListStorage<T> {
    #[inline]
    fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    #[inline]
    fn try_insert_node(&mut self, node: ListNode<T>) -> Result<NexusKey, Full<T>> {
        self.inner
            .insert(node)
            .map(|entry| entry.key())
            .map_err(|e| Full(e.0.data))
    }
}

// =============================================================================
// GrowableListStorage - Unbounded
// =============================================================================

/// Growable storage for [`List`](crate::List).
///
/// Backed by [`Slab`], this storage type grows as needed. Insertions always
/// succeed but may allocate.
///
/// # Example
///
/// ```
/// use nexus_collections::{List, GrowableListStorage};
///
/// let mut storage: GrowableListStorage<u64> = GrowableListStorage::new();
/// let mut list: List<u64, GrowableListStorage<u64>> = List::new();
///
/// // Can grow indefinitely
/// for i in 0..10_000 {
///     list.push_back(&mut storage, i);
/// }
/// ```
#[derive(Debug)]
pub struct GrowableListStorage<T> {
    inner: Slab<ListNode<T>>,
}

impl<T> GrowableListStorage<T> {
    /// Creates empty growable storage.
    #[inline]
    pub fn new() -> Self {
        Self { inner: Slab::new() }
    }

    /// Creates growable storage with pre-allocated capacity.
    ///
    /// The storage will grow beyond this if needed.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Slab::with_capacity(capacity),
        }
    }
}

impl<T> ListStorageOps<T> for GrowableListStorage<T> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.slab_len()
    }

    #[inline]
    fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    #[inline]
    fn get_node(&self, key: NexusKey) -> Option<&ListNode<T>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    #[inline]
    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut ListNode<T>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &ListNode<T> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut ListNode<T> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    #[inline]
    fn remove_node(&mut self, key: NexusKey) -> Option<ListNode<T>> {
        self.inner.slab_try_remove(key)
    }

    #[inline]
    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> ListNode<T> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

impl<T> GrowableListStorageOps<T> for GrowableListStorage<T> {
    #[inline]
    fn insert_node(&mut self, node: ListNode<T>) -> NexusKey {
        self.inner.insert(node).key()
    }
}

impl<T> Default for GrowableListStorage<T> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_storage_basic() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(16);
        assert!(storage.is_empty());
        assert_eq!(storage.capacity(), 16);

        let key = storage.try_insert_node(ListNode::new(42)).unwrap();
        assert_eq!(storage.len(), 1);
        assert!(storage.contains(key));

        let node = storage.get_node(key).unwrap();
        assert_eq!(node.data, 42);

        let removed = storage.remove_node(key).unwrap();
        assert_eq!(removed.data, 42);
        assert!(storage.is_empty());
    }

    #[test]
    fn list_storage_full() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(2);

        storage.try_insert_node(ListNode::new(1)).unwrap();
        storage.try_insert_node(ListNode::new(2)).unwrap();
        assert!(storage.is_full());

        let err = storage.try_insert_node(ListNode::new(3));
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().into_inner(), 3);
    }

    #[test]
    fn growable_list_storage_basic() {
        let mut storage: GrowableListStorage<u64> = GrowableListStorage::new();
        assert!(storage.is_empty());

        let key = storage.insert_node(ListNode::new(42));
        assert_eq!(storage.len(), 1);
        assert!(storage.contains(key));

        let node = storage.get_node(key).unwrap();
        assert_eq!(node.data, 42);

        let removed = storage.remove_node(key).unwrap();
        assert_eq!(removed.data, 42);
        assert!(storage.is_empty());
    }

    #[test]
    fn growable_list_storage_grows() {
        let mut storage: GrowableListStorage<u64> = GrowableListStorage::new();

        // Insert many elements - should grow as needed
        let mut keys = Vec::new();
        for i in 0..1000 {
            keys.push(storage.insert_node(ListNode::new(i)));
        }

        assert_eq!(storage.len(), 1000);

        // Verify all values
        for (i, key) in keys.iter().enumerate() {
            let node = storage.get_node(*key).unwrap();
            assert_eq!(node.data, i as u64);
        }
    }
}
