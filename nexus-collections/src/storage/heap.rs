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
//! # Entry API
//!
//! The entry API provides ergonomic access to stored values through cloneable
//! handles. See [`HeapEntry`] for details.
//!
//! ```
//! use nexus_collections::{Heap, HeapStorage};
//!
//! let mut storage: HeapStorage<u64> = HeapStorage::with_capacity(100);
//! let mut heap: Heap<u64, HeapStorage<u64>> = Heap::new();
//!
//! // Insert and get entry handle
//! let entry = heap.try_push_entry(&mut storage, 42).unwrap();
//!
//! // Direct access via entry (no key lookup needed)
//! assert_eq!(*entry.get(), 42);
//!
//! // Clone for caching
//! let cached = entry.clone();
//! assert_eq!(*cached.get(), 42);
//! ```
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

    /// Creates an entry handle from a key.
    ///
    /// Returns `None` if the key is invalid.
    fn entry(&self, key: NexusKey) -> Option<HeapEntry<T>>;
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

    /// Inserts with access to the entry before the value exists.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full.
    fn insert_with<F>(&self, f: F) -> Result<HeapEntry<T>, CapacityError>
    where
        F: FnOnce(HeapEntry<T>) -> T;
}

/// Operations for growable heap storage (infallible insertion).
pub trait GrowableHeapStorageOps<T>: HeapStorageOps<T> {
    /// Inserts a node, returning its key. May allocate.
    fn insert_node(&mut self, node: HeapNode<T>) -> NexusKey;

    /// Inserts with access to the entry before the value exists.
    fn insert_with<F>(&self, f: F) -> HeapEntry<T>
    where
        F: FnOnce(HeapEntry<T>) -> T;
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
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: BoundedSlab::with_capacity(capacity),
        }
    }

    /// Returns the total capacity.
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Returns the number of elements stored.
    pub fn len(&self) -> usize {
        self.inner.slab_len()
    }

    /// Returns `true` if no elements are stored.
    pub fn is_empty(&self) -> bool {
        self.inner.slab_is_empty()
    }

    /// Returns `true` if storage is at capacity.
    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }

    /// Returns `true` if the key is valid.
    pub fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    /// Attempts to insert a node, returning its key.
    pub(crate) fn try_insert(&mut self, node: HeapNode<T>) -> Result<NexusKey, Full<T>> {
        self.inner
            .insert(node)
            .map(|entry| entry.key())
            .map_err(|e| Full(e.0.data))
    }

    /// Returns a reference to the node at `key`.
    pub(crate) fn get_node(&self, key: NexusKey) -> Option<&HeapNode<T>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    /// Returns a mutable reference to the node at `key`.
    pub(crate) fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut HeapNode<T>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    pub(crate) unsafe fn get_node_unchecked(&self, key: NexusKey) -> &HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    /// Returns a mutable reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    pub(crate) unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    /// Removes and returns the node at `key`.
    pub(crate) fn remove_node(&mut self, key: NexusKey) -> Option<HeapNode<T>> {
        self.inner.slab_try_remove(key)
    }

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
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
    pub fn new() -> Self {
        Self { inner: Slab::new() }
    }

    /// Creates growable storage with pre-allocated capacity.
    ///
    /// The storage will grow beyond this if needed.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Slab::with_capacity(capacity),
        }
    }

    /// Returns the number of elements stored.
    pub fn len(&self) -> usize {
        self.inner.slab_len()
    }

    /// Returns `true` if no elements are stored.
    pub fn is_empty(&self) -> bool {
        self.inner.slab_is_empty()
    }

    /// Returns `true` if the key is valid.
    pub fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    /// Inserts a node, returning its key.
    pub(crate) fn insert(&mut self, node: HeapNode<T>) -> NexusKey {
        self.inner.insert(node).key()
    }

    /// Returns a reference to the node at `key`.
    pub(crate) fn get_node(&self, key: NexusKey) -> Option<&HeapNode<T>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    /// Returns a mutable reference to the node at `key`.
    pub(crate) fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut HeapNode<T>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    /// Returns a reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    pub(crate) unsafe fn get_node_unchecked(&self, key: NexusKey) -> &HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    /// Returns a mutable reference to the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    pub(crate) unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    /// Removes and returns the node at `key`.
    pub(crate) fn remove_node(&mut self, key: NexusKey) -> Option<HeapNode<T>> {
        self.inner.slab_try_remove(key)
    }

    /// Removes the node without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
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

    fn len(&self) -> usize {
        self.inner.slab_len()
    }

    fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    fn get_node(&self, key: NexusKey) -> Option<&HeapNode<T>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut HeapNode<T>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    fn remove_node(&mut self, key: NexusKey) -> Option<HeapNode<T>> {
        self.inner.slab_try_remove(key)
    }

    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> HeapNode<T> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }

    fn entry(&self, key: NexusKey) -> Option<HeapEntry<T>> {
        self.inner.entry(key).map(HeapEntry::new)
    }
}

