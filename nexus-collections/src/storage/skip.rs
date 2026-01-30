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
//! # Entry API
//!
//! The entry API provides ergonomic access to stored values through cloneable
//! handles. See [`SkipEntry`] for details.
//!
//! Note: This is distinct from SkipList's key-based Entry API (`skiplist.entry(key)`).
//! `SkipEntry` provides storage-key access via `NexusKey`, while the SkipList
//! Entry API provides sorted-key access via `K`.
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

// Some inherent methods duplicate trait methods for direct storage usage in tests.
#![allow(dead_code)]
// SlabOps uses interior mutability (&self for mutations), but &mut self is
// semantically correct for our API - we are mutating the storage.
#![allow(clippy::needless_pass_by_ref_mut)]

use core::ops::{Deref, DerefMut};

use crate::internal::SlabOps;

use super::Full;
use nexus_slab::{BoundedSlab, CapacityError, Key as NexusKey, Slab};

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

    /// Creates an entry handle from a storage key.
    ///
    /// Returns `None` if the key is invalid.
    fn entry(&self, key: NexusKey) -> Option<SkipEntry<K, V, MAX_LEVEL>>;
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

    /// Inserts with access to the entry before the value exists.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full.
    fn insert_with<F>(&self, f: F) -> Result<SkipEntry<K, V, MAX_LEVEL>, CapacityError>
    where
        F: FnOnce(SkipEntry<K, V, MAX_LEVEL>) -> (K, V, u8);
}

