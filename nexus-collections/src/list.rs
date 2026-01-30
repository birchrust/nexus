//! Doubly-linked list with internal node storage.
//!
//! Nodes are stored in external storage, with the list managing the links
//! internally. This allows O(1) insertion and removal without requiring
//! users to embed link fields in their types.
//!
//! # Storage Invariant
//!
//! A list instance must always be used with the same storage instance.
//! Passing a different storage is undefined behavior. This is the caller's
//! responsibility to enforce (same discipline as the `slab` crate).
//!
//! # Bounded vs Unbounded Storage
//!
//! Insert operations have different APIs depending on storage type:
//!
//! ```
//! use nexus_collections::{ListStorage, List};
//!
//! // Bounded storage (BoxedStorage, nexus_slab) - fallible insertion
//! let mut storage: ListStorage<u64> = ListStorage::with_capacity(16);
//! let mut list: List<u64, ListStorage<u64>> = List::new();
//!
//! let key = list.try_push_back(&mut storage, 42).unwrap();
//! ```
//!
//! ```ignore
//! // Unbounded storage (slab::Slab) - infallible insertion
//! let mut storage: SlabListStorage<u64> = slab::Slab::with_capacity(16);
//! let mut list: List<u64, SlabListStorage<u64>, usize> = List::new();
//!
//! let key = list.push_back(&mut storage, 42); // No Result!
//! ```
//!
//! # Example
//!
//! ```
//! use nexus_collections::{ListStorage, List};
//!
//! let mut storage: ListStorage<u64> = ListStorage::with_capacity(16);
//! let mut list: List<u64, ListStorage<u64>> = List::new();
//!
//! // Insert values - returns key for O(1) access/removal later
//! let a = list.try_push_back(&mut storage, 1).unwrap();
//! let b = list.try_push_back(&mut storage, 2).unwrap();
//! let c = list.try_push_back(&mut storage, 3).unwrap();
//!
//! assert_eq!(list.len(), 3);
//! assert_eq!(list.get(&storage, b), Some(&2));
//!
//! // Remove from middle - O(1)
//! let value = list.remove(&mut storage, b);
//! assert_eq!(value, Some(2));
//! assert_eq!(list.len(), 2);
//!
//! // Pop from front
//! assert_eq!(list.pop_front(&mut storage), Some(1));
//! assert_eq!(list.pop_front(&mut storage), Some(3));
//! assert_eq!(list.pop_front(&mut storage), None);
//! ```
//!
//! # Moving Between Lists
//!
//! Use `unlink` and `link_back`/`link_front` to move nodes between lists
//! without deallocating. The storage key remains stable.
//!
//! ```
//! use nexus_collections::{ListStorage, List};
//!
//! let mut storage: ListStorage<u64> = ListStorage::with_capacity(16);
//! let mut list_a: List<u64, ListStorage<u64>> = List::new();
//! let mut list_b: List<u64, ListStorage<u64>> = List::new();
//!
//! let key = list_a.try_push_back(&mut storage, 42).unwrap();
//!
//! // Move to list_b - key stays valid
//! list_a.unlink(&mut storage, key);
//! list_b.link_back(&mut storage, key);
//!
//! assert!(list_a.is_empty());
//! assert_eq!(list_b.get(&storage, key), Some(&42));
//! ```
//!
//! # Use Case: Order Queues
//!
//! Multiple price-level queues sharing one storage pool:
//!
//! ```
//! use nexus_collections::{ListStorage, List};
//!
//! #[derive(Debug)]
//! struct Order {
//!     id: u64,
//!     qty: u64,
//! }
//!
//! // One storage for all orders
//! let mut orders: ListStorage<Order> = ListStorage::with_capacity(100_000);
//!
//! // Separate queues per price level
//! let mut queue_100: List<Order, ListStorage<Order>> = List::new();
//! let mut queue_101: List<Order, ListStorage<Order>> = List::new();
//!
//! let key = queue_100.try_push_back(&mut orders, Order { id: 1, qty: 50 }).unwrap();
//!
//! // Price amendment: move order to different level
//! queue_100.unlink(&mut orders, key);
//! queue_101.link_back(&mut orders, key);
//! // Client's handle (key) remains valid
//! ```

use std::marker::PhantomData;

use nexus_slab::Key as NexusKey;

use crate::storage::{
    BoundedListStorageOps, Full, GrowableListStorageOps, ListEntry, ListNode, ListStorageOps,
};

/// A doubly-linked list over external storage.
///
/// The list tracks head, tail, and length. Nodes live in user-provided
/// storage, wrapped in [`ListNode`].
///
/// # Type Parameters
///
/// - `T`: Element type
/// - `S`: Storage type (e.g., [`ListStorage<T>`])
/// - `NexusKey`: Key type (default `u32`)
///
/// # Example
///
/// ```
/// use nexus_collections::{ListStorage, List};
///
/// let mut storage: ListStorage<String> = ListStorage::with_capacity(100);
/// let mut list: List<String, ListStorage<String>> = List::new();
///
/// let key = list.try_push_back(&mut storage, "hello".into()).unwrap();
/// assert_eq!(list.get(&storage, key), Some(&"hello".into()));
/// ```
#[derive(Debug)]
pub struct List<T, S>
where
    S: ListStorageOps<T>,
{
    head: NexusKey,
    tail: NexusKey,
    len: usize,
    _marker: PhantomData<(T, S)>,
}