impl<T> BoundedHeapStorageOps<T> for HeapStorage<T> {

    fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    fn try_insert_node(&mut self, node: HeapNode<T>) -> Result<NexusKey, Full<T>> {
        self.inner
            .insert(node)
            .map(|entry| entry.key())
            .map_err(|e| Full(e.0.data))
    }

    fn insert_with<F>(&self, f: F) -> Result<HeapEntry<T>, CapacityError>
    where
        F: FnOnce(HeapEntry<T>) -> T,
    {
        self.inner
            .insert_with(|slab_entry| {
                let heap_entry = HeapEntry::new(slab_entry);
                HeapNode::new(f(heap_entry))
            })
            .map(HeapEntry::new)
    }
}

impl<T> HeapStorageOps<T> for GrowableHeapStorage<T> {

    fn len(&self) -> usize {
        self.inner.slab_len()
    }

    fn contains(&self, key: NexusKey) -> bool {
        self.inner.slab_contains(key)
    }

    fn get_node(&self, key: NexusKey) -> Option<&HeapNode<T>> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { self.inner.slab_get_untracked(key) }
    }

    fn get_node_mut(&mut self, key: NexusKey) -> Option<&mut HeapNode<T>> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { self.inner.slab_get_untracked_mut(key) }
    }

    unsafe fn get_node_unchecked(&self, key: NexusKey) -> &HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked(key) }
    }

    unsafe fn get_node_unchecked_mut(&mut self, key: NexusKey) -> &mut HeapNode<T> {
        unsafe { self.inner.slab_get_unchecked_mut(key) }
    }

    fn remove_node(&mut self, key: NexusKey) -> Option<HeapNode<T>> {
        self.inner.slab_try_remove(key)
    }

    unsafe fn remove_node_unchecked(&mut self, key: NexusKey) -> HeapNode<T> {
        unsafe { self.inner.slab_remove_unchecked(key) }
    }

    fn entry(&self, key: NexusKey) -> Option<HeapEntry<T>> {
        self.inner.entry(key).map(HeapEntry::new)
    }
}

impl<T> GrowableHeapStorageOps<T> for GrowableHeapStorage<T> {

    fn insert_node(&mut self, node: HeapNode<T>) -> NexusKey {
        self.inner.insert(node).key()
    }

    fn insert_with<F>(&self, f: F) -> HeapEntry<T>
    where
        F: FnOnce(HeapEntry<T>) -> T,
    {
        HeapEntry::new(self.inner.insert_with(|slab_entry| {
            let heap_entry = HeapEntry::new(slab_entry);
            HeapNode::new(f(heap_entry))
        }))
    }
}

// =============================================================================
// Entry Types
// =============================================================================

/// Handle to an element in heap storage.
///
/// `HeapEntry` wraps `nexus_slab::Entry<HeapNode<T>>`, exposing only the user
/// data (not the internal node structure). Clone to cache in multiple locations.
///
/// # Entry vs Key
///
/// - **Key** (`NexusKey`): Lightweight identifier for storage slot
/// - **Entry** (`HeapEntry<T>`): Handle with direct access to value
///
/// Use entries when you want to cache handles for repeated access. Use keys
/// for passing to collection methods like `heap.update()` or `heap.remove()`.
///
/// # Example
///
/// ```
/// use nexus_collections::{Heap, HeapStorage};
/// use std::collections::HashMap;
///
/// let mut storage: HeapStorage<u64> = HeapStorage::with_capacity(100);
/// let mut heap: Heap<u64, HeapStorage<u64>> = Heap::new();
///
/// // Insert and cache entry
/// let entry = heap.try_push_entry(&mut storage, 42).unwrap();
/// let mut cache: HashMap<&str, _> = HashMap::new();
/// cache.insert("answer", entry.clone());
///
/// // Later: direct access via cached entry
/// let entry = cache.get("answer").unwrap();
/// assert_eq!(*entry.get(), 42);
/// ```
pub struct HeapEntry<T> {
    inner: nexus_slab::Entry<HeapNode<T>>,
}