/// Operations for growable skip storage (infallible insertion).
pub trait GrowableSkipStorageOps<K, V, const MAX_LEVEL: usize>:
    SkipStorageOps<K, V, MAX_LEVEL>
{
    /// Inserts a node, returning its key. May allocate.
    fn insert_node(&mut self, node: SkipNode<K, V, MAX_LEVEL>) -> NexusKey;

    /// Inserts with access to the entry before the value exists.
    fn insert_with<F>(&self, f: F) -> SkipEntry<K, V, MAX_LEVEL>
    where
        F: FnOnce(SkipEntry<K, V, MAX_LEVEL>) -> (K, V, u8);
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

    #[inline]
    fn entry(&self, key: NexusKey) -> Option<SkipEntry<K, V, MAX_LEVEL>> {
        self.inner.entry(key).map(SkipEntry::new)
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

    #[inline]
    fn insert_with<F>(&self, f: F) -> Result<SkipEntry<K, V, MAX_LEVEL>, CapacityError>
    where
        F: FnOnce(SkipEntry<K, V, MAX_LEVEL>) -> (K, V, u8),
    {
        self.inner
            .insert_with(|slab_entry| {
                let skip_entry = SkipEntry::new(slab_entry);
                let (key, value, level) = f(skip_entry);
                SkipNode::new(key, value, level)
            })
            .map(SkipEntry::new)
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

    #[inline]
    fn entry(&self, key: NexusKey) -> Option<SkipEntry<K, V, MAX_LEVEL>> {
        self.inner.entry(key).map(SkipEntry::new)
    }
}

impl<K, V, const MAX_LEVEL: usize> GrowableSkipStorageOps<K, V, MAX_LEVEL>
    for GrowableSkipStorage<K, V, MAX_LEVEL>
{
    #[inline]
    fn insert_node(&mut self, node: SkipNode<K, V, MAX_LEVEL>) -> NexusKey {
        self.inner.insert(node).key()
    }

    #[inline]
    fn insert_with<F>(&self, f: F) -> SkipEntry<K, V, MAX_LEVEL>
    where
        F: FnOnce(SkipEntry<K, V, MAX_LEVEL>) -> (K, V, u8),
    {
        SkipEntry::new(self.inner.insert_with(|slab_entry| {
            let skip_entry = SkipEntry::new(slab_entry);
            let (key, value, level) = f(skip_entry);
            SkipNode::new(key, value, level)
        }))
    }
}

// =============================================================================
// Entry Types
// =============================================================================

/// Handle to an element in skip list storage.
///
/// `SkipEntry` wraps `nexus_slab::Entry<SkipNode<K, V, MAX_LEVEL>>`, exposing
/// the key and value (but not internal node structure like forward pointers).
/// Clone to cache in multiple locations.
///
/// # Note
///
/// This is distinct from SkipList's key-based Entry API. `SkipEntry` provides
/// storage-key access via `NexusKey`, while `skiplist.entry(key)` provides
/// sorted-key access via `K`.
///
/// # Usage
///
/// `SkipEntry` is primarily used internally and in advanced patterns where
/// you need cloneable handles to storage entries. For typical skip list
/// operations, use [`SkipList`](crate::SkipList) methods directly.
///
/// The entry provides:
/// - `storage_key()`: The internal storage key (NexusKey)
/// - `value()`: RAII guard that dereferences to `&V`
/// - `value_mut()`: RAII guard that dereferences to `&mut V`
/// - The guards also provide `key()` to access the sorted key `&K`
pub struct SkipEntry<K, V, const MAX_LEVEL: usize> {
    inner: nexus_slab::Entry<SkipNode<K, V, MAX_LEVEL>>,
}

impl<K, V, const MAX_LEVEL: usize> SkipEntry<K, V, MAX_LEVEL> {
    /// Creates a new entry from a nexus-slab entry.
    #[inline]
    pub(crate) fn new(inner: nexus_slab::Entry<SkipNode<K, V, MAX_LEVEL>>) -> Self {
        Self { inner }
    }

    /// Returns the storage key.
    ///
    /// Use this for collection operations like `skiplist.remove_by_storage_key()`.
    #[inline]
    pub fn storage_key(&self) -> NexusKey {
        self.inner.key()
    }

    /// Returns `true` if the entry is still valid (not removed).
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }

    // =========================================================================
    // Safe Access (panics if invalid/borrowed)
    // =========================================================================

    /// Returns a reference to the value.
    ///
    /// # Panics
    ///
    /// Panics if the entry is invalid or already borrowed.
    #[inline]
    pub fn value(&self) -> SkipRef<K, V, MAX_LEVEL> {
        SkipRef {
            inner: self.inner.get(),
        }
    }

    /// Returns a mutable reference to the value.
    ///
    /// # Panics
    ///
    /// Panics if the entry is invalid or already borrowed.
    ///
    /// # Note
    ///
    /// Mutating the key field breaks the skip list ordering invariant.
    /// Only mutate the value field.
    #[inline]
    pub fn value_mut(&self) -> SkipRefMut<K, V, MAX_LEVEL> {
        SkipRefMut {
            inner: self.inner.get_mut(),
        }
    }

    // =========================================================================
    // Try Access (returns None if invalid/borrowed)
    // =========================================================================

    /// Returns a reference to the value, or `None` if invalid/borrowed.
    #[inline]
    pub fn try_value(&self) -> Option<SkipRef<K, V, MAX_LEVEL>> {
        self.inner.try_get().map(|inner| SkipRef { inner })
    }

    /// Returns a mutable reference to the value, or `None` if invalid/borrowed.
    #[inline]
    pub fn try_value_mut(&self) -> Option<SkipRefMut<K, V, MAX_LEVEL>> {
        self.inner.try_get_mut().map(|inner| SkipRefMut { inner })
    }

    // =========================================================================
    // Untracked Access (bypasses borrow tracking, checks validity)
    // =========================================================================

    /// Returns an untracked reference to the value if the entry is valid.
    ///
    /// # Safety
    ///
    /// Caller must ensure no concurrent mutable access to this slot.
    #[inline]
    pub unsafe fn value_untracked(&self) -> Option<&V> {
        unsafe { self.inner.get_untracked().map(|node| &node.value) }
    }

    /// Returns an untracked mutable reference to the value if the entry is valid.
    ///
    /// # Safety
    ///
    /// Caller must ensure exclusive access to this slot.
    #[inline]
    #[allow(clippy::mut_from_ref)] // Interior mutability via nexus_slab
    pub unsafe fn value_untracked_mut(&self) -> Option<&mut V> {
        unsafe { self.inner.get_untracked_mut().map(|node| &mut node.value) }
    }

    // =========================================================================
    // Unchecked Access (no checks at all)
    // =========================================================================

    /// Returns a reference to the value without any checks.
    ///
    /// # Safety
    ///
    /// - Entry must be valid
    /// - No concurrent mutable access to this slot
    #[inline]
    pub unsafe fn value_unchecked(&self) -> &V {
        unsafe { &self.inner.get_unchecked().value }
    }

    /// Returns a mutable reference to the value without any checks.
    ///
    /// # Safety
    ///
    /// - Entry must be valid
    /// - Exclusive access to this slot
    #[inline]
    #[allow(clippy::mut_from_ref)] // Interior mutability via nexus_slab
    pub unsafe fn value_unchecked_mut(&self) -> &mut V {
        unsafe { &mut self.inner.get_unchecked_mut().value }
    }
}

impl<K, V, const MAX_LEVEL: usize> Clone for SkipEntry<K, V, MAX_LEVEL> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<K, V, const MAX_LEVEL: usize> PartialEq for SkipEntry<K, V, MAX_LEVEL> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<K, V, const MAX_LEVEL: usize> Eq for SkipEntry<K, V, MAX_LEVEL> {}

impl<K, V, const MAX_LEVEL: usize> core::fmt::Debug for SkipEntry<K, V, MAX_LEVEL> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SkipEntry")
            .field("storage_key", &self.storage_key())
            .field("valid", &self.is_valid())
            .finish()
    }
}