impl<T, S> Default for List<T, S>
where
    S: ListStorageOps<T>,
{
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Base impl - works with any Storage (read/link/remove operations)
// =============================================================================

impl<T, S> List<T, S>
where
    S: ListStorageOps<T>,
{
    /// Creates an empty list.
    #[inline]
    pub const fn new() -> Self {
        Self {
            head: NexusKey::NONE,
            tail: NexusKey::NONE,
            len: 0,
            _marker: PhantomData,
        }
    }

    /// Returns the number of elements in the list.
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the list is empty.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the head node's key, or `None` if empty.
    #[inline]
    pub fn front_key(&self) -> Option<NexusKey> {
        if self.head.is_none() {
            None
        } else {
            Some(self.head)
        }
    }

    /// Returns the tail node's key, or `None` if empty.
    #[inline]
    pub fn back_key(&self) -> Option<NexusKey> {
        if self.tail.is_none() {
            None
        } else {
            Some(self.tail)
        }
    }

    // ========================================================================
    // Remove operations (unlink + deallocate)
    // ========================================================================

    /// Removes and returns the front element.
    ///
    /// Returns `None` if the list is empty.
    #[inline]
    pub fn pop_front(&mut self, storage: &mut S) -> Option<T> {
        if self.head.is_none() {
            return None;
        }

        let key = self.head;
        self.unlink(storage, key);
        storage.remove_node(key).map(|node| node.data)
    }

    /// Removes and returns the back element.
    ///
    /// Returns `None` if the list is empty.
    #[inline]
    pub fn pop_back(&mut self, storage: &mut S) -> Option<T> {
        if self.tail.is_none() {
            return None;
        }

        let key = self.tail;
        self.unlink(storage, key);
        storage.remove_node(key).map(|node| node.data)
    }

    /// Removes an element by key.
    ///
    /// Returns `None` if the key is invalid.
    #[inline]
    pub fn remove(&mut self, storage: &mut S, key: NexusKey) -> Option<T> {
        storage.get_node(key)?;
        self.unlink(storage, key);
        storage.remove_node(key).map(|node| node.data)
    }

    // ========================================================================
    // Link operations (just relink, no alloc/dealloc)
    // ========================================================================

    /// Links an existing node to the back of the list.
    ///
    /// The node must already exist in storage but not be in any list.
    /// Use this with `unlink` to move nodes between lists.
    ///
    /// # Panics
    ///
    /// Panics if `key` is not valid in storage.
    #[inline]
    pub fn link_back(&mut self, storage: &mut S, key: NexusKey) {
        let node = storage.get_node_mut(key).expect("invalid key");
        node.prev = self.tail;
        node.next = NexusKey::NONE;

        if self.tail.is_some() {
            // Safety: tail is valid when is_some()
            unsafe { storage.get_node_unchecked_mut(self.tail) }.next = key;
        } else {
            self.head = key;
        }

        self.tail = key;
        self.len += 1;
    }

    /// Links an existing node to the front of the list.
    ///
    /// The node must already exist in storage but not be in any list.
    /// Use this with `unlink` to move nodes between lists.
    ///
    /// # Panics
    ///
    /// Panics if `key` is not valid in storage.
    #[inline]
    pub fn link_front(&mut self, storage: &mut S, key: NexusKey) {
        let node = storage.get_node_mut(key).expect("invalid key");
        node.next = self.head;
        node.prev = NexusKey::NONE;

        if self.head.is_some() {
            // Safety: head is valid when is_some()
            unsafe { storage.get_node_unchecked_mut(self.head) }.prev = key;
        } else {
            self.tail = key;
        }

        self.head = key;
        self.len += 1;
    }

    /// Links an existing node after another node.
    ///
    /// # Panics
    ///
    /// Panics if `after` or `key` is not valid in storage.
    #[inline]
    pub fn link_after(&mut self, storage: &mut S, after: NexusKey, key: NexusKey) {
        let next = storage.get_node(after).expect("invalid 'after' key").next;
        let node = storage.get_node_mut(key).expect("invalid key");
        node.prev = after;
        node.next = next;

        // Safety: after validated above
        unsafe { storage.get_node_unchecked_mut(after) }.next = key;

        if next.is_some() {
            // Safety: next is valid when is_some() (list invariant)
            unsafe { storage.get_node_unchecked_mut(next) }.prev = key;
        } else {
            self.tail = key;
        }

        self.len += 1;
    }

    /// Links an existing node before another node.
    ///
    /// # Panics
    ///
    /// Panics if `before` or `key` is not valid in storage.
    #[inline]
    pub fn link_before(&mut self, storage: &mut S, before: NexusKey, key: NexusKey) {
        let prev = storage.get_node(before).expect("invalid 'before' key").prev;
        let node = storage.get_node_mut(key).expect("invalid key");
        node.next = before;
        node.prev = prev;

        // Safety: before validated above
        unsafe { storage.get_node_unchecked_mut(before) }.prev = key;

        if prev.is_some() {
            // Safety: prev is valid when is_some() (list invariant)
            unsafe { storage.get_node_unchecked_mut(prev) }.next = key;
        } else {
            self.head = key;
        }

        self.len += 1;
    }

    /// Unlinks a node from the list without deallocating.
    ///
    /// The node remains in storage and can be linked to another list.
    /// Use with `link_back`/`link_front` to move nodes between lists.
    ///
    /// Returns `true` if the node was in the list.
    ///
    /// # Panics
    ///
    /// Panics if `key` is not valid in storage.
    #[inline]
    pub fn unlink(&mut self, storage: &mut S, key: NexusKey) -> bool {
        let node = storage.get_node(key).expect("invalid key");
        let prev = node.prev;
        let next = node.next;

        // Check if actually in a list (has links or is head/tail)
        let in_list = prev.is_some() || next.is_some() || self.head == key;
        if !in_list {
            return false;
        }

        if prev.is_some() {
            // Safety: prev is valid when is_some() (list invariant)
            unsafe { storage.get_node_unchecked_mut(prev) }.next = next;
        } else {
            self.head = next;
        }

        if next.is_some() {
            // Safety: next is valid when is_some() (list invariant)
            unsafe { storage.get_node_unchecked_mut(next) }.prev = prev;
        } else {
            self.tail = prev;
        }

        // Clear the removed node's links
        // Safety: key already validated
        let node = unsafe { storage.get_node_unchecked_mut(key) };
        node.prev = NexusKey::NONE;
        node.next = NexusKey::NONE;

        self.len -= 1;
        true
    }

    // ========================================================================
    // Access
    // ========================================================================

    /// Returns a reference to the element at the given key.
    #[inline]
    pub fn get<'a>(&'a self, storage: &'a S, key: NexusKey) -> Option<&'a T> {
        storage.get_node(key).map(|node| &node.data)
    }

    /// Returns a mutable reference to the element at the given key.
    #[inline]
    pub fn get_mut<'a>(&'a mut self, storage: &'a mut S, key: NexusKey) -> Option<&'a mut T> {
        storage.get_node_mut(key).map(|node| &mut node.data)
    }

    /// Returns a reference to the front element.
    #[inline]
    pub fn front<'a>(&'a self, storage: &'a S) -> Option<&'a T> {
        if self.head.is_none() {
            None
        } else {
            // Safety: head is valid when is_some()
            Some(unsafe { &storage.get_node_unchecked(self.head).data })
        }
    }

    /// Returns a mutable reference to the front element.
    #[inline]
    pub fn front_mut<'a>(&'a mut self, storage: &'a mut S) -> Option<&'a mut T> {
        if self.head.is_none() {
            None
        } else {
            // Safety: head is valid when is_some()
            Some(unsafe { &mut storage.get_node_unchecked_mut(self.head).data })
        }
    }

    /// Returns a reference to the back element.
    #[inline]
    pub fn back<'a>(&'a self, storage: &'a S) -> Option<&'a T> {
        if self.tail.is_none() {
            None
        } else {
            // Safety: tail is valid when is_some()
            Some(unsafe { &storage.get_node_unchecked(self.tail).data })
        }
    }

    /// Returns a mutable reference to the back element.
    #[inline]
    pub fn back_mut<'a>(&'a mut self, storage: &'a mut S) -> Option<&'a mut T> {
        if self.tail.is_none() {
            None
        } else {
            // Safety: tail is valid when is_some()
            Some(unsafe { &mut storage.get_node_unchecked_mut(self.tail).data })
        }
    }

    // ========================================================================
    // Bulk operations
    // ========================================================================

    /// Clears the list, removing all elements.
    ///
    /// This unlinks and deallocates all nodes.
    pub fn clear(&mut self, storage: &mut S) {
        let mut key = self.head;
        while key.is_some() {
            // Safety: key is valid (came from list traversal)
            let next = unsafe { storage.get_node_unchecked(key) }.next;
            storage.remove_node(key);
            key = next;
        }

        self.head = NexusKey::NONE;
        self.tail = NexusKey::NONE;
        self.len = 0;
    }

    /// Appends `other` to the end of this list.
    ///
    /// After this operation, `other` will be empty. This is O(1).
    #[inline]
    pub fn append(&mut self, storage: &mut S, other: &mut Self) {
        if other.is_empty() {
            return;
        }

        if self.is_empty() {
            self.head = other.head;
            self.tail = other.tail;
            self.len = other.len;
        } else {
            // Safety: self.tail and other.head are valid (non-empty lists)
            unsafe { storage.get_node_unchecked_mut(self.tail) }.next = other.head;
            unsafe { storage.get_node_unchecked_mut(other.head) }.prev = self.tail;
            self.tail = other.tail;
            self.len += other.len;
        }

        other.head = NexusKey::NONE;
        other.tail = NexusKey::NONE;
        other.len = 0;
    }

    /// Moves a node to the back of the list.
    ///
    /// More efficient than `unlink` + `link_back` for repositioning.
    /// Useful for LRU caches.
    ///
    /// # Panics
    ///
    /// Panics if `key` is not valid in storage.
    #[inline]
    pub fn move_to_back(&mut self, storage: &mut S, key: NexusKey) {
        // Already at back
        if self.tail == key {
            return;
        }

        let node = storage.get_node(key).expect("invalid key");
        let prev = node.prev;
        let next = node.next;

        // Unlink from current position
        if prev.is_some() {
            // Safety: prev is valid (list invariant)
            unsafe { storage.get_node_unchecked_mut(prev) }.next = next;
        } else {
            self.head = next;
        }

        if next.is_some() {
            // Safety: next is valid (list invariant)
            unsafe { storage.get_node_unchecked_mut(next) }.prev = prev;
        }
        // Note: next can't be NONE here since we already checked key != tail

        // Link at back
        // Safety: tail is valid (list is non-empty)
        unsafe { storage.get_node_unchecked_mut(self.tail) }.next = key;

        // Safety: key validated above
        let node = unsafe { storage.get_node_unchecked_mut(key) };
        node.prev = self.tail;
        node.next = NexusKey::NONE;

        self.tail = key;
    }

    /// Moves a node to the front of the list.
    ///
    /// More efficient than `unlink` + `link_front` for repositioning.
    ///
    /// # Panics
    ///
    /// Panics if `key` is not valid in storage.
    #[inline]
    pub fn move_to_front(&mut self, storage: &mut S, key: NexusKey) {
        // Already at front
        if self.head == key {
            return;
        }

        // Safety: key validated above
        let node = storage.get_node(key).expect("invalid key");
        let prev = node.prev;
        let next = node.next;

        // Unlink from current position
        if prev.is_some() {
            // Safety: prev is valid (list invariant)
            unsafe { storage.get_node_unchecked_mut(prev) }.next = next;
        }
        // Note: prev can't be NONE here since we already checked key != head

        if next.is_some() {
            // Safety: next is valid (list invariant)
            unsafe { storage.get_node_unchecked_mut(next) }.prev = prev;
        } else {
            self.tail = prev;
        }

        // Link at front
        // Safety: head is valid (list is non-empty)
        unsafe { storage.get_node_unchecked_mut(self.head) }.prev = key;

        // Safety: key validated above
        let node = unsafe { storage.get_node_unchecked_mut(key) };
        node.next = self.head;
        node.prev = NexusKey::NONE;

        self.head = key;
    }

    /// Splits the list at the given node.
    ///
    /// Returns a new list containing `key` and all nodes after it.
    /// `self` will contain all nodes before `key`.
    ///
    /// This is O(n) due to counting elements in the split portion.
    ///
    /// # Panics
    ///
    /// Panics if `key` is not valid in storage.
    #[inline]
    pub fn split_off(&mut self, storage: &mut S, key: NexusKey) -> Self {
        let prev = storage.get_node(key).expect("invalid key").prev;

        // Splitting at head = take everything
        if self.head == key {
            let other = Self {
                head: self.head,
                tail: self.tail,
                len: self.len,
                _marker: PhantomData,
            };
            self.head = NexusKey::NONE;
            self.tail = NexusKey::NONE;
            self.len = 0;
            return other;
        }

        // Count nodes in the split-off portion
        let mut count = 0;
        let mut curr = key;
        while curr.is_some() {
            count += 1;
            curr = unsafe { storage.get_node_unchecked(curr) }.next;
        }

        // Unlink at split point
        // Safety: prev is valid (key != head, so prev.is_some())
        unsafe { storage.get_node_unchecked_mut(prev) }.next = NexusKey::NONE;
        unsafe { storage.get_node_unchecked_mut(key) }.prev = NexusKey::NONE;

        let other = Self {
            head: key,
            tail: self.tail,
            len: count,
            _marker: PhantomData,
        };

        self.tail = prev;
        self.len -= count;

        other
    }

    // ========================================================================
    // Position checks
    // ========================================================================

    /// Returns `true` if the node is currently the head of this list.
    #[inline]
    pub fn is_head(&self, key: NexusKey) -> bool {
        self.head == key
    }

    /// Returns `true` if the node is currently the tail of this list.
    #[inline]
    pub fn is_tail(&self, key: NexusKey) -> bool {
        self.tail == key
    }

    // ========================================================================
    // Navigation
    // ========================================================================

    /// Returns the key of the next node after `key`.
    ///
    /// Returns `None` if `key` is the tail or invalid.
    #[inline]
    pub fn next_key(&self, storage: &S, key: NexusKey) -> Option<NexusKey> {
        let next = storage.get_node(key)?.next;
        if next.is_none() { None } else { Some(next) }
    }

    /// Returns the key of the previous node before `key`.
    ///
    /// Returns `None` if `key` is the head or invalid.
    #[inline]
    pub fn prev_key(&self, storage: &S, key: NexusKey) -> Option<NexusKey> {
        let prev = storage.get_node(key)?.prev;
        if prev.is_none() { None } else { Some(prev) }
    }

    // ========================================================================
    // Iteration
    // ========================================================================

    /// Returns an iterator over references to elements, front to back.
    #[inline]
    pub fn iter<'a>(&self, storage: &'a S) -> Iter<'a, T, S> {
        Iter {
            storage,
            front: self.head,
            back: self.tail,
            _marker: PhantomData,
        }
    }

    /// Returns an iterator over mutable references to elements, front to back.
    #[inline]
    pub fn iter_mut<'a>(&self, storage: &'a mut S) -> IterMut<'a, T, S> {
        IterMut {
            storage,
            front: self.head,
            back: self.tail,
            _marker: PhantomData,
        }
    }

    /// Returns an iterator over keys, front to back.
    ///
    /// Useful when you need both the key and the value, or when you
    /// plan to modify the list during iteration (collect keys first).
    #[inline]
    pub fn keys<'a>(&self, storage: &'a S) -> Keys<'a, T, S> {
        Keys {
            storage,
            front: self.head,
            back: self.tail,
            _marker: PhantomData,
        }
    }

    /// Clears the list, returning an iterator over removed elements.
    ///
    /// The list is empty after this call. Elements are deallocated from
    /// storage as the iterator is consumed.
    #[inline]
    pub fn drain<'a>(&'a mut self, storage: &'a mut S) -> Drain<'a, T, S> {
        let head = self.head;
        self.head = NexusKey::NONE;
        self.tail = NexusKey::NONE;
        self.len = 0;

        Drain {
            storage,
            current: head,
            _marker: PhantomData,
        }
    }

    /// Returns a cursor positioned at the front of the list.
    ///
    /// The cursor allows mutable access and removal during iteration.
    /// See [`Cursor`] for usage examples.
    #[inline]
    pub fn cursor_front<'a>(&'a mut self, storage: &'a mut S) -> Cursor<'a, T, S> {
        let head = self.head;
        Cursor {
            list: self,
            storage,
            current: head,
        }
    }

    /// Returns a cursor positioned at the back of the list.
    #[inline]
    pub fn cursor_back<'a>(&'a mut self, storage: &'a mut S) -> Cursor<'a, T, S> {
        let tail = self.tail;
        Cursor {
            list: self,
            storage,
            current: tail,
        }
    }
}

