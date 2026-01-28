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
//! let mut list: List<u64, ListStorage<u64>, _> = List::new();
//!
//! let key = list.try_push_back(&mut storage, 42).unwrap();
//! assert_eq!(list.get(&storage, key), Some(&42));
//! ```

// TODO: Remove these allows once List is updated to use the new storage types directly
#![allow(dead_code)]
// SlabOps uses interior mutability (&self for mutations), but &mut self is
// semantically correct for our API - we are mutating the storage.
#![allow(clippy::needless_pass_by_ref_mut)]

use crate::internal::SlabOps;
use crate::Key;

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
pub struct ListNode<T, K> {
    pub(crate) data: T,
    pub(crate) prev: K,
    pub(crate) next: K,
}

impl<T, K: Key> ListNode<T, K> {
    /// Creates a new unlinked node.
    #[inline]
    pub(crate) fn new(data: T) -> Self {
        Self {
            data,
            prev: K::NONE,
            next: K::NONE,
        }
    }
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
/// let mut list: List<u64, ListStorage<u64>, _> = List::new();
///
/// for i in 0..100 {
///     list.try_push_back(&mut storage, i).unwrap();
/// }
/// assert!(list.try_push_back(&mut storage, 100).is_err());
/// ```
#[derive(Debug)]
pub struct ListStorage<T> {
    inner: BoundedSlab<ListNode<T, NexusKey>>,
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

    /// Returns the total capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Returns the number of elements stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.slab_len()
    }

    /// Returns `true` if no elements are stored.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.slab_is_empty()
    }

    /// Returns `true` if storage is at capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }

    /// Returns `true` if the key is valid.
    #[inline]
    pub fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    /// Attempts to insert a node, returning its key.
    #[inline]
    pub(crate) fn try_insert(&mut self, node: ListNode<T, NexusKey>) -> Result<NexusKey, Full<T>> {
        self.inner
            .insert(node)
            .map(|entry| entry.key())
            .map_err(|e| Full(e.0.data))
    }

    /// Returns a reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node(&self, key: NexusKey) -> Option<&ListNode<T, NexusKey>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    /// Returns a mutable reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut ListNode<T, NexusKey>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn get_node_unchecked(&self, key: NexusKey) -> &ListNode<T, NexusKey> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    /// Returns a mutable reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn get_node_unchecked_mut(
        &mut self,
        key: NexusKey,
    ) -> &mut ListNode<T, NexusKey> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    /// Removes and returns the node at `key`.
    #[inline]
    pub(crate) fn remove_node(&mut self, key: NexusKey) -> Option<ListNode<T, NexusKey>> {
        self.inner.slab_try_remove(key)
    }

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> ListNode<T, NexusKey> {
        unsafe { self.inner.slab_remove_unchecked(key) }
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
/// let mut list: List<u64, GrowableListStorage<u64>, _> = List::new();
///
/// // Can grow indefinitely
/// for i in 0..10_000 {
///     list.push_back(&mut storage, i);
/// }
/// ```
#[derive(Debug)]
pub struct GrowableListStorage<T> {
    inner: Slab<ListNode<T, NexusKey>>,
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

    /// Returns the number of elements stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.slab_len()
    }

    /// Returns `true` if no elements are stored.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.slab_is_empty()
    }

    /// Returns `true` if the key is valid.
    #[inline]
    pub fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    /// Inserts a node, returning its key.
    #[inline]
    pub(crate) fn insert(&mut self, node: ListNode<T, NexusKey>) -> NexusKey {
        self.inner.insert(node).key()
    }

    /// Returns a reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node(&self, key: NexusKey) -> Option<&ListNode<T, NexusKey>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    /// Returns a mutable reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut ListNode<T, NexusKey>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn get_node_unchecked(&self, key: NexusKey) -> &ListNode<T, NexusKey> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    /// Returns a mutable reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn get_node_unchecked_mut(
        &mut self,
        key: NexusKey,
    ) -> &mut ListNode<T, NexusKey> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    /// Removes and returns the node at `key`.
    #[inline]
    pub(crate) fn remove_node(&mut self, key: NexusKey) -> Option<ListNode<T, NexusKey>> {
        self.inner.slab_try_remove(key)
    }

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> ListNode<T, NexusKey> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