// =============================================================================
// Ref Guards
// =============================================================================

/// RAII guard for a borrowed value reference.
///
/// Derefs to `&V` (value), not `&SkipNode<K, V, MAX_LEVEL>`.
/// Also provides access to the key via `.key()`.
/// Clears the borrow flag on drop.
pub struct SkipRef<K, V, const MAX_LEVEL: usize> {
    inner: nexus_slab::Ref<SkipNode<K, V, MAX_LEVEL>>,
}

impl<K, V, const MAX_LEVEL: usize> SkipRef<K, V, MAX_LEVEL> {
    /// Returns a reference to the sorted key.
    #[inline]
    pub fn key(&self) -> &K {
        &self.inner.key
    }
}

impl<K, V, const MAX_LEVEL: usize> Deref for SkipRef<K, V, MAX_LEVEL> {
    type Target = V;

    #[inline]
    fn deref(&self) -> &V {
        &self.inner.value
    }
}

impl<K, V: core::fmt::Debug, const MAX_LEVEL: usize> core::fmt::Debug for SkipRef<K, V, MAX_LEVEL> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

impl<K, V: core::fmt::Display, const MAX_LEVEL: usize> core::fmt::Display
    for SkipRef<K, V, MAX_LEVEL>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

/// RAII guard for a mutably borrowed value reference.
///
/// Derefs to `&V`/`&mut V` (value), not `&SkipNode<K, V, MAX_LEVEL>`.
/// Also provides access to the key via `.key()`.
/// Clears the borrow flag on drop.
///
/// # Warning
///
/// Do not mutate the key field - this would break the skip list ordering.
pub struct SkipRefMut<K, V, const MAX_LEVEL: usize> {
    inner: nexus_slab::RefMut<SkipNode<K, V, MAX_LEVEL>>,
}

impl<K, V, const MAX_LEVEL: usize> SkipRefMut<K, V, MAX_LEVEL> {
    /// Returns a reference to the sorted key.
    #[inline]
    pub fn key(&self) -> &K {
        &self.inner.key
    }
}

impl<K, V, const MAX_LEVEL: usize> Deref for SkipRefMut<K, V, MAX_LEVEL> {
    type Target = V;