// =============================================================================
// Bounded storage impl - fallible insertion
// =============================================================================

impl<T, S> List<T, S>
where
    S: BoundedListStorageOps<T>,
{
    /// Pushes a value to the back of the list.
    ///
    /// Returns the key of the inserted element.
    ///
    /// # Errors
    ///
    /// Returns `Err(Full(value))` if storage is full.
    #[inline]
    pub fn try_push_back(&mut self, storage: &mut S, value: T) -> Result<NexusKey, Full<T>> {
        let key = storage.try_insert_node(ListNode::new(value))?;
        self.link_back(storage, key);
        Ok(key)
    }

    /// Pushes a value to the front of the list.
    ///
    /// Returns the key of the inserted element.
    ///
    /// # Errors
    ///
    /// Returns `Err(Full(value))` if storage is full.
    #[inline]
    pub fn try_push_front(&mut self, storage: &mut S, value: T) -> Result<NexusKey, Full<T>> {
        let key = storage.try_insert_node(ListNode::new(value))?;
        self.link_front(storage, key);
        Ok(key)
    }

    /// Inserts a value after an existing node.
    ///
    /// Returns the key of the inserted element.
    ///
    /// # Errors
    ///
    /// Returns `Err(Full(value))` if storage is full.
    ///
    /// # Panics
    ///
    /// Panics if `after` is not valid in storage (debug builds only).
    #[inline]
    pub fn try_insert_after(
        &mut self,
        storage: &mut S,
        after: NexusKey,
        value: T,
    ) -> Result<NexusKey, Full<T>> {
        let key = storage.try_insert_node(ListNode::new(value))?;
        self.link_after(storage, after, key);
        Ok(key)
    }

    /// Inserts a value before an existing node.
    ///
    /// Returns the key of the inserted element.
    ///
    /// # Errors
    ///
    /// Returns `Err(Full(value))` if storage is full.
    ///
    /// # Panics
    ///
    /// Panics if `before` is not valid in storage (debug builds only).
    #[inline]
    pub fn try_insert_before(
        &mut self,
        storage: &mut S,
        before: NexusKey,
        value: T,
    ) -> Result<NexusKey, Full<T>> {
        let key = storage.try_insert_node(ListNode::new(value))?;
        self.link_before(storage, before, key);
        Ok(key)
    }

    // =========================================================================
    // Entry-returning methods
    // =========================================================================

    /// Pushes a value to the back of the list, returning an entry handle.
    ///
    /// Like [`try_push_back`](List::try_push_back) but returns a [`ListEntry`]
    /// for direct value access without key lookups.
    ///
    /// # Errors
    ///
    /// Returns `Err(Full(value))` if storage is full.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_collections::{List, ListStorage};
    ///
    /// let mut storage: ListStorage<u64> = ListStorage::with_capacity(100);
    /// let mut list: List<u64, ListStorage<u64>> = List::new();
    ///
    /// let entry = list.try_push_back_entry(&mut storage, 42).unwrap();
    /// assert_eq!(*entry.get(), 42);
    /// ```
    #[inline]
    pub fn try_push_back_entry(
        &mut self,
        storage: &mut S,
        value: T,
    ) -> Result<ListEntry<T>, Full<T>> {
        let key = self.try_push_back(storage, value)?;
        Ok(storage.entry(key).expect("just inserted"))
    }

    /// Pushes a value to the front of the list, returning an entry handle.
    ///
    /// Like [`try_push_front`](List::try_push_front) but returns a [`ListEntry`]
    /// for direct value access without key lookups.
    ///
    /// # Errors
    ///
    /// Returns `Err(Full(value))` if storage is full.
    #[inline]
    pub fn try_push_front_entry(
        &mut self,
        storage: &mut S,
        value: T,
    ) -> Result<ListEntry<T>, Full<T>> {
        let key = self.try_push_front(storage, value)?;
        Ok(storage.entry(key).expect("just inserted"))
    }

    /// Pushes a value to the back with access to the entry before it exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full.
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
    /// let mut storage: ListStorage<Order> = ListStorage::with_capacity(100);
    /// let mut list: List<Order, ListStorage<Order>> = List::new();
    ///
    /// let entry = list.try_push_back_with(&mut storage, |e| Order {
    ///     id: 1,
    ///     self_key: e.key(),
    /// }).unwrap();
    ///
    /// assert_eq!(entry.get().self_key, entry.key());
    /// ```
    #[inline]
    pub fn try_push_back_with<F>(
        &mut self,
        storage: &mut S,
        f: F,
    ) -> Result<ListEntry<T>, nexus_slab::CapacityError>
    where
        F: FnOnce(ListEntry<T>) -> T,
    {
        let entry = storage.insert_with(f)?;
        let key = entry.key();
        self.link_back(storage, key);
        Ok(entry)
    }

    /// Pushes a value to the front with access to the entry before it exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    ///
    /// # Errors
    ///
    /// Returns `Err(CapacityError)` if storage is full.
    #[inline]
    pub fn try_push_front_with<F>(
        &mut self,
        storage: &mut S,
        f: F,
    ) -> Result<ListEntry<T>, nexus_slab::CapacityError>
    where
        F: FnOnce(ListEntry<T>) -> T,
    {
        let entry = storage.insert_with(f)?;
        let key = entry.key();
        self.link_front(storage, key);
        Ok(entry)
    }
}

// =============================================================================
// Unbounded storage impl - infallible insertion
// =============================================================================

impl<T, S> List<T, S>
where
    S: GrowableListStorageOps<T>,
{
    /// Pushes a value to the back of the list.
    ///
    /// Returns the key of the inserted element.
    #[inline]
    pub fn push_back(&mut self, storage: &mut S, value: T) -> NexusKey {
        let key = storage.insert_node(ListNode::new(value));
        self.link_back(storage, key);
        key
    }

    /// Pushes a value to the front of the list.
    ///
    /// Returns the key of the inserted element.
    #[inline]
    pub fn push_front(&mut self, storage: &mut S, value: T) -> NexusKey {
        let key = storage.insert_node(ListNode::new(value));
        self.link_front(storage, key);
        key
    }

    /// Inserts a value after an existing node.
    ///
    /// Returns the key of the inserted element.
    ///
    /// # Panics
    ///
    /// Panics if `after` is not valid in storage (debug builds only).
    #[inline]
    pub fn insert_after(&mut self, storage: &mut S, after: NexusKey, value: T) -> NexusKey {
        let key = storage.insert_node(ListNode::new(value));
        self.link_after(storage, after, key);
        key
    }

    /// Inserts a value before an existing node.
    ///
    /// Returns the key of the inserted element.
    ///
    /// # Panics
    ///
    /// Panics if `before` is not valid in storage (debug builds only).
    #[inline]
    pub fn insert_before(&mut self, storage: &mut S, before: NexusKey, value: T) -> NexusKey {
        let key = storage.insert_node(ListNode::new(value));
        self.link_before(storage, before, key);
        key
    }

    // =========================================================================
    // Entry-returning methods
    // =========================================================================

    /// Pushes a value to the back of the list, returning an entry handle.
    ///
    /// Like [`push_back`](List::push_back) but returns a [`ListEntry`]
    /// for direct value access without key lookups.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_collections::{List, GrowableListStorage};
    ///
    /// let mut storage: GrowableListStorage<u64> = GrowableListStorage::new();
    /// let mut list: List<u64, GrowableListStorage<u64>> = List::new();
    ///
    /// let entry = list.push_back_entry(&mut storage, 42);
    /// assert_eq!(*entry.get(), 42);
    /// ```
    #[inline]
    pub fn push_back_entry(&mut self, storage: &mut S, value: T) -> ListEntry<T> {
        let key = self.push_back(storage, value);
        storage.entry(key).expect("just inserted")
    }

    /// Pushes a value to the front of the list, returning an entry handle.
    ///
    /// Like [`push_front`](List::push_front) but returns a [`ListEntry`]
    /// for direct value access without key lookups.
    #[inline]
    pub fn push_front_entry(&mut self, storage: &mut S, value: T) -> ListEntry<T> {
        let key = self.push_front(storage, value);
        storage.entry(key).expect("just inserted")
    }

    /// Pushes a value to the back with access to the entry before it exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_collections::{List, GrowableListStorage};
    /// use nexus_slab::Key as NexusKey;
    ///
    /// struct Order {
    ///     id: u64,
    ///     self_key: NexusKey,
    /// }
    ///
    /// let mut storage: GrowableListStorage<Order> = GrowableListStorage::new();
    /// let mut list: List<Order, GrowableListStorage<Order>> = List::new();
    ///
    /// let entry = list.push_back_with(&mut storage, |e| Order {
    ///     id: 1,
    ///     self_key: e.key(),
    /// });
    ///
    /// assert_eq!(entry.get().self_key, entry.key());
    /// ```
    #[inline]
    pub fn push_back_with<F>(&mut self, storage: &mut S, f: F) -> ListEntry<T>
    where
        F: FnOnce(ListEntry<T>) -> T,
    {
        let entry = storage.insert_with(f);
        let key = entry.key();
        self.link_back(storage, key);
        entry
    }

    /// Pushes a value to the front with access to the entry before it exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    #[inline]
    pub fn push_front_with<F>(&mut self, storage: &mut S, f: F) -> ListEntry<T>
    where
        F: FnOnce(ListEntry<T>) -> T,
    {
        let entry = storage.insert_with(f);
        let key = entry.key();
        self.link_front(storage, key);
        entry
    }
}

