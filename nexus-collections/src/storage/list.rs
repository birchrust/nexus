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
//! # Entry API
//!
//! The entry API provides ergonomic access to stored values through cloneable
//! handles. See [`ListEntry`] for details.
//!
//! ```
//! use nexus_collections::{List, ListStorage};
//!
//! let mut storage: ListStorage<u64> = ListStorage::with_capacity(100);
//! let mut list: List<u64, ListStorage<u64>> = List::new();
//!
//! // Insert and get entry handle
//! let entry = list.try_push_back_entry(&mut storage, 42).unwrap();
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

use core::ops::{Deref, DerefMut};

use crate::internal::SlabOps;

use super::Full;
use nexus_slab::{BoundedSlab, CapacityError, Key as NexusKey, Slab};

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

    /// Creates an entry handle from a key.
    ///
    /// Returns `None` if the key is invalid.
    fn entry(&self, key: NexusKey) -> Option<ListEntry<T>>;
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

    /// Inserts with access to the entry before the value exists.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full.
    fn insert_with<F>(&self, f: F) -> Result<ListEntry<T>, CapacityError>
    where
        F: FnOnce(ListEntry<T>) -> T;
}

/// Operations for growable list storage (infallible insertion).
pub trait GrowableListStorageOps<T>: ListStorageOps<T> {
    /// Inserts a node, returning its key. May allocate.
    fn insert_node(&mut self, node: ListNode<T>) -> NexusKey;

    /// Inserts with access to the entry before the value exists.
    fn insert_with<F>(&self, f: F) -> ListEntry<T>
    where
        F: FnOnce(ListEntry<T>) -> T;
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

