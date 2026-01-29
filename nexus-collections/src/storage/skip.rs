//! Specialized storage types for [`SkipList`](crate::SkipList).
//!
//! These storage types hide the internal [`SkipNode`] structure from users,
//! providing a cleaner API where you only specify the key and value types.
//!
//! # Storage Types
//!
//! | Type | Capacity | Use Case |
//! |------|----------|----------|
//! | [`SkipStorage<K, V>`] | Fixed | Known max size, no allocation after init |
//! | [`GrowableSkipStorage<K, V>`] | Grows | Unknown size, may allocate on insert |
//!
//! # Example
//!
//! ```
//! use nexus_collections::{SkipList, SkipStorage};
//! use rand::rngs::SmallRng;
//! use rand::SeedableRng;
//!
//! let mut storage: SkipStorage<u64, String, 16> = SkipStorage::with_capacity(100);
//! let rng = SmallRng::seed_from_u64(12345);
//! let mut skiplist: SkipList<u64, String, SkipStorage<u64, String, 16>, SmallRng, 16> =
//!     SkipList::new(rng);
//!
//! skiplist.try_insert(&mut storage, 42, "hello".into()).unwrap();
//! assert_eq!(skiplist.get(&storage, &42), Some(&"hello".into()));
//! ```

// TODO: Remove these allows once SkipList is updated to use the new storage types directly
#![allow(dead_code)]
// SlabOps uses interior mutability (&self for mutations), but &mut self is
// semantically correct for our API - we are mutating the storage.
#![allow(clippy::needless_pass_by_ref_mut)]

use crate::internal::SlabOps;

use super::Full;
use nexus_slab::{BoundedSlab, Key as NexusKey, Slab};

// =============================================================================
// SkipNode
// =============================================================================

/// A node in the skip list containing key, value, and forward pointers.
///
/// Forward pointers at each level point to the next node at that level.
/// Nodes with higher `level` values participate in more express lanes,
/// allowing O(log n) traversal.
#[derive(Debug, Clone)]
pub struct SkipNode<K, V, const MAX_LEVEL: usize> {
    /// The key used for ordering.
    pub key: K,
    /// The value associated with this key.
    pub value: V,
    /// Forward pointers at each level. `forward[i]` points to the next node at level i.
    pub(crate) forward: [NexusKey; MAX_LEVEL],
    /// The level of this node (0-indexed). Node participates in levels 0..=level.
    pub(crate) level: u8,
}

impl<K, V, const MAX_LEVEL: usize> SkipNode<K, V, MAX_LEVEL> {
    /// Creates a new node with the given key, value, and level.
    #[inline]
    pub(crate) fn new(key: K, value: V, level: u8) -> Self {
        Self {
            key,
            value,
            forward: [NexusKey::NONE; MAX_LEVEL],
            level,
        }
    }
}

// =============================================================================
// SkipStorageOps trait
// =============================================================================

/// Operations required for skip list storage.
///
/// This is a sealed trait implemented by [`SkipStorage`] and [`GrowableSkipStorage`].
/// It enables [`SkipList`](crate::SkipList) to work with either bounded or growable storage.
pub trait SkipStorageOps<K, V, const MAX_LEVEL: usize>:
    skip_sealed::Sealed<K, V, MAX_LEVEL>
{
    /// Returns the number of elements stored.
    fn len(&self) -> usize;

    /// Returns `true` if no elements are stored.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the key is valid.
    fn contains(&self, key: NexusKey) -> bool;

    /// Returns a reference to the node at `key`.
    fn get_node(&self, key: NexusKey) -> Option<&SkipNode<K, V, MAX_LEVEL>>;

    /// Returns a mutable reference to the node at `key`.
    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut SkipNode<K, V, MAX_LEVEL>>;

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &SkipNode<K, V, MAX_LEVEL>;

    /// Returns a mutable reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut SkipNode<K, V, MAX_LEVEL>;

    /// Removes and returns the node at `key`.
    fn remove_node(&mut self, key: NexusKey) -> Option<SkipNode<K, V, MAX_LEVEL>>;

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> SkipNode<K, V, MAX_LEVEL>;
}