// =============================================================================
// Cursor
// =============================================================================

/// A cursor providing mutable access to list elements with removal capability.
///
/// Useful for matching engine workflows where you walk a queue,
/// modify elements, and conditionally remove them.
///
/// # Example
///
/// ```
/// use nexus_collections::{ListStorage, List};
///
/// #[derive(Debug)]
/// struct Order { qty: u64 }
///
/// let mut storage: ListStorage<Order> = ListStorage::with_capacity(100);
/// let mut queue: List<Order, ListStorage<Order>> = List::new();
///
/// queue.try_push_back(&mut storage, Order { qty: 100 }).unwrap();
/// queue.try_push_back(&mut storage, Order { qty: 50 }).unwrap();
///
/// let mut incoming_qty = 120u64;
/// let mut cursor = queue.cursor_front(&mut storage);
///
/// while let Some(resting) = cursor.current_mut() {
///     let fill = incoming_qty.min(resting.qty);
///     resting.qty -= fill;
///     incoming_qty -= fill;
///
///     if resting.qty == 0 {
///         cursor.remove_current(); // Removes and advances
///     } else {
///         cursor.move_next();
///     }
///
///     if incoming_qty == 0 {
///         break;
///     }
/// }
/// ```
pub struct Cursor<'a, T, S>
where
    S: ListStorageOps<T>,
{
    list: &'a mut List<T, S>,
    storage: &'a mut S,
    current: NexusKey,
}

impl<T, S> Cursor<'_, T, S>
where
    S: ListStorageOps<T>,
{
    /// Returns a reference to the current element.
    ///
    /// Returns `None` if the cursor is exhausted (past end of list).
    #[inline]
    pub fn current(&self) -> Option<&T> {
        if self.current.is_none() {
            None
        } else {
            // Safety: current is valid when not NONE
            Some(unsafe { &self.storage.get_node_unchecked(self.current).data })
        }
    }

    /// Returns a mutable reference to the current element.
    ///
    /// Returns `None` if the cursor is exhausted (past end of list).
    #[inline]
    pub fn current_mut(&mut self) -> Option<&mut T> {
        if self.current.is_none() {
            None
        } else {
            // Safety: current is valid when not NONE
            Some(unsafe { &mut self.storage.get_node_unchecked_mut(self.current).data })
        }
    }

    /// Returns the key of the current element.
    ///
    /// Returns `None` if the cursor is exhausted.
    #[inline]
    pub fn key(&self) -> Option<NexusKey> {
        if self.current.is_none() {
            None
        } else {
            Some(self.current)
        }
    }

    /// Advances the cursor to the next element.
    ///
    /// If already at end, cursor remains exhausted.
    #[inline]
    pub fn move_next(&mut self) {
        if self.current.is_some() {
            // Safety: current is valid when not NONE
            self.current = unsafe { self.storage.get_node_unchecked(self.current) }.next;
        }
    }

    /// Moves the cursor to the previous element.
    ///
    /// If already at start, cursor remains at start.
    #[inline]
    pub fn move_prev(&mut self) {
        if self.current.is_some() {
            // Safety: current is valid when not NONE
            self.current = unsafe { self.storage.get_node_unchecked(self.current) }.prev;
        }
    }

    /// Removes the current element and advances to the next.
    ///
    /// Returns the removed value, or `None` if cursor is exhausted.
    /// After removal, cursor points to what was the next element.
    #[inline]
    pub fn remove_current(&mut self) -> Option<T> {
        if self.current.is_none() {
            return None;
        }

        let key = self.current;
        // Safety: current is valid (cursor invariant)
        let next = unsafe { self.storage.get_node_unchecked(key) }.next;

        self.list.unlink(self.storage, key);
        self.current = next;

        // Safety: key was valid, we just unlinked it
        let node = unsafe { self.storage.remove_node_unchecked(key) };
        Some(node.data)
    }

    /// Returns `true` if the cursor is exhausted (no current element).
    #[inline]
    pub fn is_exhausted(&self) -> bool {
        self.current.is_none()
    }

    /// Peeks at the next element without advancing.
    ///
    /// Returns `None` if at end or cursor is exhausted.
    #[inline]
    pub fn peek_next(&self) -> Option<&T> {
        if self.current.is_none() {
            return None;
        }
        // Safety: current is valid
        let next = unsafe { self.storage.get_node_unchecked(self.current) }.next;
        if next.is_none() {
            None
        } else {
            // Safety: next is valid when not NONE
            Some(unsafe { &self.storage.get_node_unchecked(next).data })
        }
    }
}

// =============================================================================
// Iterators
// =============================================================================

/// Iterator over references to list elements.
pub struct Iter<'a, T, S> {
    storage: &'a S,
    front: NexusKey,
    back: NexusKey,
    _marker: PhantomData<T>,
}

impl<'a, T: 'a, S: ListStorageOps<T>> Iterator for Iter<'a, T, S> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.front.is_none() {
            return None;
        }

        // Safety: list invariants guarantee front is valid
        let node = unsafe { self.storage.get_node_unchecked(self.front) };

        // Check if we've met in the middle
        if self.front == self.back {
            self.front = NexusKey::NONE;
            self.back = NexusKey::NONE;
        } else {
            self.front = node.next;
        }

        Some(&node.data)
    }
}

impl<'a, T: 'a, S: ListStorageOps<T>> DoubleEndedIterator for Iter<'a, T, S> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.back.is_none() {
            return None;
        }

        // Safety: list invariants guarantee back is valid
        let node = unsafe { self.storage.get_node_unchecked(self.back) };

        // Check if we've met in the middle
        if self.front == self.back {
            self.front = NexusKey::NONE;
            self.back = NexusKey::NONE;
        } else {
            self.back = node.prev;
        }

        Some(&node.data)
    }
}

/// Iterator over mutable references to list elements.
pub struct IterMut<'a, T, S> {
    storage: &'a mut S,
    front: NexusKey,
    back: NexusKey,
    _marker: PhantomData<T>,
}

impl<'a, T: 'a, S: ListStorageOps<T>> Iterator for IterMut<'a, T, S> {
    type Item = &'a mut T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.front.is_none() {
            return None;
        }

        // Safety: list invariants guarantee front is valid
        let node = unsafe { self.storage.get_node_unchecked_mut(self.front) };

        // Check if we've met in the middle
        if self.front == self.back {
            self.front = NexusKey::NONE;
            self.back = NexusKey::NONE;
        } else {
            self.front = node.next;
        }

        // Extend lifetime - safe because we visit each node exactly once
        // The raw pointer dance is needed for lifetime extension
        #[allow(clippy::deref_addrof)]
        Some(unsafe { &mut *(&raw mut node.data) })
    }
}

impl<'a, T: 'a, S: ListStorageOps<T>> DoubleEndedIterator for IterMut<'a, T, S> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.back.is_none() {
            return None;
        }

        // Safety: list invariants guarantee back is valid
        let node = unsafe { self.storage.get_node_unchecked_mut(self.back) };

        // Check if we've met in the middle
        if self.front == self.back {
            self.front = NexusKey::NONE;
            self.back = NexusKey::NONE;
        } else {
            self.back = node.prev;
        }

        // Extend lifetime - safe because we visit each node exactly once
        // The raw pointer dance is needed for lifetime extension
        #[allow(clippy::deref_addrof)]
        Some(unsafe { &mut *(&raw mut node.data) })
    }
}

/// Iterator over keys in the list.
pub struct Keys<'a, T, S> {
    storage: &'a S,
    front: NexusKey,
    back: NexusKey,
    _marker: PhantomData<T>,
}

impl<T, S: ListStorageOps<T>> Iterator for Keys<'_, T, S> {
    type Item = NexusKey;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.front.is_none() {
            return None;
        }

        let key = self.front;
        // Safety: list invariants guarantee front is valid
        let node = unsafe { self.storage.get_node_unchecked(self.front) };

        if self.front == self.back {
            self.front = NexusKey::NONE;
            self.back = NexusKey::NONE;
        } else {
            self.front = node.next;
        }

        Some(key)
    }
}