    #[inline]
    fn deref(&self) -> &V {
        &self.inner.value
    }
}

impl<K, V, const MAX_LEVEL: usize> DerefMut for SkipRefMut<K, V, MAX_LEVEL> {
    #[inline]
    fn deref_mut(&mut self) -> &mut V {
        &mut self.inner.value
    }
}

impl<K, V: core::fmt::Debug, const MAX_LEVEL: usize> core::fmt::Debug
    for SkipRefMut<K, V, MAX_LEVEL>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

impl<K, V: core::fmt::Display, const MAX_LEVEL: usize> core::fmt::Display
    for SkipRefMut<K, V, MAX_LEVEL>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

// =============================================================================
// Vacant Entry
// =============================================================================

/// Reserved slot in skip storage for self-referential patterns.
///
/// After `insert()`, the entry exists in storage but is NOT in any skip list.
pub struct SkipVacant<K, V, const MAX_LEVEL: usize> {
    inner: nexus_slab::VacantEntry<SkipNode<K, V, MAX_LEVEL>>,
}

impl<K, V, const MAX_LEVEL: usize> SkipVacant<K, V, MAX_LEVEL> {
    /// Returns the storage key this slot will have once filled.
    #[inline]
    pub fn storage_key(&self) -> NexusKey {
        self.inner.key()
    }

    /// Fills the slot with a key-value pair.
    ///
    /// Returns a [`SkipEntry`] handle. The entry exists in storage but is
    /// NOT in any skip list.
    #[inline]
    pub fn insert(self, key: K, value: V, level: u8) -> SkipEntry<K, V, MAX_LEVEL> {
        let inner = self.inner.insert(SkipNode::new(key, value, level));
        SkipEntry::new(inner)
    }
}

impl<K, V, const MAX_LEVEL: usize> core::fmt::Debug for SkipVacant<K, V, MAX_LEVEL> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SkipVacant")
            .field("storage_key", &self.storage_key())
            .finish()
    }
}

/// Reserved slot in growable skip storage for self-referential patterns.
///
/// This is the growable storage equivalent of [`SkipVacant`].
pub struct GrowableSkipVacant<K, V, const MAX_LEVEL: usize> {
    inner: nexus_slab::SlabVacantEntry<SkipNode<K, V, MAX_LEVEL>>,
}

impl<K, V, const MAX_LEVEL: usize> GrowableSkipVacant<K, V, MAX_LEVEL> {
    /// Returns the storage key this slot will have once filled.
    #[inline]
    pub fn storage_key(&self) -> NexusKey {
        self.inner.key()
    }

    /// Fills the slot with a key-value pair.
    ///
    /// Returns a [`SkipEntry`] handle. The entry exists in storage but is
    /// NOT in any skip list.
    #[inline]
    pub fn insert(self, key: K, value: V, level: u8) -> SkipEntry<K, V, MAX_LEVEL> {
        let inner = self.inner.insert(SkipNode::new(key, value, level));
        SkipEntry::new(inner)
    }
}

impl<K, V, const MAX_LEVEL: usize> core::fmt::Debug for GrowableSkipVacant<K, V, MAX_LEVEL> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GrowableSkipVacant")
            .field("storage_key", &self.storage_key())
            .finish()
    }
}

// =============================================================================
// Storage Entry Methods
// =============================================================================

impl<K, V, const MAX_LEVEL: usize> SkipStorage<K, V, MAX_LEVEL> {
    // === Entry Access ===

    /// Creates an entry handle from a storage key.
    ///
    /// Returns `None` if the key is invalid.
    #[inline]
    pub fn entry(&self, key: NexusKey) -> Option<SkipEntry<K, V, MAX_LEVEL>> {
        self.inner.entry(key).map(SkipEntry::new)
    }

    // === Vacant Entry ===

    /// Reserves a slot for self-referential patterns.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full.
    #[inline]
    pub fn vacant(&self) -> Result<SkipVacant<K, V, MAX_LEVEL>, CapacityError> {
        self.inner.vacant_entry().map(|inner| SkipVacant { inner })
    }