/// Operations for bounded skip storage (fallible insertion).
pub trait BoundedSkipStorageOps<K, V, const MAX_LEVEL: usize>:
    SkipStorageOps<K, V, MAX_LEVEL>
{
    /// Returns the total capacity.
    fn capacity(&self) -> usize;

    /// Returns `true` if storage is at capacity.
    fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }

    /// Attempts to insert a node, returning its key.
    fn try_insert_node(
        &mut self,
        node: SkipNode<K, V, MAX_LEVEL>,
    ) -> Result<NexusKey, Full<(K, V)>>;
}

/// Operations for growable skip storage (infallible insertion).
pub trait GrowableSkipStorageOps<K, V, const MAX_LEVEL: usize>:
    SkipStorageOps<K, V, MAX_LEVEL>
{
    /// Inserts a node, returning its key. May allocate.
    fn insert_node(&mut self, node: SkipNode<K, V, MAX_LEVEL>) -> NexusKey;
}

mod skip_sealed {
    use super::{GrowableSkipStorage, SkipStorage};

    pub trait Sealed<K, V, const MAX_LEVEL: usize> {}
    impl<K, V, const MAX_LEVEL: usize> Sealed<K, V, MAX_LEVEL> for SkipStorage<K, V, MAX_LEVEL> {}
    impl<K, V, const MAX_LEVEL: usize> Sealed<K, V, MAX_LEVEL>
        for GrowableSkipStorage<K, V, MAX_LEVEL>
    {
    }
}

// =============================================================================
// SkipStorage - Bounded
// =============================================================================

/// Fixed-capacity storage for [`SkipList`](crate::SkipList).
///
/// Backed by [`BoundedSlab`], this storage type has a fixed capacity set at
/// creation time. Insertions fail with [`Full`] when capacity is reached.
///
/// # Type Parameters
///
/// - `K`: Key type
/// - `V`: Value type
/// - `MAX_LEVEL`: Maximum skip list level (default 16, supports ~65K elements efficiently)
///
/// # Example
///
/// ```
/// use nexus_collections::{SkipList, SkipStorage};
/// use rand::rngs::SmallRng;
/// use rand::SeedableRng;
///
/// let mut storage: SkipStorage<u64, String, 16> = SkipStorage::with_capacity(100);
/// let rng = SmallRng::seed_from_u64(12345);
/// let mut skiplist: SkipList<u64, String, SkipStorage<u64, String, 16>, SmallRng, 16> =
///     SkipList::new(rng);
///
/// for i in 0..100 {
///     skiplist.try_insert(&mut storage, i, format!("value-{}", i)).unwrap();
/// }
/// ```
#[derive(Debug)]
pub struct SkipStorage<K, V, const MAX_LEVEL: usize = 16> {
    inner: BoundedSlab<SkipNode<K, V, MAX_LEVEL>>,
}

impl<K, V, const MAX_LEVEL: usize> SkipStorage<K, V, MAX_LEVEL> {
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
    pub(crate) fn try_insert(
        &mut self,
        node: SkipNode<K, V, MAX_LEVEL>,
    ) -> Result<NexusKey, Full<(K, V)>> {
        self.inner
            .insert(node)
            .map(|entry| entry.key())
            .map_err(|e| Full((e.0.key, e.0.value)))
    }

    /// Returns a reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node(&self, key: NexusKey) -> Option<&SkipNode<K, V, MAX_LEVEL>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    /// Returns a mutable reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut SkipNode<K, V, MAX_LEVEL>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn get_node_unchecked(&self, key: NexusKey) -> &SkipNode<K, V, MAX_LEVEL> {
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
    ) -> &mut SkipNode<K, V, MAX_LEVEL> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    /// Removes and returns the node at `key`.
    #[inline]
    pub(crate) fn remove_node(&mut self, key: NexusKey) -> Option<SkipNode<K, V, MAX_LEVEL>> {
        self.inner.slab_try_remove(key)
    }

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn remove_node_unchecked(
        &mut self,
        key: NexusKey,
    ) -> SkipNode<K, V, MAX_LEVEL> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