impl<T, S: ListStorageOps<T>> DoubleEndedIterator for Keys<'_, T, S> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.back.is_none() {
            return None;
        }

        let key = self.back;
        // Safety: list invariants guarantee back is valid
        let node = unsafe { self.storage.get_node_unchecked(self.back) };

        if self.front == self.back {
            self.front = NexusKey::NONE;
            self.back = NexusKey::NONE;
        } else {
            self.back = node.prev;
        }

        Some(key)
    }
}

/// Iterator that removes and returns elements from a list.
pub struct Drain<'a, T, S>
where
    S: ListStorageOps<T>,
{
    storage: &'a mut S,
    current: NexusKey,
    _marker: PhantomData<T>,
}

impl<T, S> Iterator for Drain<'_, T, S>
where
    S: ListStorageOps<T>,
{
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_none() {
            return None;
        }

        let key = self.current;
        // Safety: current came from list traversal, must be valid
        self.current = unsafe { self.storage.get_node_unchecked(key) }.next;
        self.storage.remove_node(key).map(|node| node.data)
    }
}

impl<T, S> Drop for Drain<'_, T, S>
where
    S: ListStorageOps<T>,
{
    fn drop(&mut self) {
        // Exhaust remaining elements to ensure cleanup
        for _ in self.by_ref() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GrowableListStorage, ListStorage};

    #[test]
    fn new_list_is_empty() {
        let list: List<u64, ListStorage<u64>> = List::new();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        assert!(list.front_key().is_none());
        assert!(list.back_key().is_none());
    }

    #[test]
    fn try_push_back_single() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();

        assert_eq!(list.len(), 1);
        assert_eq!(list.front_key(), Some(a));
        assert_eq!(list.back_key(), Some(a));
        assert_eq!(list.get(&storage, a), Some(&1));
        assert!(list.front(&storage).is_some_and(|&front| front == 1));
        assert!(list.back(&storage).is_some_and(|&back| back == 1));
    }

    #[test]
    fn try_push_back_multiple() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();
        let _b = list.try_push_back(&mut storage, 2).unwrap();
        let c = list.try_push_back(&mut storage, 3).unwrap();

        assert_eq!(list.len(), 3);
        assert_eq!(list.front_key(), Some(a));
        assert_eq!(list.back_key(), Some(c));

        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn try_push_front_multiple() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_front(&mut storage, 1).unwrap();
        let _b = list.try_push_front(&mut storage, 2).unwrap();
        let c = list.try_push_front(&mut storage, 3).unwrap();

        assert_eq!(list.len(), 3);
        assert_eq!(list.front_key(), Some(c));
        assert_eq!(list.back_key(), Some(a));

        // Order should be 3, 2, 1
        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![3, 2, 1]);
    }

    #[test]
    fn pop_front() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        assert_eq!(list.pop_front(&mut storage), Some(1));
        assert_eq!(list.len(), 2);

        assert_eq!(list.pop_front(&mut storage), Some(2));
        assert_eq!(list.pop_front(&mut storage), Some(3));
        assert_eq!(list.pop_front(&mut storage), None);
        assert!(list.is_empty());
    }

    #[test]
    fn pop_back() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();

        assert_eq!(list.pop_back(&mut storage), Some(2));
        assert_eq!(list.len(), 1);

        assert_eq!(list.pop_back(&mut storage), Some(1));
        assert!(list.is_empty());
    }

    #[test]
    fn remove_middle() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let _a = list.try_push_back(&mut storage, 1).unwrap();
        let b = list.try_push_back(&mut storage, 2).unwrap();
        let _c = list.try_push_back(&mut storage, 3).unwrap();

        assert_eq!(list.remove(&mut storage, b), Some(2));
        assert_eq!(list.len(), 2);

        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![1, 3]);
    }

    #[test]
    fn unlink_and_link_to_another_list() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list_a: List<u64, _> = List::new();
        let mut list_b: List<u64, _> = List::new();

        let key = list_a.try_push_back(&mut storage, 42).unwrap();
        list_a.try_push_back(&mut storage, 99).unwrap();

        // Move key to list_b
        assert!(list_a.unlink(&mut storage, key));
        list_b.link_back(&mut storage, key);

        assert_eq!(list_a.len(), 1);
        assert_eq!(list_b.len(), 1);
        assert_eq!(list_b.get(&storage, key), Some(&42));

        // Original key still works
        assert_eq!(storage.get_node(key).map(|n| &n.data), Some(&42));
    }

    #[test]
    fn unlink_not_in_list() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let key = list.try_push_back(&mut storage, 1).unwrap();
        list.unlink(&mut storage, key);

        // Second unlink should return false
        assert!(!list.unlink(&mut storage, key));
    }

    #[test]
    fn get_and_get_mut() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 10).unwrap();

        assert_eq!(list.get(&storage, a), Some(&10));

        *list.get_mut(&mut storage, a).unwrap() = 20;
        assert_eq!(list.get(&storage, a), Some(&20));
    }

    #[test]
    fn front_and_back() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        assert!(list.front(&storage).is_none());
        assert!(list.back(&storage).is_none());

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        assert_eq!(list.front(&storage), Some(&1));
        assert_eq!(list.back(&storage), Some(&3));
    }

    #[test]
    fn try_insert_after() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();
        let _c = list.try_push_back(&mut storage, 3).unwrap();
        let _b = list.try_insert_after(&mut storage, a, 2).unwrap();

        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn try_insert_before() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let _a = list.try_push_back(&mut storage, 1).unwrap();
        let c = list.try_push_back(&mut storage, 3).unwrap();
        let _b = list.try_insert_before(&mut storage, c, 2).unwrap();

        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn clear() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        list.clear(&mut storage);

        assert!(list.is_empty());
        assert!(list.front_key().is_none());
        assert!(list.back_key().is_none());
    }

    #[test]
    fn append() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list1: List<u64, _> = List::new();
        let mut list2: List<u64, _> = List::new();

        list1.try_push_back(&mut storage, 1).unwrap();
        list1.try_push_back(&mut storage, 2).unwrap();
        list2.try_push_back(&mut storage, 3).unwrap();
        list2.try_push_back(&mut storage, 4).unwrap();

        list1.append(&mut storage, &mut list2);

        assert_eq!(list1.len(), 4);
        assert!(list2.is_empty());

        let values: Vec<_> = list1.iter(&storage).copied().collect();
        assert_eq!(values, vec![1, 2, 3, 4]);
    }

    #[test]
    fn move_to_back() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        list.move_to_back(&mut storage, a);

        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![2, 3, 1]);
    }

    #[test]
    fn move_to_front() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        let c = list.try_push_back(&mut storage, 3).unwrap();

        list.move_to_front(&mut storage, c);

        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![3, 1, 2]);
    }

    #[test]
    fn split_off() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        let b = list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        let tail = list.split_off(&mut storage, b);

        assert_eq!(list.len(), 1);
        let values1: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values1, vec![1]);

        assert_eq!(tail.len(), 2);
        let values2: Vec<_> = tail.iter(&storage).copied().collect();
        assert_eq!(values2, vec![2, 3]);
    }

    #[test]
    fn is_head_and_is_tail() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();
        let b = list.try_push_back(&mut storage, 2).unwrap();
        let c = list.try_push_back(&mut storage, 3).unwrap();

        assert!(list.is_head(a));
        assert!(!list.is_head(b));
        assert!(!list.is_head(c));

        assert!(!list.is_tail(a));
        assert!(!list.is_tail(b));
        assert!(list.is_tail(c));
    }

    #[test]
    fn iter_empty() {
        let storage = ListStorage::with_capacity(16);
        let list: List<u64, _> = List::new();

        assert_eq!(list.iter(&storage).count(), 0);
    }

    #[test]
    fn iter_mut() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        for val in list.iter_mut(&mut storage) {
            *val *= 10;
        }

        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![10, 20, 30]);
    }

    #[test]
    fn keys_iterator() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();
        let b = list.try_push_back(&mut storage, 2).unwrap();
        let c = list.try_push_back(&mut storage, 3).unwrap();

        let keys: Vec<_> = list.keys(&storage).collect();
        assert_eq!(keys, vec![a, b, c]);
    }

    #[test]
    fn storage_reuse_after_remove() {
        let mut storage = ListStorage::with_capacity(4);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();
        let _b = list.try_push_back(&mut storage, 2).unwrap();

        list.remove(&mut storage, a);

        // Should be able to insert again
        let c = list.try_push_back(&mut storage, 3).unwrap();
        assert_eq!(c, a); // Reused slot

        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![2, 3]);
    }

    // ============================================================================
    // Cursor tests
    // ============================================================================

    #[test]
    fn cursor_basic_navigation() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();
        let b = list.try_push_back(&mut storage, 2).unwrap();
        let c = list.try_push_back(&mut storage, 3).unwrap();

        let mut cursor = list.cursor_front(&mut storage);

        assert_eq!(cursor.key(), Some(a));
        assert_eq!(cursor.current(), Some(&1));

        cursor.move_next();
        assert_eq!(cursor.key(), Some(b));
        assert_eq!(cursor.current(), Some(&2));

        cursor.move_next();
        assert_eq!(cursor.key(), Some(c));
        assert_eq!(cursor.current(), Some(&3));

        cursor.move_next();
        assert!(cursor.is_exhausted());
        assert_eq!(cursor.current(), None);
    }

    #[test]
    fn cursor_move_prev() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();
        let b = list.try_push_back(&mut storage, 2).unwrap();
        let c = list.try_push_back(&mut storage, 3).unwrap();

        let mut cursor = list.cursor_back(&mut storage);

        assert_eq!(cursor.key(), Some(c));
        cursor.move_prev();
        assert_eq!(cursor.key(), Some(b));
        cursor.move_prev();
        assert_eq!(cursor.key(), Some(a));
        cursor.move_prev();
        assert!(cursor.is_exhausted());
    }

    #[test]
    fn cursor_current_mut() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 10).unwrap();
        list.try_push_back(&mut storage, 20).unwrap();

        let mut cursor = list.cursor_front(&mut storage);
        *cursor.current_mut().unwrap() = 100;
        cursor.move_next();
        *cursor.current_mut().unwrap() = 200;

        drop(cursor);

        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![100, 200]);
    }

    #[test]
    fn cursor_remove_current() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        let b = list.try_push_back(&mut storage, 2).unwrap();
        let c = list.try_push_back(&mut storage, 3).unwrap();

        let mut cursor = list.cursor_front(&mut storage);

        // Remove first element
        assert_eq!(cursor.remove_current(), Some(1));
        assert_eq!(cursor.key(), Some(b)); // Advanced to b

        // Remove second element (now first)
        assert_eq!(cursor.remove_current(), Some(2));
        assert_eq!(cursor.key(), Some(c)); // Advanced to c

        // Remove last element
        assert_eq!(cursor.remove_current(), Some(3));
        assert!(cursor.is_exhausted());

        assert!(list.is_empty());
    }

    #[test]
    fn cursor_remove_middle() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        let mut cursor = list.cursor_front(&mut storage);
        cursor.move_next(); // Move to middle

        assert_eq!(cursor.remove_current(), Some(2));
        assert_eq!(cursor.current(), Some(&3)); // Now at what was third

        drop(cursor);

        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![1, 3]);
    }

    #[test]
    fn cursor_peek_next() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        let mut cursor = list.cursor_front(&mut storage);

        assert_eq!(cursor.current(), Some(&1));
        assert_eq!(cursor.peek_next(), Some(&2));

        cursor.move_next();
        assert_eq!(cursor.current(), Some(&2));
        assert_eq!(cursor.peek_next(), Some(&3));

        cursor.move_next();
        assert_eq!(cursor.current(), Some(&3));
        assert_eq!(cursor.peek_next(), None); // At tail
    }

    #[test]
    fn cursor_empty_list() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let mut cursor = list.cursor_front(&mut storage);

        assert!(cursor.is_exhausted());
        assert_eq!(cursor.current(), None);
        assert_eq!(cursor.key(), None);
        assert_eq!(cursor.remove_current(), None);
    }

    #[test]
    fn cursor_single_element() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 42).unwrap();

        let mut cursor = list.cursor_front(&mut storage);

        assert_eq!(cursor.current(), Some(&42));
        assert_eq!(cursor.peek_next(), None);

        assert_eq!(cursor.remove_current(), Some(42));
        assert!(cursor.is_exhausted());
        assert!(list.is_empty());
    }

    #[test]
    fn cursor_matching_workflow() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        // Resting orders: 100, 50, 75
        list.try_push_back(&mut storage, 100).unwrap();
        list.try_push_back(&mut storage, 50).unwrap();
        list.try_push_back(&mut storage, 75).unwrap();

        // Incoming order wants to fill 120
        let mut remaining = 120u64;
        let mut cursor = list.cursor_front(&mut storage);

        while let Some(resting) = cursor.current_mut() {
            let fill = remaining.min(*resting);
            *resting -= fill;
            remaining -= fill;

            if *resting == 0 {
                cursor.remove_current();
            } else {
                cursor.move_next();
            }

            if remaining == 0 {
                break;
            }
        }

        assert_eq!(remaining, 0);
        drop(cursor);

        // First order (100) fully filled and removed
        // Second order (50) partially filled, 30 remaining
        let values: Vec<_> = list.iter(&storage).copied().collect();
        assert_eq!(values, vec![30, 75]);
    }

    // ============================================================================
    // Iterator tests
    // ============================================================================

    #[test]
    fn iter_rev() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        let values: Vec<_> = list.iter(&storage).rev().copied().collect();
        assert_eq!(values, vec![3, 2, 1]);
    }

    #[test]
    fn iter_double_ended() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();
        list.try_push_back(&mut storage, 4).unwrap();

        let mut iter = list.iter(&storage);
        assert_eq!(iter.next(), Some(&1));
        assert_eq!(iter.next_back(), Some(&4));
        assert_eq!(iter.next(), Some(&2));
        assert_eq!(iter.next_back(), Some(&3));
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next_back(), None);
    }

    #[test]
    fn keys_rev() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();
        let b = list.try_push_back(&mut storage, 2).unwrap();
        let c = list.try_push_back(&mut storage, 3).unwrap();

        let keys: Vec<_> = list.keys(&storage).rev().collect();
        assert_eq!(keys, vec![c, b, a]);
    }

    #[test]
    fn drain_all() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        let values: Vec<_> = list.drain(&mut storage).collect();
        assert_eq!(values, vec![1, 2, 3]);

        assert!(list.is_empty());
        assert!(storage.is_empty());
    }

    #[test]
    fn drain_partial_then_drop() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();
        list.try_push_back(&mut storage, 3).unwrap();

        {
            let mut drain = list.drain(&mut storage);
            assert_eq!(drain.next(), Some(1));
            // Drop drain without consuming all
        }

        // Storage should still be cleaned up
        assert!(list.is_empty());
        assert!(storage.is_empty());
    }

    #[test]
    fn next_key_and_prev_key() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let a = list.try_push_back(&mut storage, 1).unwrap();
        let b = list.try_push_back(&mut storage, 2).unwrap();
        let c = list.try_push_back(&mut storage, 3).unwrap();

        assert_eq!(list.next_key(&storage, a), Some(b));
        assert_eq!(list.next_key(&storage, b), Some(c));
        assert_eq!(list.next_key(&storage, c), None);

        assert_eq!(list.prev_key(&storage, a), None);
        assert_eq!(list.prev_key(&storage, b), Some(a));
        assert_eq!(list.prev_key(&storage, c), Some(b));
    }

    #[test]
    fn try_push_back_full_error() {
        let mut storage = ListStorage::with_capacity(2);
        let mut list: List<u64, _> = List::new();

        list.try_push_back(&mut storage, 1).unwrap();
        list.try_push_back(&mut storage, 2).unwrap();

        // Should fail - storage is full
        let result = list.try_push_back(&mut storage, 3);
        assert!(result.is_err());

        // Value should be returned
        let Full(val) = result.unwrap_err();
        assert_eq!(val, 3);
    }

    // ========================================================================
    // Entry API Tests
    // ========================================================================

    #[test]
    fn try_push_back_entry_basic() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let entry = list.try_push_back_entry(&mut storage, 42).unwrap();

        assert_eq!(list.len(), 1);
        assert_eq!(*entry.get(), 42);
        assert!(entry.is_valid());
    }

    #[test]
    fn try_push_front_entry_basic() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        list.try_push_back_entry(&mut storage, 1).unwrap();
        let entry = list.try_push_front_entry(&mut storage, 2).unwrap();

        assert_eq!(list.len(), 2);
        assert_eq!(*entry.get(), 2);
        assert_eq!(list.front(&storage), Some(&2));
    }

    #[test]
    fn entry_clone() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let entry = list.try_push_back_entry(&mut storage, 42).unwrap();
        let cloned = entry.clone();

        assert_eq!(entry.key(), cloned.key());
        assert_eq!(*cloned.get(), 42);
    }

    #[test]
    fn entry_mutation() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let entry = list.try_push_back_entry(&mut storage, 42).unwrap();
        *entry.get_mut() = 100;

        assert_eq!(*entry.get(), 100);
        assert_eq!(list.front(&storage), Some(&100));
    }

    #[test]
    fn push_back_entry_growable() {
        let mut storage = GrowableListStorage::new();
        let mut list: List<u64, _> = List::new();

        let entry = list.push_back_entry(&mut storage, 42);

        assert_eq!(list.len(), 1);
        assert_eq!(*entry.get(), 42);
    }

    #[test]
    fn push_front_entry_growable() {
        let mut storage = GrowableListStorage::new();
        let mut list: List<u64, _> = List::new();

        list.push_back_entry(&mut storage, 1);
        let entry = list.push_front_entry(&mut storage, 2);

        assert_eq!(list.len(), 2);
        assert_eq!(*entry.get(), 2);
        assert_eq!(list.front(&storage), Some(&2));
    }

    #[test]
    fn link_from_vacant() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        // Insert directly to storage via vacant
        let vacant = storage.vacant().unwrap();
        let entry = vacant.insert(42);

        // Not in list yet
        assert!(list.is_empty());

        // Link into list
        list.link_back(&mut storage, entry.key());

        assert_eq!(list.len(), 1);
        assert_eq!(list.front(&storage), Some(&42));
    }

    #[test]
    fn link_from_insert_with() {
        struct Order {
            id: u64,
            self_key: NexusKey,
        }

        let mut storage: ListStorage<Order> = ListStorage::with_capacity(16);
        let mut list: List<Order, _> = List::new();

        // Create self-referential item
        let entry = storage
            .insert_with(|e| Order {
                id: 42,
                self_key: e.key(),
            })
            .unwrap();

        // Verify self-reference
        assert_eq!(entry.get().self_key, entry.key());

        // Link into list
        list.link_back(&mut storage, entry.key());

        assert_eq!(list.len(), 1);
        assert_eq!(list.front(&storage).unwrap().id, 42);
    }

    #[test]
    fn try_push_back_with_self_referential() {
        struct Order {
            id: u64,
            self_key: NexusKey,
        }

        let mut storage: ListStorage<Order> = ListStorage::with_capacity(16);
        let mut list: List<Order, _> = List::new();

        let entry = list
            .try_push_back_with(&mut storage, |e| Order {
                id: 100,
                self_key: e.key(),
            })
            .unwrap();

        // Verify self-reference and list membership
        assert_eq!(entry.get().self_key, entry.key());
        assert_eq!(list.len(), 1);
        assert_eq!(list.front(&storage).unwrap().id, 100);
    }

    #[test]
    fn push_back_with_self_referential() {
        #[allow(dead_code)]
        struct Order {
            id: u64,
            self_key: NexusKey,
        }

        let mut storage: GrowableListStorage<Order> = GrowableListStorage::new();
        let mut list: List<Order, _> = List::new();

        let entry = list.push_back_with(&mut storage, |e| Order {
            id: 100,
            self_key: e.key(),
        });

        assert_eq!(entry.get().self_key, entry.key());
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn entry_becomes_invalid_after_remove() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let entry = list.try_push_back_entry(&mut storage, 42).unwrap();
        assert!(entry.is_valid());

        list.remove(&mut storage, entry.key());
        assert!(!entry.is_valid());
        assert!(entry.try_get().is_none());
    }

    #[test]
    fn entry_try_methods() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let entry = list.try_push_back_entry(&mut storage, 42).unwrap();

        // try_get works
        assert_eq!(*entry.try_get().unwrap(), 42);

        // try_get_mut works
        *entry.try_get_mut().unwrap() = 100;
        assert_eq!(*entry.get(), 100);
    }

    #[test]
    fn multiple_entries_order() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list: List<u64, _> = List::new();

        let e1 = list.try_push_back_entry(&mut storage, 1).unwrap();
        let e2 = list.try_push_back_entry(&mut storage, 2).unwrap();
        let e3 = list.try_push_back_entry(&mut storage, 3).unwrap();

        // Entries maintain their values
        assert_eq!(*e1.get(), 1);
        assert_eq!(*e2.get(), 2);
        assert_eq!(*e3.get(), 3);

        // List order is correct
        assert_eq!(list.pop_front(&mut storage), Some(1));
        assert_eq!(list.pop_front(&mut storage), Some(2));
        assert_eq!(list.pop_front(&mut storage), Some(3));
    }

    #[test]
    fn move_between_lists_with_entry() {
        let mut storage = ListStorage::with_capacity(16);
        let mut list_a: List<u64, _> = List::new();
        let mut list_b: List<u64, _> = List::new();

        let entry = list_a.try_push_back_entry(&mut storage, 42).unwrap();

        // Move to list_b - entry still valid
        list_a.unlink(&mut storage, entry.key());
        list_b.link_back(&mut storage, entry.key());

        assert!(list_a.is_empty());
        assert_eq!(list_b.len(), 1);
        assert_eq!(*entry.get(), 42);
    }
}