    #[inline]
    fn entry(&self, key: NexusKey) -> Option<ListEntry<T>> {
        self.inner.entry(key).map(ListEntry::new)
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

    #[inline]
    fn insert_with<F>(&self, f: F) -> Result<ListEntry<T>, CapacityError>
    where
        F: FnOnce(ListEntry<T>) -> T,
    {
        self.inner
            .insert_with(|slab_entry| {
                let list_entry = ListEntry::new(slab_entry);
                ListNode::new(f(list_entry))
            })
            .map(ListEntry::new)
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

    #[inline]
    fn entry(&self, key: NexusKey) -> Option<ListEntry<T>> {
        self.inner.entry(key).map(ListEntry::new)
    }
}

impl<T> GrowableListStorageOps<T> for GrowableListStorage<T> {
    #[inline]
    fn insert_node(&mut self, node: ListNode<T>) -> NexusKey {
        self.inner.insert(node).key()
    }

    #[inline]
    fn insert_with<F>(&self, f: F) -> ListEntry<T>
    where
        F: FnOnce(ListEntry<T>) -> T,
    {
        ListEntry::new(self.inner.insert_with(|slab_entry| {
            let list_entry = ListEntry::new(slab_entry);
            ListNode::new(f(list_entry))
        }))
    }
}

impl<T> Default for GrowableListStorage<T> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Entry Types
// =============================================================================

/// Handle to an element in list storage.
///
/// `ListEntry` wraps `nexus_slab::Entry<ListNode<T>>`, exposing only the user
/// data (not the internal node structure). Clone to cache in multiple locations.
///
/// # Entry vs Key
///
/// - **Key** (`NexusKey`): Lightweight identifier for storage slot
/// - **Entry** (`ListEntry<T>`): Handle with direct access to value
///
/// Use entries when you want to cache handles for repeated access. Use keys
/// for passing to collection methods like `list.remove()`.
///
/// # Example
///
/// ```
/// use nexus_collections::{List, ListStorage};
/// use std::collections::HashMap;
///
/// let mut storage: ListStorage<u64> = ListStorage::with_capacity(100);
/// let mut list: List<u64, ListStorage<u64>> = List::new();
///
/// // Insert and cache entry
/// let entry = list.try_push_back_entry(&mut storage, 42).unwrap();
/// let mut cache: HashMap<&str, _> = HashMap::new();
/// cache.insert("answer", entry.clone());
///
/// // Later: direct access via cached entry
/// let entry = cache.get("answer").unwrap();
/// assert_eq!(*entry.get(), 42);
/// ```
pub struct ListEntry<T> {
    inner: nexus_slab::Entry<ListNode<T>>,
}

impl<T> ListEntry<T> {
    /// Creates a new entry from a nexus-slab entry.
    #[inline]
    pub(crate) fn new(inner: nexus_slab::Entry<ListNode<T>>) -> Self {
        Self { inner }
    }

    /// Returns the storage key.
    ///
    /// Use this for collection operations like `list.remove(storage, key)`.
    #[inline]
    pub fn key(&self) -> NexusKey {
        self.inner.key()
    }

    /// Returns `true` if the entry is still valid (not removed).
    ///
    /// An entry becomes invalid when:
    /// - The element is removed from storage
    /// - The storage is dropped
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
    pub fn get(&self) -> ListRef<T> {
        ListRef {
            inner: self.inner.get(),
        }
    }

    /// Returns a mutable reference to the value.
    ///
    /// # Panics
    ///
    /// Panics if the entry is invalid or already borrowed.
    #[inline]
    pub fn get_mut(&self) -> ListRefMut<T> {
        ListRefMut {
            inner: self.inner.get_mut(),
        }
    }

    // =========================================================================
    // Try Access (returns None if invalid/borrowed)
    // =========================================================================

    /// Returns a reference to the value, or `None` if invalid/borrowed.
    #[inline]
    pub fn try_get(&self) -> Option<ListRef<T>> {
        self.inner.try_get().map(|inner| ListRef { inner })
    }

    /// Returns a mutable reference to the value, or `None` if invalid/borrowed.
    #[inline]
    pub fn try_get_mut(&self) -> Option<ListRefMut<T>> {
        self.inner.try_get_mut().map(|inner| ListRefMut { inner })
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
    #[inline]
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
    #[inline]
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
    #[inline]
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
    #[inline]
    #[allow(clippy::mut_from_ref)] // Interior mutability via nexus_slab
    pub unsafe fn get_unchecked_mut(&self) -> &mut T {
        // SAFETY: Caller ensures entry is valid and exclusive access.
        unsafe { &mut self.inner.get_unchecked_mut().data }
    }
}

impl<T> Clone for ListEntry<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> PartialEq for ListEntry<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T> Eq for ListEntry<T> {}

impl<T> core::fmt::Debug for ListEntry<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ListEntry")
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
/// Derefs to `&T` (user data), not `&ListNode<T>`.
/// Clears the borrow flag on drop.
pub struct ListRef<T> {
    inner: nexus_slab::Ref<ListNode<T>>,
}

impl<T> Deref for ListRef<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.inner.data
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for ListRef<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: core::fmt::Display> core::fmt::Display for ListRef<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

/// RAII guard for a mutably borrowed value reference.
///
/// Derefs to `&T`/`&mut T` (user data), not `&ListNode<T>`.
/// Clears the borrow flag on drop.
pub struct ListRefMut<T> {
    inner: nexus_slab::RefMut<ListNode<T>>,
}

impl<T> Deref for ListRefMut<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.inner.data
    }
}

impl<T> DerefMut for ListRefMut<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner.data
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for ListRefMut<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: core::fmt::Display> core::fmt::Display for ListRefMut<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

// =============================================================================
// Vacant Entry
// =============================================================================

/// Reserved slot in list storage for self-referential patterns.
///
/// After `insert()`, the entry exists in storage but is NOT in any list.
/// Use `list.link_back()` or `list.link_front()` to add it to a list.
///
/// # Example
///
/// ```
/// use nexus_collections::{List, ListStorage};
/// use nexus_slab::Key as NexusKey;
///
/// #[derive(Debug)]
/// struct Order {
///     id: u64,
///     self_key: NexusKey,
/// }
///
/// let mut storage: ListStorage<Order> = ListStorage::with_capacity(100);
/// let mut list: List<Order, ListStorage<Order>> = List::new();
///
/// // Reserve slot, get key, create self-referential value
/// let vacant = storage.vacant().unwrap();
/// let key = vacant.key();
/// let entry = vacant.insert(Order {
///     id: 1,
///     self_key: key,
/// });
///
/// // Link into list
/// list.link_back(&mut storage, entry.key());
/// ```
pub struct ListVacant<T> {
    inner: nexus_slab::VacantEntry<ListNode<T>>,
}

impl<T> ListVacant<T> {
    /// Returns the key this slot will have once filled.
    #[inline]
    pub fn key(&self) -> NexusKey {
        self.inner.key()
    }

    /// Fills the slot with a value.
    ///
    /// Returns a [`ListEntry`] handle. The entry exists in storage but is
    /// NOT in any list. Use `list.link_back()` or `list.link_front()` to add it.
    #[inline]
    pub fn insert(self, value: T) -> ListEntry<T> {
        let inner = self.inner.insert(ListNode::new(value));
        ListEntry::new(inner)
    }
}

impl<T> core::fmt::Debug for ListVacant<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ListVacant")
            .field("key", &self.key())
            .finish()
    }
}

/// Reserved slot in growable list storage for self-referential patterns.
///
/// This is the growable storage equivalent of [`ListVacant`].
/// After `insert()`, the entry exists in storage but is NOT in any list.
pub struct GrowableListVacant<T> {
    inner: nexus_slab::SlabVacantEntry<ListNode<T>>,
}

impl<T> GrowableListVacant<T> {
    /// Returns the key this slot will have once filled.
    #[inline]
    pub fn key(&self) -> NexusKey {
        self.inner.key()
    }