impl<T> HeapEntry<T> {
    /// Creates a new entry from a nexus-slab entry.
    pub(crate) fn new(inner: nexus_slab::Entry<HeapNode<T>>) -> Self {
        Self { inner }
    }

    /// Returns the storage key.
    ///
    /// Use this for collection operations like `heap.update(storage, key)`.
    pub fn key(&self) -> NexusKey {
        self.inner.key()
    }

    /// Returns `true` if the entry is still valid (not removed).
    ///
    /// An entry becomes invalid when:
    /// - The element is removed from storage
    /// - The storage is dropped
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
    pub fn get(&self) -> HeapRef<T> {
        HeapRef {
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
    /// Mutating a heap element's ordering field (e.g., priority) breaks the
    /// heap property. After mutation, call `heap.update(storage, key)` to
    /// restore the heap invariant.
    pub fn get_mut(&self) -> HeapRefMut<T> {
        HeapRefMut {
            inner: self.inner.get_mut(),
        }
    }

    // =========================================================================
    // Try Access (returns None if invalid/borrowed)
    // =========================================================================

    /// Returns a reference to the value, or `None` if invalid/borrowed.
    pub fn try_get(&self) -> Option<HeapRef<T>> {
        self.inner.try_get().map(|inner| HeapRef { inner })
    }

    /// Returns a mutable reference to the value, or `None` if invalid/borrowed.
    pub fn try_get_mut(&self) -> Option<HeapRefMut<T>> {
        self.inner.try_get_mut().map(|inner| HeapRefMut { inner })
    }

    // =========================================================================
    // Untracked Access (bypasses borrow tracking, checks validity)
    // =========================================================================

    /// Returns an untracked reference if the entry is valid.
    ///
    /// This bypasses runtime borrow tracking for performance.
    ///
    /// # Safety
    ///
    /// Caller must ensure no concurrent mutable access to this slot.
    pub unsafe fn get_untracked(&self) -> Option<&T> {
        // SAFETY: Caller ensures no concurrent mutable access.
        unsafe { self.inner.get_untracked().map(|node| &node.data) }
    }

    /// Returns an untracked mutable reference if the entry is valid.
    ///
    /// This bypasses runtime borrow tracking for performance.
    ///
    /// # Safety
    ///
    /// Caller must ensure exclusive access to this slot.
    #[allow(clippy::mut_from_ref)] // Interior mutability via nexus_slab
    pub unsafe fn get_untracked_mut(&self) -> Option<&mut T> {
        // SAFETY: Caller ensures exclusive access.
        unsafe { self.inner.get_untracked_mut().map(|node| &mut node.data) }
    }

    // =========================================================================
    // Unchecked Access (no checks at all)
    // =========================================================================

    /// Returns a reference without any checks.
    ///
    /// # Safety
    ///
    /// - Entry must be valid (not removed, storage not dropped)
    /// - No concurrent mutable access to this slot
    pub unsafe fn get_unchecked(&self) -> &T {
        // SAFETY: Caller ensures entry is valid and no concurrent mutable access.
        unsafe { &self.inner.get_unchecked().data }
    }

    /// Returns a mutable reference without any checks.
    ///
    /// # Safety
    ///
    /// - Entry must be valid (not removed, storage not dropped)
    /// - Exclusive access to this slot
    #[allow(clippy::mut_from_ref)] // Interior mutability via nexus_slab
    pub unsafe fn get_unchecked_mut(&self) -> &mut T {
        // SAFETY: Caller ensures entry is valid and exclusive access.
        unsafe { &mut self.inner.get_unchecked_mut().data }
    }
}

impl<T> Clone for HeapEntry<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> PartialEq for HeapEntry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T> Eq for HeapEntry<T> {}

impl<T> core::fmt::Debug for HeapEntry<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HeapEntry")
            .field("key", &self.key())
            .field("valid", &self.is_valid())
            .finish()
    }
}

// =============================================================================
// Ref Guards
// =============================================================================