// =============================================================================
// GrowableSkipStorage - Unbounded
// =============================================================================

/// Growable storage for [`SkipList`](crate::SkipList).
///
/// Backed by [`Slab`], this storage type grows as needed. Insertions always
/// succeed but may allocate.
///
/// # Type Parameters
///
/// - `K`: Key type
/// - `V`: Value type
/// - `MAX_LEVEL`: Maximum skip list level (default 16, supports ~65K elements efficiently)
///
/// # Example
///
/// ```
/// use nexus_collections::{SkipList, GrowableSkipStorage};
/// use rand::rngs::SmallRng;
/// use rand::SeedableRng;
///
/// let mut storage: GrowableSkipStorage<u64, String, 16> = GrowableSkipStorage::new();
/// let rng = SmallRng::seed_from_u64(12345);
/// let mut skiplist: SkipList<u64, String, GrowableSkipStorage<u64, String, 16>, SmallRng, 16> =
///     SkipList::new(rng);
///
/// // Can grow indefinitely
/// for i in 0..10_000 {
///     skiplist.insert(&mut storage, i, format!("value-{}", i));
/// }
/// ```
#[derive(Debug)]
pub struct GrowableSkipStorage<K, V, const MAX_LEVEL: usize = 16> {
    inner: Slab<SkipNode<K, V, MAX_LEVEL>>,
}

impl<K, V, const MAX_LEVEL: usize> GrowableSkipStorage<K, V, MAX_LEVEL> {
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
    pub(crate) fn insert(&mut self, node: SkipNode<K, V, MAX_LEVEL>) -> NexusKey {
        self.inner.insert(node).key()
    }