    /// Inserts with access to the entry before the value exists.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full.
    #[inline]
    pub fn insert_with<F>(&self, f: F) -> Result<SkipEntry<K, V, MAX_LEVEL>, CapacityError>
    where
        F: FnOnce(SkipEntry<K, V, MAX_LEVEL>) -> (K, V, u8),
    {
        self.inner
            .insert_with(|slab_entry| {
                let skip_entry = SkipEntry::new(slab_entry);
                let (key, value, level) = f(skip_entry);
                SkipNode::new(key, value, level)
            })
            .map(SkipEntry::new)
    }
}

impl<K, V, const MAX_LEVEL: usize> GrowableSkipStorage<K, V, MAX_LEVEL> {
    // === Entry Access ===

    /// Creates an entry handle from a storage key.
    ///
    /// Returns `None` if the key is invalid.
    #[inline]
    pub fn entry(&self, key: NexusKey) -> Option<SkipEntry<K, V, MAX_LEVEL>> {
        self.inner.entry(key).map(SkipEntry::new)
    }

    // === Vacant Entry ===

    /// Reserves a slot for self-referential patterns.
    #[inline]
    pub fn vacant(&self) -> GrowableSkipVacant<K, V, MAX_LEVEL> {
        GrowableSkipVacant {
            inner: self.inner.vacant_entry(),
        }
    }