/// RAII guard for a borrowed value reference.
///
/// Derefs to `&T` (user data), not `&HeapNode<T>`.
/// Clears the borrow flag on drop.
pub struct HeapRef<T> {
    inner: nexus_slab::Ref<HeapNode<T>>,
}

impl<T> Deref for HeapRef<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner.data
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for HeapRef<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: core::fmt::Display> core::fmt::Display for HeapRef<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

/// RAII guard for a mutably borrowed value reference.
///
/// Derefs to `&T`/`&mut T` (user data), not `&HeapNode<T>`.
/// Clears the borrow flag on drop.
pub struct HeapRefMut<T> {
    inner: nexus_slab::RefMut<HeapNode<T>>,
}

impl<T> Deref for HeapRefMut<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner.data
    }
}

impl<T> DerefMut for HeapRefMut<T> {

    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner.data
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for HeapRefMut<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: core::fmt::Display> core::fmt::Display for HeapRefMut<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

// =============================================================================
// Vacant Entry
// =============================================================================

/// Reserved slot in heap storage for self-referential patterns.
///
/// After `insert()`, the entry exists in storage but is NOT in any heap.
/// Use `heap.link()` to add it to a heap.
///
/// # Example
///
/// ```
/// use nexus_collections::{Heap, HeapStorage};
/// use nexus_slab::Key as NexusKey;
///
/// #[derive(Eq, PartialEq)]
/// struct Task {
///     id: u64,
///     priority: i32,
///     self_key: NexusKey,
/// }
///
/// impl Ord for Task {
///     fn cmp(&self, other: &Self) -> std::cmp::Ordering {
///         self.priority.cmp(&other.priority)
///     }
/// }
/// impl PartialOrd for Task {
///     fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
///         Some(self.cmp(other))
///     }
/// }
///
/// let mut storage: HeapStorage<Task> = HeapStorage::with_capacity(100);
/// let mut heap: Heap<Task, HeapStorage<Task>> = Heap::new();
///
/// // Reserve slot, get key, create self-referential value
/// let vacant = storage.vacant().unwrap();
/// let key = vacant.key();
/// let entry = vacant.insert(Task {
///     id: 1,
///     priority: 10,
///     self_key: key,
/// });
///
/// // Link into heap
/// heap.link(&mut storage, entry.key());
/// ```
pub struct HeapVacant<T> {
    inner: nexus_slab::VacantEntry<HeapNode<T>>,
}

impl<T> HeapVacant<T> {
    /// Returns the key this slot will have once filled.
    pub fn key(&self) -> NexusKey {
        self.inner.key()
    }

    /// Fills the slot with a value.
    ///
    /// Returns a [`HeapEntry`] handle. The entry exists in storage but is
    /// NOT in any heap. Use `heap.link()` to add it to a heap.
    pub fn insert(self, value: T) -> HeapEntry<T> {
        let inner = self.inner.insert(HeapNode::new(value));
        HeapEntry::new(inner)
    }
}

impl<T> core::fmt::Debug for HeapVacant<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HeapVacant")
            .field("key", &self.key())
            .finish()
    }
}

/// Reserved slot in growable heap storage for self-referential patterns.
///
/// This is the growable storage equivalent of [`HeapVacant`].
/// After `insert()`, the entry exists in storage but is NOT in any heap.
pub struct GrowableHeapVacant<T> {
    inner: nexus_slab::SlabVacantEntry<HeapNode<T>>,
}

impl<T> GrowableHeapVacant<T> {
    /// Returns the key this slot will have once filled.
    pub fn key(&self) -> NexusKey {
        self.inner.key()
    }

    /// Fills the slot with a value.
    ///
    /// Returns a [`HeapEntry`] handle. The entry exists in storage but is
    /// NOT in any heap. Use `heap.link()` to add it to a heap.
    pub fn insert(self, value: T) -> HeapEntry<T> {
        let inner = self.inner.insert(HeapNode::new(value));
        HeapEntry::new(inner)
    }
}

impl<T> core::fmt::Debug for GrowableHeapVacant<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GrowableHeapVacant")
            .field("key", &self.key())
            .finish()
    }
}

// =============================================================================
// Storage Entry Methods
// =============================================================================

impl<T> HeapStorage<T> {
    // === Value Access ===

