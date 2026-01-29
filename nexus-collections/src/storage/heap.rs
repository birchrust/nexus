//! Specialized storage types for [`Heap`](crate::Heap).
//!
//! These storage types hide the internal [`HeapNode`] structure from users,
//! providing a cleaner API where you only specify the element type.
//!
//! # Storage Types
//!
//! | Type | Capacity | Use Case |
//! |------|----------|----------|
//! | [`HeapStorage<T>`] | Fixed | Known max size, no allocation after init |
//! | [`GrowableHeapStorage<T>`] | Grows | Unknown size, may allocate on insert |
//!
//! # Example
//!
//! ```
//! use nexus_collections::{Heap, HeapStorage};
//!
//! let mut storage: HeapStorage<u64> = HeapStorage::with_capacity(100);
//! let mut heap: Heap<u64, HeapStorage<u64>> = Heap::new();
//!
//! let key = heap.try_push(&mut storage, 42).unwrap();
//! assert_eq!(heap.peek(&storage), Some(&42));
//! ```

// TODO: Remove these allows once Heap is updated to use the new storage types directly
#![allow(dead_code)]
// SlabOps uses interior mutability (&self for mutations), but &mut self is
// semantically correct for our API - we are mutating the storage.
#![allow(clippy::needless_pass_by_ref_mut)]

use crate::internal::SlabOps;

use super::Full;
use nexus_slab::{BoundedSlab, Key as NexusKey, Slab};

// =============================================================================
// HeapNode
// =============================================================================

pub(crate) const HEAP_POS_NONE: usize = usize::MAX;

/// A node in the heap.
///
/// This wraps user data with heap position tracking. Users interact with `&T`
/// and `&mut T` through the heap's accessor methods; the node structure is an
/// implementation detail.
#[derive(Debug)]
pub struct HeapNode<T> {
    pub(crate) data: T,
    pub(crate) heap_pos: usize, // position in indices vec
}

impl<T> HeapNode<T> {
    /// Creates a new node not yet in a heap.
    #[inline]
    pub(crate) fn new(data: T) -> Self {
        Self {
            data,
            heap_pos: HEAP_POS_NONE,
        }
    }
}

// =============================================================================
// HeapStorageOps trait
// =============================================================================

/// Operations required for heap storage.
///
/// This is a sealed trait implemented by [`HeapStorage`] and [`GrowableHeapStorage`].
/// It enables [`Heap`](crate::Heap) to work with either bounded or growable storage.
pub trait HeapStorageOps<T>: heap_sealed::Sealed {
    /// Returns the number of elements stored.
    fn len(&self) -> usize;

    /// Returns `true` if no elements are stored.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the key is valid.
    fn contains(&self, key: NexusKey) -> bool;

    /// Returns a reference to the node at `key`.
    fn get_node(&self, key: NexusKey) -> Option<&HeapNode<T>>;

    /// Returns a mutable reference to the node at `key`.
    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut HeapNode<T>>;

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &HeapNode<T>;

    /// Returns a mutable reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut HeapNode<T>;

    /// Removes and returns the node at `key`.
    fn remove_node(&mut self, key: NexusKey) -> Option<HeapNode<T>>;

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> HeapNode<T>;
}

/// Operations for bounded heap storage (fallible insertion).
pub trait BoundedHeapStorageOps<T>: HeapStorageOps<T> {
    /// Returns the total capacity.
    fn capacity(&self) -> usize;

    /// Returns `true` if storage is at capacity.
    fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }

    /// Attempts to insert a node, returning its key.
    fn try_insert_node(&mut self, node: HeapNode<T>) -> Result<NexusKey, Full<T>>;
}

/// Operations for growable heap storage (infallible insertion).
pub trait GrowableHeapStorageOps<T>: HeapStorageOps<T> {
    /// Inserts a node, returning its key. May allocate.
    fn insert_node(&mut self, node: HeapNode<T>) -> NexusKey;
}

mod heap_sealed {
    use super::{GrowableHeapStorage, HeapStorage};

    pub trait Sealed {}
    impl<T> Sealed for HeapStorage<T> {}
    impl<T> Sealed for GrowableHeapStorage<T> {}
}

// =============================================================================
// HeapStorage - Bounded
// =============================================================================

/// Fixed-capacity storage for [`Heap`](crate::Heap).
///
/// Backed by [`BoundedSlab`], this storage type has a fixed capacity set at
/// creation time. Insertions fail with [`Full`] when capacity is reached.
///
/// # Example
///
/// ```
/// use nexus_collections::{Heap, HeapStorage};
///
/// let mut storage: HeapStorage<u64> = HeapStorage::with_capacity(100);
/// let mut heap: Heap<u64, HeapStorage<u64>> = Heap::new();
///
/// for i in 0..100 {
///     heap.try_push(&mut storage, i).unwrap();
/// }
/// assert!(heap.try_push(&mut storage, 100).is_err());
/// ```
#[derive(Debug)]
pub struct HeapStorage<T> {
    inner: BoundedSlab<HeapNode<T>>,
}

impl<T> HeapStorage<T> {
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
    pub(crate) fn try_insert(&mut self, node: HeapNode<T>) -> Result<NexusKey, Full<T>> {
        self.inner
            .insert(node)
            .map(|entry| entry.key())
            .map_err(|e| Full(e.0.data))
    }