    /// Inserts with access to the entry before the value exists.
    #[inline]
    pub fn insert_with<F>(&self, f: F) -> SkipEntry<K, V, MAX_LEVEL>
    where
        F: FnOnce(SkipEntry<K, V, MAX_LEVEL>) -> (K, V, u8),
    {
        SkipEntry::new(self.inner.insert_with(|slab_entry| {
            let skip_entry = SkipEntry::new(slab_entry);
            let (key, value, level) = f(skip_entry);
            SkipNode::new(key, value, level)
        }))
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

    // =========================================================================
    // SkipEntry API Tests
    // =========================================================================

    #[test]
    fn skip_entry_basic() {
        let mut storage: SkipStorage<u64, String, 8> = SkipStorage::with_capacity(16);
        let key = storage
            .try_insert(SkipNode::new(42, "hello".to_string(), 3))
            .unwrap();

        let entry = storage.entry(key).unwrap();
        assert_eq!(entry.storage_key(), key);
        assert!(entry.is_valid());

        // Access value through entry
        assert_eq!(*entry.value(), "hello");
        assert_eq!(entry.value().key(), &42);
    }

    #[test]
    fn skip_entry_clone() {
        let mut storage: SkipStorage<u64, String, 8> = SkipStorage::with_capacity(16);
        let key = storage
            .try_insert(SkipNode::new(42, "hello".to_string(), 3))
            .unwrap();

        let entry = storage.entry(key).unwrap();
        let cached = entry.clone();

        // Keys are equal
        assert_eq!(entry.storage_key(), cached.storage_key());

        // Both access the same underlying value (check separately to avoid borrow conflict)
        assert_eq!(*entry.value(), "hello");
        assert_eq!(*cached.value(), "hello");
    }

    #[test]
    fn skip_entry_mutation() {
        let mut storage: SkipStorage<u64, String, 8> = SkipStorage::with_capacity(16);
        let key = storage
            .try_insert(SkipNode::new(42, "hello".to_string(), 3))
            .unwrap();

        let entry = storage.entry(key).unwrap();

        // Mutate through entry
        *entry.value_mut() = "world".to_string();
        assert_eq!(*entry.value(), "world");
    }

    #[test]
    fn skip_entry_try_methods() {
        let mut storage: SkipStorage<u64, String, 8> = SkipStorage::with_capacity(16);
        let key = storage
            .try_insert(SkipNode::new(42, "hello".to_string(), 3))
            .unwrap();

        let entry = storage.entry(key).unwrap();

        // try_value should succeed
        assert!(entry.try_value().is_some());
        assert_eq!(*entry.try_value().unwrap(), "hello");

        // try_value_mut should succeed
        assert!(entry.try_value_mut().is_some());
    }

    #[test]
    fn skip_entry_becomes_invalid_after_remove() {
        let mut storage: SkipStorage<u64, String, 8> = SkipStorage::with_capacity(16);
        let key = storage
            .try_insert(SkipNode::new(42, "hello".to_string(), 3))
            .unwrap();

        let entry = storage.entry(key).unwrap();
        assert!(entry.is_valid());

        // Remove the element
        storage.remove_node(key);

        // Entry should now be invalid
        assert!(!entry.is_valid());
        assert!(entry.try_value().is_none());
    }

    #[test]
    fn skip_entry_vacant_insert() {
        let storage: SkipStorage<u64, String, 8> = SkipStorage::with_capacity(16);

        // Reserve a slot
        let vacant = storage.vacant().unwrap();
        let reserved_key = vacant.storage_key();

        // Insert with the reserved key
        let entry = vacant.insert(42, "hello".to_string(), 3);
        assert_eq!(entry.storage_key(), reserved_key);
        assert_eq!(*entry.value(), "hello");
    }

    #[test]
    fn skip_entry_insert_with() {
        let storage: SkipStorage<u64, String, 8> = SkipStorage::with_capacity(16);

        // Insert with closure that has access to the entry key
        let entry = storage
            .insert_with(|entry| {
                let key = entry.storage_key();
                (42, format!("stored at {:?}", key), 3)
            })
            .unwrap();

        assert!(entry.value().starts_with("stored at Key"));
    }

    #[test]
    fn skip_entry_growable_basic() {
        let mut storage: GrowableSkipStorage<u64, String, 8> = GrowableSkipStorage::new();
        let key = storage.insert(SkipNode::new(42, "hello".to_string(), 3));

        let entry = storage.entry(key).unwrap();
        assert_eq!(entry.storage_key(), key);
        assert!(entry.is_valid());
        assert_eq!(*entry.value(), "hello");
    }

    #[test]
    fn skip_entry_growable_insert_with() {
        let storage: GrowableSkipStorage<u64, String, 8> = GrowableSkipStorage::new();

        let entry = storage.insert_with(|entry| {
            let key = entry.storage_key();
            (42, format!("stored at {:?}", key), 3)
        });

        assert!(entry.value().starts_with("stored at Key"));
    }

    #[test]
    fn skip_entry_growable_vacant() {
        let storage: GrowableSkipStorage<u64, String, 8> = GrowableSkipStorage::new();

        let vacant = storage.vacant();
        let reserved_key = vacant.storage_key();

        let entry = vacant.insert(42, "hello".to_string(), 3);
        assert_eq!(entry.storage_key(), reserved_key);
    }

    #[test]
    fn skip_ref_deref_to_value() {
        let mut storage: SkipStorage<u64, String, 8> = SkipStorage::with_capacity(16);
        let key = storage
            .try_insert(SkipNode::new(42, "hello".to_string(), 3))
            .unwrap();

        let entry = storage.entry(key).unwrap();
        let value_ref = entry.value();

        // Deref to &String
        assert_eq!(value_ref.len(), 5);
        assert!(value_ref.starts_with("hel"));

        // Access sorted key
        assert_eq!(value_ref.key(), &42);
    }

    #[test]
    fn skip_ref_mut_deref() {
        let mut storage: SkipStorage<u64, String, 8> = SkipStorage::with_capacity(16);
        let key = storage
            .try_insert(SkipNode::new(42, "hello".to_string(), 3))
            .unwrap();

        let entry = storage.entry(key).unwrap();
        let mut value_ref = entry.value_mut();

        // DerefMut to &mut String
        value_ref.push_str(" world");

        drop(value_ref);
        assert_eq!(*entry.value(), "hello world");
    }
}