    /// Returns a reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node(&self, key: NexusKey) -> Option<&SkipNode<K, V, MAX_LEVEL>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    /// Returns a mutable reference to the node at `key`.
    #[inline]
    pub(crate) fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut SkipNode<K, V, MAX_LEVEL>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn get_node_unchecked(&self, key: NexusKey) -> &SkipNode<K, V, MAX_LEVEL> {
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
    ) -> &mut SkipNode<K, V, MAX_LEVEL> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    /// Removes and returns the node at `key`.
    #[inline]
    pub(crate) fn remove_node(&mut self, key: NexusKey) -> Option<SkipNode<K, V, MAX_LEVEL>> {
        self.inner.slab_try_remove(key)
    }

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    #[inline]
    pub(crate) unsafe fn remove_node_unchecked(
        &mut self,
        key: NexusKey,
    ) -> SkipNode<K, V, MAX_LEVEL> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

impl<K, V, const MAX_LEVEL: usize> Default for GrowableSkipStorage<K, V, MAX_LEVEL> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// SkipStorageOps trait implementations
// =============================================================================

impl<K, V, const MAX_LEVEL: usize> SkipStorageOps<K, V, MAX_LEVEL>
    for SkipStorage<K, V, MAX_LEVEL>
{
    #[inline]
    fn len(&self) -> usize {
        self.inner.slab_len()
    }

    #[inline]
    fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    #[inline]
    fn get_node(&self, key: NexusKey) -> Option<&SkipNode<K, V, MAX_LEVEL>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    #[inline]
    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut SkipNode<K, V, MAX_LEVEL>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &SkipNode<K, V, MAX_LEVEL> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut SkipNode<K, V, MAX_LEVEL> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    #[inline]
    fn remove_node(&mut self, key: NexusKey) -> Option<SkipNode<K, V, MAX_LEVEL>> {
        self.inner.slab_try_remove(key)
    }

    #[inline]
    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> SkipNode<K, V, MAX_LEVEL> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

impl<K, V, const MAX_LEVEL: usize> BoundedSkipStorageOps<K, V, MAX_LEVEL>
    for SkipStorage<K, V, MAX_LEVEL>
{
    #[inline]
    fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    #[inline]
    fn try_insert_node(
        &mut self,
        node: SkipNode<K, V, MAX_LEVEL>,
    ) -> Result<NexusKey, Full<(K, V)>> {
        self.inner
            .insert(node)
            .map(|entry| entry.key())
            .map_err(|e| Full((e.0.key, e.0.value)))
    }
}

impl<K, V, const MAX_LEVEL: usize> SkipStorageOps<K, V, MAX_LEVEL>
    for GrowableSkipStorage<K, V, MAX_LEVEL>
{
    #[inline]
    fn len(&self) -> usize {
        self.inner.slab_len()
    }

    #[inline]
    fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    #[inline]
    fn get_node(&self, key: NexusKey) -> Option<&SkipNode<K, V, MAX_LEVEL>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    #[inline]
    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut SkipNode<K, V, MAX_LEVEL>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &SkipNode<K, V, MAX_LEVEL> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    #[inline]
    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut SkipNode<K, V, MAX_LEVEL> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    #[inline]
    fn remove_node(&mut self, key: NexusKey) -> Option<SkipNode<K, V, MAX_LEVEL>> {
        self.inner.slab_try_remove(key)
    }

    #[inline]
    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> SkipNode<K, V, MAX_LEVEL> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }
}

impl<K, V, const MAX_LEVEL: usize> GrowableSkipStorageOps<K, V, MAX_LEVEL>
    for GrowableSkipStorage<K, V, MAX_LEVEL>
{
    #[inline]
    fn insert_node(&mut self, node: SkipNode<K, V, MAX_LEVEL>) -> NexusKey {
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
    fn skip_storage_basic() {
        let mut storage: SkipStorage<u64, String, 8> = SkipStorage::with_capacity(16);
        assert!(storage.is_empty());
        assert_eq!(storage.capacity(), 16);

        let key = storage
            .try_insert(SkipNode::new(42, "hello".to_string(), 3))
            .unwrap();
        assert_eq!(storage.len(), 1);
        assert!(storage.contains(key));

        let node = storage.get_node(key).unwrap();
        assert_eq!(node.key, 42);
        assert_eq!(node.value, "hello");

        let removed = storage.remove_node(key).unwrap();
        assert_eq!(removed.key, 42);
        assert!(storage.is_empty());
    }

    #[test]
    fn skip_storage_full() {
        let mut storage: SkipStorage<u64, u64, 4> = SkipStorage::with_capacity(2);

        storage.try_insert(SkipNode::new(1, 10, 0)).unwrap();
        storage.try_insert(SkipNode::new(2, 20, 0)).unwrap();
        assert!(storage.is_full());

        let err = storage.try_insert(SkipNode::new(3, 30, 0));
        assert!(err.is_err());
        let (k, v) = err.unwrap_err().into_inner();
        assert_eq!(k, 3);
        assert_eq!(v, 30);
    }

    #[test]
    fn growable_skip_storage_basic() {
        let mut storage: GrowableSkipStorage<u64, String, 8> = GrowableSkipStorage::new();
        assert!(storage.is_empty());

        let key = storage.insert(SkipNode::new(42, "hello".to_string(), 3));
        assert_eq!(storage.len(), 1);
        assert!(storage.contains(key));

        let node = storage.get_node(key).unwrap();
        assert_eq!(node.key, 42);
        assert_eq!(node.value, "hello");

        let removed = storage.remove_node(key).unwrap();
        assert_eq!(removed.key, 42);
        assert!(storage.is_empty());
    }

    #[test]
    fn growable_skip_storage_grows() {
        let mut storage: GrowableSkipStorage<u64, u64, 4> = GrowableSkipStorage::new();

        // Insert many elements - should grow as needed
        let mut keys = Vec::new();
        for i in 0..1000 {
            keys.push(storage.insert(SkipNode::new(i, i * 10, 0)));
        }

        assert_eq!(storage.len(), 1000);

        // Verify all values
        for (i, key) in keys.iter().enumerate() {
            let node = storage.get_node(*key).unwrap();
            assert_eq!(node.key, i as u64);
            assert_eq!(node.value, (i * 10) as u64);
        }
    }
}