    /// Returns a reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node(&self, key: NexusKey) -> Option<&HeapNode<T>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    /// Returns a mutable reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut HeapNode<T>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn get_node_unchecked(&self, key: NexusKey) -> &HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    /// Returns a mutable reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    /// Removes and returns the node at `key`.
    #[inline]
    pub(crate) fn remove_node(&mut self, key: NexusKey) -> Option<HeapNode<T>> {
        self.inner.slab_try_remove(key)
    }

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> HeapNode<T> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

// =============================================================================
// GrowableHeapStorage - Unbounded
// =============================================================================

/// Growable storage for [`Heap`](crate::Heap).
///
/// Backed by [`Slab`], this storage type grows as needed. Insertions always
/// succeed but may allocate.
///
/// # Example
///
/// ```
/// use nexus_collections::{Heap, GrowableHeapStorage};
///
/// let mut storage: GrowableHeapStorage<u64> = GrowableHeapStorage::new();
/// let mut heap: Heap<u64, GrowableHeapStorage<u64>> = Heap::new();
///
/// // Can grow indefinitely
/// for i in 0..10_000 {
///     heap.push(&mut storage, i);
/// }
/// ```
#[derive(Debug)]
pub struct GrowableHeapStorage<T> {
    inner: Slab<HeapNode<T>>,
}

impl<T> GrowableHeapStorage<T> {
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
    pub(crate) fn insert(&mut self, node: HeapNode<T>) -> NexusKey {
        self.inner.insert(node).key()
    }

    /// Returns a reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node(&self, key: NexusKey) -> Option<&HeapNode<T>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    /// Returns a mutable reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut HeapNode<T>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn get_node_unchecked(&self, key: NexusKey) -> &HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    /// Returns a mutable reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    /// Removes and returns the node at `key`.
    #[inline]
    pub(crate) fn remove_node(&mut self, key: NexusKey) -> Option<HeapNode<T>> {
        self.inner.slab_try_remove(key)
    }

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> HeapNode<T> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

impl<T> Default for GrowableHeapStorage<T> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// HeapStorageOps trait implementations
// =============================================================================

impl<T> HeapStorageOps<T> for HeapStorage<T> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.slab_len()
    }

    #[inline]
    fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    #[inline]
    fn get_node(&self, key: NexusKey) -> Option<&HeapNode<T>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    #[inline]
    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut HeapNode<T>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    #[inline]
    fn remove_node(&mut self, key: NexusKey) -> Option<HeapNode<T>> {
        self.inner.slab_try_remove(key)
    }

    #[inline]
    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> HeapNode<T> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

impl<T> BoundedHeapStorageOps<T> for HeapStorage<T> {
    #[inline]
    fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    #[inline]
    fn try_insert_node(&mut self, node: HeapNode<T>) -> Result<NexusKey, Full<T>> {
        self.inner
            .insert(node)
            .map(|entry| entry.key())
            .map_err(|e| Full(e.0.data))
    }
}

impl<T> HeapStorageOps<T> for GrowableHeapStorage<T> {
    #[inline]
    fn len(&self) -> usize {
        self.inner.slab_len()
    }

    #[inline]
    fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    #[inline]
    fn get_node(&self, key: NexusKey) -> Option<&HeapNode<T>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    #[inline]
    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut HeapNode<T>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    #[inline]
    fn remove_node(&mut self, key: NexusKey) -> Option<HeapNode<T>> {
        self.inner.slab_try_remove(key)
    }

    #[inline]
    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> HeapNode<T> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

impl<T> GrowableHeapStorageOps<T> for GrowableHeapStorage<T> {
    #[inline]
    fn insert_node(&mut self, node: HeapNode<T>) -> NexusKey {
        self.inner.insert(node).key()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_storage_basic() {
        let mut storage: HeapStorage<u64> = HeapStorage::with_capacity(16);
        assert!(storage.is_empty());
        assert_eq!(storage.capacity(), 16);

        let key = storage.try_insert(HeapNode::new(42)).unwrap();
        assert_eq!(storage.len(), 1);
        assert!(storage.contains(key));

        let node = storage.get_node(key).unwrap();
        assert_eq!(node.data, 42);

        let removed = storage.remove_node(key).unwrap();
        assert_eq!(removed.data, 42);
        assert!(storage.is_empty());
    }

    #[test]
    fn heap_storage_full() {
        let mut storage: HeapStorage<u64> = HeapStorage::with_capacity(2);

        storage.try_insert(HeapNode::new(1)).unwrap();
        storage.try_insert(HeapNode::new(2)).unwrap();
        assert!(storage.is_full());

        let err = storage.try_insert(HeapNode::new(3));
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().into_inner(), 3);
    }

    #[test]
    fn growable_heap_storage_basic() {
        let mut storage: GrowableHeapStorage<u64> = GrowableHeapStorage::new();
        assert!(storage.is_empty());

        let key = storage.insert(HeapNode::new(42));
        assert_eq!(storage.len(), 1);
        assert!(storage.contains(key));

        let node = storage.get_node(key).unwrap();
        assert_eq!(node.data, 42);

        let removed = storage.remove_node(key).unwrap();
        assert_eq!(removed.data, 42);
        assert!(storage.is_empty());
    }

    #[test]
    fn growable_heap_storage_grows() {
        let mut storage: GrowableHeapStorage<u64> = GrowableHeapStorage::new();

        // Insert many elements - should grow as needed
        let mut keys = Vec::new();
        for i in 0..1000 {
            keys.push(storage.insert(HeapNode::new(i)));
        }

        assert_eq!(storage.len(), 1000);

        // Verify all values
        for (i, key) in keys.iter().enumerate() {
            let node = storage.get_node(*key).unwrap();
            assert_eq!(node.data, i as u64);
        }
    }
}