    /// Fills the slot with a value.
    ///
    /// Returns a [`ListEntry`] handle. The entry exists in storage but is
    /// NOT in any list. Use `list.link_back()` or `list.link_front()` to add it.
    #[inline]
    pub fn insert(self, value: T) -> ListEntry<T> {
        let inner = self.inner.insert(ListNode::new(value));
        ListEntry::new(inner)
    }
}

impl<T> core::fmt::Debug for GrowableListVacant<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GrowableListVacant")
            .field("key", &self.key())
            .finish()
    }
}

// =============================================================================
// Storage Entry Methods
// =============================================================================

impl<T> ListStorage<T> {
    // === Value Access ===

    /// Returns a reference to the value at `key`.
    ///
    /// Returns `None` if the key is invalid.
    #[inline]
    pub fn get(&self, key: NexusKey) -> Option<&T> {
        self.get_node(key).map(|node| &node.data)
    }

    /// Returns a mutable reference to the value at `key`.
    ///
    /// Returns `None` if the key is invalid.
    #[inline]
    pub fn get_mut(&mut self, key: NexusKey) -> Option<&mut T> {
        self.get_node_mut(key).map(|node| &mut node.data)
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

    // === Entry Access ===

    /// Creates an entry handle from a key.
    ///
    /// Returns `None` if the key is invalid.
    #[inline]
    pub fn entry(&self, key: NexusKey) -> Option<ListEntry<T>> {
        self.inner.entry(key).map(ListEntry::new)
    }

    // === Vacant Entry ===

    /// Reserves a slot for self-referential patterns.
    ///
    /// The slot is allocated but not yet filled. Use [`ListVacant::insert`]
    /// to fill it, then `list.link_back()` or `list.link_front()` to add it.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full.
    #[inline]
    pub fn vacant(&self) -> Result<ListVacant<T>, CapacityError> {
        self.inner.vacant_entry().map(|inner| ListVacant { inner })
    }

    /// Inserts with access to the entry before the value exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    /// The closure receives a [`ListEntry`] that will point to the value
    /// once created.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full. The closure is not
    /// called in this case.
    ///
    /// # Note
    ///
    /// The returned entry is NOT in any list. Use `list.link_back()` or
    /// `list.link_front()` to add it.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_collections::{List, ListStorage};
    /// use nexus_slab::Key as NexusKey;
    ///
    /// struct Order {
    ///     id: u64,
    ///     self_key: NexusKey,
    /// }
    ///
    /// let storage: ListStorage<Order> = ListStorage::with_capacity(100);
    ///
    /// let entry = storage.insert_with(|e| Order {
    ///     id: 1,
    ///     self_key: e.key(),
    /// }).unwrap();
    ///
    /// assert_eq!(entry.get().self_key, entry.key());
    /// ```
    #[inline]
    pub fn insert_with<F>(&self, f: F) -> Result<ListEntry<T>, CapacityError>
    where
        F: FnOnce(ListEntry<T>) -> T,
    {
        self.inner
            .insert_with(|slab_entry| {
                let list_entry = ListEntry::new(slab_entry);
                ListNode::new(f(list_entry))
            })
            .map(ListEntry::new)
    }
}

impl<T> GrowableListStorage<T> {
    // === Value Access ===

    /// Returns a reference to the value at `key`.
    ///
    /// Returns `None` if the key is invalid.
    #[inline]
    pub fn get(&self, key: NexusKey) -> Option<&T> {
        self.get_node(key).map(|node| &node.data)
    }

    /// Returns a mutable reference to the value at `key`.
    ///
    /// Returns `None` if the key is invalid.
    #[inline]
    pub fn get_mut(&mut self, key: NexusKey) -> Option<&mut T> {
        self.get_node_mut(key).map(|node| &mut node.data)
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

    // === Entry Access ===

    /// Creates an entry handle from a key.
    ///
    /// Returns `None` if the key is invalid.
    #[inline]
    pub fn entry(&self, key: NexusKey) -> Option<ListEntry<T>> {
        self.inner.entry(key).map(ListEntry::new)
    }

    // === Vacant Entry ===

    /// Reserves a slot for self-referential patterns.
    ///
    /// The slot is allocated but not yet filled. Use [`GrowableListVacant::insert`]
    /// to fill it, then `list.link_back()` or `list.link_front()` to add it.
    #[inline]
    pub fn vacant(&self) -> GrowableListVacant<T> {
        GrowableListVacant {
            inner: self.inner.vacant_entry(),
        }
    }

    /// Inserts with access to the entry before the value exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    ///
    /// # Note
    ///
    /// The returned entry is NOT in any list. Use `list.link_back()` or
    /// `list.link_front()` to add it.
    #[inline]
    pub fn insert_with<F>(&self, f: F) -> ListEntry<T>
    where
        F: FnOnce(ListEntry<T>) -> T,
    {
        ListEntry::new(self.inner.insert_with(|slab_entry| {
            let list_entry = ListEntry::new(slab_entry);
            ListNode::new(f(list_entry))
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