impl<T> Default for GrowableListStorage<T> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Storage trait implementations for legacy compatibility
// =============================================================================

use super::{BoundedStorage, Storage, UnboundedStorage};

impl<T> Storage<ListNode<T, NexusKey>> for ListStorage<T> {
    type Key = NexusKey;

    #[inline]
    fn remove(&mut self, key: Self::Key) -> Option<ListNode<T, NexusKey>> {
        self.remove_node(key)
    }

    #[inline]
    fn get(&self, key: Self::Key) -> Option<&ListNode<T, NexusKey>> {
        self.get_node(key)
    }

    #[inline]
    fn get_mut(&mut self, key: Self::Key) -> Option<&mut ListNode<T, NexusKey>> {
        self.get_node_mut(key)
    }

    #[inline]
    fn len(&self) -> usize {
        self.len()
    }

    #[inline]
    unsafe fn get_unchecked(&self, key: Self::Key) -> &ListNode<T, NexusKey> {
        unsafe { self.get_node_unchecked(key) }
    }

    #[inline]
    unsafe fn get_unchecked_mut(&mut self, key: Self::Key) -> &mut ListNode<T, NexusKey> {
        unsafe { self.get_node_unchecked_mut(key) }
    }

    #[inline]
    unsafe fn remove_unchecked(&mut self, key: Self::Key) -> ListNode<T, NexusKey> {
        unsafe { self.remove_node_unchecked(key) }
    }
}

impl<T> BoundedStorage<ListNode<T, NexusKey>> for ListStorage<T> {
    #[inline]
    fn try_insert(&mut self, node: ListNode<T, NexusKey>) -> Result<Self::Key, Full<ListNode<T, NexusKey>>> {
        self.inner
            .insert(node)
            .map(|entry| entry.key())
            .map_err(|e| Full(e.0))
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.capacity()
    }
}

impl<T> Storage<ListNode<T, NexusKey>> for GrowableListStorage<T> {
    type Key = NexusKey;

    #[inline]
    fn remove(&mut self, key: Self::Key) -> Option<ListNode<T, NexusKey>> {
        self.remove_node(key)
    }

    #[inline]
    fn get(&self, key: Self::Key) -> Option<&ListNode<T, NexusKey>> {
        self.get_node(key)
    }

    #[inline]
    fn get_mut(&mut self, key: Self::Key) -> Option<&mut ListNode<T, NexusKey>> {
        self.get_node_mut(key)
    }

    #[inline]
    fn len(&self) -> usize {
        GrowableListStorage::len(self)
    }

    #[inline]
    unsafe fn get_unchecked(&self, key: Self::Key) -> &ListNode<T, NexusKey> {
        unsafe { self.get_node_unchecked(key) }
    }

    #[inline]
    unsafe fn get_unchecked_mut(&mut self, key: Self::Key) -> &mut ListNode<T, NexusKey> {
        unsafe { self.get_node_unchecked_mut(key) }
    }

    #[inline]
    unsafe fn remove_unchecked(&mut self, key: Self::Key) -> ListNode<T, NexusKey> {
        unsafe { self.remove_node_unchecked(key) }
    }
}

impl<T> UnboundedStorage<ListNode<T, NexusKey>> for GrowableListStorage<T> {
    #[inline]
    fn insert(&mut self, node: ListNode<T, NexusKey>) -> Self::Key {
        GrowableListStorage::insert(self, node)
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

        let key = storage.try_insert(ListNode::new(42)).unwrap();
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

        storage.try_insert(ListNode::new(1)).unwrap();
        storage.try_insert(ListNode::new(2)).unwrap();
        assert!(storage.is_full());

        let err = storage.try_insert(ListNode::new(3));
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().into_inner(), 3);
    }

    #[test]
    fn growable_list_storage_basic() {
        let mut storage: GrowableListStorage<u64> = GrowableListStorage::new();
        assert!(storage.is_empty());

        let key = storage.insert(ListNode::new(42));
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
            keys.push(storage.insert(ListNode::new(i)));
        }

        assert_eq!(storage.len(), 1000);

        // Verify all values
        for (i, key) in keys.iter().enumerate() {
            let node = storage.get_node(*key).unwrap();
            assert_eq!(node.data, i as u64);
        }
    }
}