#[cfg(test)]
mod bench_boxed_storage {
    use super::*;
    use crate::ListStorage;
    use hdrhistogram::Histogram;

    #[inline]
    fn rdtscp() -> u64 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::x86_64::__rdtscp(&mut 0)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            std::time::Instant::now().elapsed().as_nanos() as u64
        }
    }

    fn print_histogram(name: &str, hist: &Histogram<u64>) {
        println!(
            "{:24} p50: {:4} cycles | p99: {:4} cycles | p999: {:5} cycles | min: {:4} | max: {:5}",
            name,
            hist.value_at_quantile(0.50),
            hist.value_at_quantile(0.99),
            hist.value_at_quantile(0.999),
            hist.min(),
            hist.max(),
        );
    }

    const WARMUP: usize = 10_000;
    const ITERATIONS: usize = 100_000;

    #[test]
    #[ignore]
    fn bench_list_try_push_back() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(ITERATIONS + WARMUP);
        let mut list: List<u64, ListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for i in 0..WARMUP {
            let _ = list.try_push_back(&mut storage, i as u64);
            let _ = list.pop_back(&mut storage);
        }

        for i in 0..ITERATIONS {
            let start = rdtscp();
            let _ = list.try_push_back(&mut storage, i as u64);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
            let _ = list.pop_back(&mut storage);
        }

        print_histogram("try_push_back", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_try_push_front() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(ITERATIONS + WARMUP);
        let mut list: List<u64, ListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for i in 0..WARMUP {
            let _ = list.try_push_front(&mut storage, i as u64);
            let _ = list.pop_front(&mut storage);
        }

        for i in 0..ITERATIONS {
            let start = rdtscp();
            let _ = list.try_push_front(&mut storage, i as u64);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
            let _ = list.pop_front(&mut storage);
        }

        print_histogram("try_push_front", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_pop_front() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(ITERATIONS + WARMUP);
        let mut list: List<u64, ListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let _ = list.try_push_back(&mut storage, 1);
            let _ = list.pop_front(&mut storage);
        }

        for i in 0..ITERATIONS {
            let _ = list.try_push_back(&mut storage, i as u64);
            let start = rdtscp();
            let _ = list.pop_front(&mut storage);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
        }

        print_histogram("pop_front", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_pop_back() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(ITERATIONS + WARMUP);
        let mut list: List<u64, ListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let _ = list.try_push_back(&mut storage, 1);
            let _ = list.pop_back(&mut storage);
        }

        for i in 0..ITERATIONS {
            let _ = list.try_push_back(&mut storage, i as u64);
            let start = rdtscp();
            let _ = list.pop_back(&mut storage);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
        }

        print_histogram("pop_back", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_get() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(1024);
        let mut list: List<u64, ListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        let mut keys = Vec::with_capacity(1000);
        for i in 0..1000 {
            keys.push(list.try_push_back(&mut storage, i as u64).unwrap());
        }

        let mid_key = keys[500];
        for _ in 0..WARMUP {
            std::hint::black_box(list.get(&storage, mid_key));
        }

        for _ in 0..ITERATIONS {
            let start = rdtscp();
            std::hint::black_box(list.get(&storage, mid_key));
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
        }

        print_histogram("get (middle)", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_remove_middle() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(16);
        let mut list: List<u64, ListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let a = list.try_push_back(&mut storage, 1).unwrap();
            let b = list.try_push_back(&mut storage, 2).unwrap();
            let c = list.try_push_back(&mut storage, 3).unwrap();
            let _ = list.remove(&mut storage, b);
            let _ = list.remove(&mut storage, a);
            let _ = list.remove(&mut storage, c);
        }

        for _ in 0..ITERATIONS {
            let a = list.try_push_back(&mut storage, 1).unwrap();
            let b = list.try_push_back(&mut storage, 2).unwrap();
            let c = list.try_push_back(&mut storage, 3).unwrap();

            let start = rdtscp();
            let _ = list.remove(&mut storage, b);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();

            let _ = list.remove(&mut storage, a);
            let _ = list.remove(&mut storage, c);
        }

        print_histogram("remove (middle)", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_unlink() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(16);
        let mut list: List<u64, ListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let key = list.try_push_back(&mut storage, 1).unwrap();
            list.unlink(&mut storage, key);
            storage.remove_node(key);
        }

        for _ in 0..ITERATIONS {
            let key = list.try_push_back(&mut storage, 1).unwrap();

            let start = rdtscp();
            list.unlink(&mut storage, key);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();

            storage.remove_node(key);
        }

        print_histogram("unlink", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_link_back() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(16);
        let mut list: List<u64, ListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let key = list.try_push_back(&mut storage, 1).unwrap();
            list.unlink(&mut storage, key);
            list.link_back(&mut storage, key);
            list.remove(&mut storage, key);
        }

        for _ in 0..ITERATIONS {
            let key = list.try_push_back(&mut storage, 1).unwrap();
            list.unlink(&mut storage, key);

            let start = rdtscp();
            list.link_back(&mut storage, key);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();

            list.remove(&mut storage, key);
        }

        print_histogram("link_back", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_move_to_back() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(1024);
        let mut list: List<u64, ListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        // Build a list of 100 elements
        let mut keys = Vec::with_capacity(100);
        for i in 0..100 {
            keys.push(list.try_push_back(&mut storage, i as u64).unwrap());
        }

        for _ in 0..WARMUP {
            list.move_to_back(&mut storage, keys[0]);
        }

        for i in 0..ITERATIONS {
            let key = keys[i % 100];
            let start = rdtscp();
            list.move_to_back(&mut storage, key);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
        }

        print_histogram("move_to_back", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_order_queue_workflow() {
        let mut storage: ListStorage<u64> = ListStorage::with_capacity(32);
        let mut queue_a: List<u64, ListStorage<u64>> = List::new();
        let mut queue_b: List<u64, ListStorage<u64>> = List::new();

        let mut hist_insert = Histogram::<u64>::new(3).unwrap();
        let mut hist_move = Histogram::<u64>::new(3).unwrap();
        let mut hist_cancel = Histogram::<u64>::new(3).unwrap();

        for i in 0..WARMUP {
            let key = queue_a.try_push_back(&mut storage, i as u64).unwrap();
            queue_a.unlink(&mut storage, key);
            queue_b.link_back(&mut storage, key);
            queue_b.remove(&mut storage, key);
        }

        for i in 0..ITERATIONS {
            let start = rdtscp();
            let key = queue_a.try_push_back(&mut storage, i as u64).unwrap();
            hist_insert.record(rdtscp() - start).unwrap();

            let start = rdtscp();
            queue_a.unlink(&mut storage, key);
            queue_b.link_back(&mut storage, key);
            hist_move.record(rdtscp() - start).unwrap();

            let start = rdtscp();
            queue_b.remove(&mut storage, key);
            hist_cancel.record(rdtscp() - start).unwrap();
        }

        println!("\n=== Order Queue Workflow (BoxedStorage) ===");
        print_histogram("insert (new order)", &hist_insert);
        print_histogram("move (price amend)", &hist_move);
        print_histogram("cancel", &hist_cancel);
    }

    #[test]
    #[ignore]
    fn bench_list_all() {
        println!("\n=== List Benchmarks (BoxedStorage) ===");
        println!(
            "Run with: cargo test --release bench_boxed_storage::bench_list_all -- --ignored --nocapture\n"
        );

        bench_list_try_push_back();
        bench_list_try_push_front();
        bench_list_pop_front();
        bench_list_pop_back();
        bench_list_get();
        bench_list_remove_middle();
        bench_list_unlink();
        bench_list_link_back();
        bench_list_move_to_back();

        println!();
        bench_list_order_queue_workflow();
    }
}

#[cfg(test)]
mod bench_nexus_slab_storage {
    use super::*;
    use crate::GrowableListStorage;
    use hdrhistogram::Histogram;

    #[inline]
    fn rdtscp() -> u64 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::x86_64::__rdtscp(&mut 0)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            std::time::Instant::now().elapsed().as_nanos() as u64
        }
    }

    fn print_histogram(name: &str, hist: &Histogram<u64>) {
        println!(
            "{:24} p50: {:4} cycles | p99: {:4} cycles | p999: {:5} cycles | min: {:4} | max: {:5}",
            name,
            hist.value_at_quantile(0.50),
            hist.value_at_quantile(0.99),
            hist.value_at_quantile(0.999),
            hist.min(),
            hist.max(),
        );
    }

    const WARMUP: usize = 10_000;
    const ITERATIONS: usize = 100_000;

    #[test]
    #[ignore]
    fn bench_list_push_back() {
        let mut storage: GrowableListStorage<u64> =
            GrowableListStorage::with_capacity(ITERATIONS + WARMUP);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for i in 0..WARMUP {
            let _ = list.push_back(&mut storage, i as u64);
            let _ = list.pop_back(&mut storage);
        }

        for i in 0..ITERATIONS {
            let start = rdtscp();
            let _ = list.push_back(&mut storage, i as u64);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
            let _ = list.pop_back(&mut storage);
        }

        print_histogram("push_back", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_push_front() {
        let mut storage: GrowableListStorage<u64> =
            GrowableListStorage::with_capacity(ITERATIONS + WARMUP);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for i in 0..WARMUP {
            let _ = list.push_front(&mut storage, i as u64);
            let _ = list.pop_front(&mut storage);
        }

        for i in 0..ITERATIONS {
            let start = rdtscp();
            let _ = list.push_front(&mut storage, i as u64);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
            let _ = list.pop_front(&mut storage);
        }

        print_histogram("push_front", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_pop_front() {
        let mut storage: GrowableListStorage<u64> =
            GrowableListStorage::with_capacity(ITERATIONS + WARMUP);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let _ = list.push_back(&mut storage, 1);
            let _ = list.pop_front(&mut storage);
        }

        for i in 0..ITERATIONS {
            let _ = list.push_back(&mut storage, i as u64);
            let start = rdtscp();
            let _ = list.pop_front(&mut storage);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
        }

        print_histogram("pop_front", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_pop_back() {
        let mut storage: GrowableListStorage<u64> =
            GrowableListStorage::with_capacity(ITERATIONS + WARMUP);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let _ = list.push_back(&mut storage, 1);
            let _ = list.pop_back(&mut storage);
        }

        for i in 0..ITERATIONS {
            let _ = list.push_back(&mut storage, i as u64);
            let start = rdtscp();
            let _ = list.pop_back(&mut storage);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
        }

        print_histogram("pop_back", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_get() {
        let mut storage: GrowableListStorage<u64> = GrowableListStorage::with_capacity(1024);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        let mut keys = Vec::with_capacity(1000);
        for i in 0..1000 {
            keys.push(list.push_back(&mut storage, i as u64));
        }

        let mid_key = keys[500];
        for _ in 0..WARMUP {
            std::hint::black_box(list.get(&storage, mid_key));
        }

        for _ in 0..ITERATIONS {
            let start = rdtscp();
            std::hint::black_box(list.get(&storage, mid_key));
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
        }

        print_histogram("get (middle)", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_remove_middle() {
        let mut storage: GrowableListStorage<u64> = GrowableListStorage::with_capacity(16);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let a = list.push_back(&mut storage, 1);
            let b = list.push_back(&mut storage, 2);
            let c = list.push_back(&mut storage, 3);
            let _ = list.remove(&mut storage, b);
            let _ = list.remove(&mut storage, a);
            let _ = list.remove(&mut storage, c);
        }

        for _ in 0..ITERATIONS {
            let a = list.push_back(&mut storage, 1);
            let b = list.push_back(&mut storage, 2);
            let c = list.push_back(&mut storage, 3);

            let start = rdtscp();
            let _ = list.remove(&mut storage, b);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();

            let _ = list.remove(&mut storage, a);
            let _ = list.remove(&mut storage, c);
        }

        print_histogram("remove (middle)", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_unlink() {
        let mut storage: GrowableListStorage<u64> = GrowableListStorage::with_capacity(16);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let key = list.push_back(&mut storage, 1);
            list.unlink(&mut storage, key);
            storage.remove_node(key);
        }

        for _ in 0..ITERATIONS {
            let key = list.push_back(&mut storage, 1);

            let start = rdtscp();
            list.unlink(&mut storage, key);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();

            storage.remove_node(key);
        }

        print_histogram("unlink", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_link_back() {
        let mut storage: GrowableListStorage<u64> = GrowableListStorage::with_capacity(16);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let key = list.push_back(&mut storage, 1);
            list.unlink(&mut storage, key);
            list.link_back(&mut storage, key);
            list.remove(&mut storage, key);
        }

        for _ in 0..ITERATIONS {
            let key = list.push_back(&mut storage, 1);
            list.unlink(&mut storage, key);

            let start = rdtscp();
            list.link_back(&mut storage, key);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();

            list.remove(&mut storage, key);
        }

        print_histogram("link_back", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_move_to_back() {
        let mut storage: GrowableListStorage<u64> = GrowableListStorage::with_capacity(1024);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        let mut keys = Vec::with_capacity(100);
        for i in 0..100 {
            keys.push(list.push_back(&mut storage, i as u64));
        }

        for _ in 0..WARMUP {
            list.move_to_back(&mut storage, keys[0]);
        }

        for i in 0..ITERATIONS {
            let key = keys[i % 100];
            let start = rdtscp();
            list.move_to_back(&mut storage, key);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
        }

        print_histogram("move_to_back", &hist);
    }

    #[test]
    #[ignore]
    fn bench_list_order_queue_workflow() {
        let mut storage: GrowableListStorage<u64> = GrowableListStorage::with_capacity(32);
        let mut queue_a: List<u64, GrowableListStorage<u64>> = List::new();
        let mut queue_b: List<u64, GrowableListStorage<u64>> = List::new();

        let mut hist_insert = Histogram::<u64>::new(3).unwrap();
        let mut hist_move = Histogram::<u64>::new(3).unwrap();
        let mut hist_cancel = Histogram::<u64>::new(3).unwrap();

        for i in 0..WARMUP {
            let key = queue_a.push_back(&mut storage, i as u64);
            queue_a.unlink(&mut storage, key);
            queue_b.link_back(&mut storage, key);
            queue_b.remove(&mut storage, key);
        }

        for i in 0..ITERATIONS {
            let start = rdtscp();
            let key = queue_a.push_back(&mut storage, i as u64);
            hist_insert.record(rdtscp() - start).unwrap();

            let start = rdtscp();
            queue_a.unlink(&mut storage, key);
            queue_b.link_back(&mut storage, key);
            hist_move.record(rdtscp() - start).unwrap();

            let start = rdtscp();
            queue_b.remove(&mut storage, key);
            hist_cancel.record(rdtscp() - start).unwrap();
        }

        println!("\n=== Order Queue Workflow (nexus_slab::Slab) ===");
        print_histogram("insert (new order)", &hist_insert);
        print_histogram("move (price amend)", &hist_move);
        print_histogram("cancel", &hist_cancel);
    }

    #[test]
    #[ignore]
    fn bench_list_all() {
        println!("\n=== List Benchmarks (nexus_slab::Slab) ===");
        println!(
            "Run with: cargo test --release --features nexus-slab bench_nexus_slab_storage::bench_list_all -- --ignored --nocapture\n"
        );

        bench_list_push_back();
        bench_list_push_front();
        bench_list_pop_front();
        bench_list_pop_back();
        bench_list_get();
        bench_list_remove_middle();
        bench_list_unlink();
        bench_list_link_back();
        bench_list_move_to_back();

        println!();
        bench_list_order_queue_workflow();

        println!("\n=== Entry API Comparison ===");
        bench_push_back_entry();
        bench_entry_get();
        bench_entry_get_mut();
    }

    // =========================================================================
    // Entry API Benchmarks
    // =========================================================================

    #[test]
    #[ignore]
    fn bench_push_back_entry() {
        let mut storage: GrowableListStorage<u64> =
            GrowableListStorage::with_capacity(ITERATIONS + WARMUP);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        for _ in 0..WARMUP {
            let _ = list.push_back_entry(&mut storage, 1);
            let _ = list.pop_front(&mut storage);
        }

        for i in 0..ITERATIONS {
            let start = rdtscp();
            let _ = list.push_back_entry(&mut storage, i as u64);
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
            let _ = list.pop_front(&mut storage);
        }

        print_histogram("push_back_entry", &hist);
    }

    #[test]
    #[ignore]
    fn bench_entry_get() {
        let mut storage: GrowableListStorage<u64> =
            GrowableListStorage::with_capacity(1024);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        // Build a list of 1000 elements and collect entries
        let mut entries = Vec::with_capacity(1000);
        for i in 0..1000 {
            entries.push(list.push_back_entry(&mut storage, i as u64));
        }

        // Warmup
        for _ in 0..WARMUP {
            for entry in &entries {
                std::hint::black_box(entry.get());
            }
        }

        // Benchmark
        for _ in 0..ITERATIONS {
            let entry = &entries[500]; // Middle entry
            let start = rdtscp();
            let _ = std::hint::black_box(entry.get());
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
        }

        print_histogram("entry.get()", &hist);
    }

    #[test]
    #[ignore]
    fn bench_entry_get_mut() {
        let mut storage: GrowableListStorage<u64> =
            GrowableListStorage::with_capacity(1024);
        let mut list: List<u64, GrowableListStorage<u64>> = List::new();
        let mut hist = Histogram::<u64>::new(3).unwrap();

        // Build a list of 1000 elements and collect entries
        let mut entries = Vec::with_capacity(1000);
        for i in 0..1000 {
            entries.push(list.push_back_entry(&mut storage, i as u64));
        }

        // Warmup
        for _ in 0..WARMUP {
            for entry in &entries {
                std::hint::black_box(entry.get_mut());
            }
        }

        // Benchmark
        for _ in 0..ITERATIONS {
            let entry = &entries[500]; // Middle entry
            let start = rdtscp();
            let _ = std::hint::black_box(entry.get_mut());
            let elapsed = rdtscp() - start;
            hist.record(elapsed).unwrap();
        }

        print_histogram("entry.get_mut()", &hist);
    }
}