    /// Returns a reference to the value at `key`.
    ///
    /// Returns `None` if the key is invalid.
    pub fn get(&self, key: NexusKey) -> Option<&T> {
        self.get_node(key).map(|node| &node.data)
    }

    /// Returns a mutable reference to the value at `key`.
    ///
    /// Returns `None` if the key is invalid.
    ///
    /// # Note
    ///
    /// Mutating a heap element's ordering field breaks the heap property.
    /// After mutation, call `heap.update(storage, key)` to restore the
    /// heap invariant.
    pub fn get_mut(&mut self, key: NexusKey) -> Option<&mut T> {
        self.get_node_mut(key).map(|node| &mut node.data)
    }

    // === Entry Access ===

    /// Creates an entry handle from a key.
    ///
    /// Returns `None` if the key is invalid.
    pub fn entry(&self, key: NexusKey) -> Option<HeapEntry<T>> {
        self.inner.entry(key).map(HeapEntry::new)
    }

    // === Vacant Entry ===

    /// Reserves a slot for self-referential patterns.
    ///
    /// The slot is allocated but not yet filled. Use [`HeapVacant::insert`]
    /// to fill it, then `heap.link()` to add it to a heap.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full.
    pub fn vacant(&self) -> Result<HeapVacant<T>, CapacityError> {
        self.inner.vacant_entry().map(|inner| HeapVacant { inner })
    }

    /// Inserts with access to the entry before the value exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    /// The closure receives a [`HeapEntry`] that will point to the value
    /// once created.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full. The closure is not
    /// called in this case.
    ///
    /// # Note
    ///
    /// The returned entry is NOT in any heap. Use `heap.link()` to add it.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_collections::{Heap, HeapStorage};
    /// use nexus_slab::Key as NexusKey;
    ///
    /// struct Task {
    ///     id: u64,
    ///     self_key: NexusKey,
    /// }
    ///
    /// let storage: HeapStorage<Task> = HeapStorage::with_capacity(100);
    ///
    /// let entry = storage.insert_with(|e| Task {
    ///     id: 1,
    ///     self_key: e.key(),
    /// }).unwrap();
    ///
    /// assert_eq!(entry.get().self_key, entry.key());
    /// ```
    pub fn insert_with<F>(&self, f: F) -> Result<HeapEntry<T>, CapacityError>
    where
        F: FnOnce(HeapEntry<T>) -> T,
    {
        self.inner
            .insert_with(|slab_entry| {
                let heap_entry = HeapEntry::new(slab_entry);
                HeapNode::new(f(heap_entry))
            })
            .map(HeapEntry::new)
    }
}

impl<T> GrowableHeapStorage<T> {
    // === Value Access ===

    /// Returns a reference to the value at `key`.
    ///
    /// Returns `None` if the key is invalid.
    pub fn get(&self, key: NexusKey) -> Option<&T> {
        self.get_node(key).map(|node| &node.data)
    }

    /// Returns a mutable reference to the value at `key`.
    ///
    /// Returns `None` if the key is invalid.
    pub fn get_mut(&mut self, key: NexusKey) -> Option<&mut T> {
        self.get_node_mut(key).map(|node| &mut node.data)
    }

    // === Entry Access ===

    /// Creates an entry handle from a key.
    ///
    /// Returns `None` if the key is invalid.
    pub fn entry(&self, key: NexusKey) -> Option<HeapEntry<T>> {
        self.inner.entry(key).map(HeapEntry::new)
    }

    // === Vacant Entry ===

    /// Reserves a slot for self-referential patterns.
    ///
    /// The slot is allocated but not yet filled. Use [`GrowableHeapVacant::insert`]
    /// to fill it, then `heap.link()` to add it to a heap.
    pub fn vacant(&self) -> GrowableHeapVacant<T> {
        GrowableHeapVacant {
            inner: self.inner.vacant_entry(),
        }
    }

    /// Inserts with access to the entry before the value exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    ///
    /// # Note
    ///
    /// The returned entry is NOT in any heap. Use `heap.link()` to add it.
    pub fn insert_with<F>(&self, f: F) -> HeapEntry<T>
    where
        F: FnOnce(HeapEntry<T>) -> T,
    {
        HeapEntry::new(self.inner.insert_with(|slab_entry| {
            let heap_entry = HeapEntry::new(slab_entry);
            HeapNode::new(f(heap_entry))
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
