//! Doubly-linked list with closure-based ownership.
//!
//! # Design Overview
//!
//! - **Slab owns data** - All node data lives in the slab
//! - **List manages links** - Just prev/next pointer bookkeeping
//! - **Slots are opaque** - No data access methods, just identity
//! - **Access via closures** - `list.read(&slot, f)`, `list.write(&mut slot, f)`
//! - **Borrow checker enforces safety** - `write(&mut self, &mut slot, f)` signature
//!
//! # User Invariants
//!
//! You must uphold these invariants for correct behavior:
//!
//! 1. **Consume all guards** - `Detached` must call `take()` or `try_take()`
//! 2. **Unlink slots before dropping** - Don't drop `ListSlot` while linked
//! 3. **Return correct slot from `take()`** - Match the popped element
//! 4. **Keep your index in sync** - Track slots in your HashMap
//!
//! Violations with safe API → panic with clear message.
//! Violations with unchecked API → undefined behavior.
//!
//! # Example: Order Queue
//!
//! ```ignore
//! use nexus_collections::list::{List, BoundedListSlab, ListSlot};
//! use std::collections::HashMap;
//!
//! struct Order { id: u64, price: f64, qty: u32 }
//!
//! let slab = BoundedListSlab::<Order>::with_capacity(10_000);
//! let mut list = List::new(slab);
//! let mut index: HashMap<u64, ListSlot<Order>> = HashMap::new();
//!
//! // Insert order
//! let order = Order { id: 1, price: 100.0, qty: 50 };
//! let order_id = order.id;
//! let detached = slab.create_node(order).unwrap();
//! let slot = list.link_back(detached);
//! index.insert(order_id, slot);
//!
//! // Read order
//! let slot = index.get(&1).unwrap();
//! let price = list.read(slot, |o| o.price);
//!
//! // Modify order
//! let slot = index.get_mut(&1).unwrap();
//! list.write(slot, |o| o.qty -= 10);
//!
//! // Cancel order (unlink)
//! let slot = index.remove(&1).unwrap();
//! let detached = list.unlink(slot);
//! let order = detached.take();
//! ```

use core::cell::Cell;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;

use crate::internal::{SlabOps, SlotOps};
use nexus_slab::{Full, Key, bounded, unbounded};

// =============================================================================
// Slab Newtypes - Controlled access to storage
// =============================================================================

/// Bounded slab for list nodes.
///
/// Wraps a `bounded::Slab` to provide controlled access. Users allocate nodes
/// via `create_node()`, which is fallible since capacity is fixed.
///
/// This type is `Copy` - it's just a pointer to the underlying storage.
/// Internal trait for slab operations used by List.
///
/// This is separate from `SlabOps` to avoid circular dependencies.
/// Users should not implement this trait.
pub trait ListSlabOps<T>: Copy {
    /// Type of slab slot.
    type Slot: SlotOps;

    /// Checks if a key is valid.
    fn contains_key(&self, key: Key) -> bool;

    /// Returns true if at capacity.
    fn is_full(&self) -> bool;

    /// Safe get (returns None if invalid).
    ///
    /// # Safety
    /// Caller must ensure no mutable references to this slot exist.
    unsafe fn get(&self, key: Key) -> Option<&Node<T>>;

    /// Unchecked mutable get.
    ///
    /// # Safety
    /// Key must be valid. No other references may exist.
    #[allow(clippy::mut_from_ref)]
    unsafe fn get_unchecked_mut(&self, key: Key) -> &mut Node<T>;

    /// Tries to remove, returning the node if present.
    fn try_remove(&self, key: Key) -> Option<Node<T>>;

    /// Unchecked remove.
    ///
    /// # Safety
    /// Key must be valid and occupied.
    unsafe fn remove_unchecked(&self, key: Key) -> Node<T>;
}

/// Bounded slab for list nodes.
///
/// Wraps a `bounded::Slab` to provide controlled access. Allocate nodes
/// via `create_node()`, which is fallible since capacity is fixed.
///
/// This type is `Copy` - it's just a pointer to the underlying storage.
#[derive(Debug)]
pub struct BoundedListSlab<T>(bounded::Slab<Node<T>>);

impl<T> Clone for BoundedListSlab<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for BoundedListSlab<T> {}

impl<T> BoundedListSlab<T> {
    /// Creates a new bounded slab with the given capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self(bounded::Slab::with_capacity(capacity))
    }

    /// Returns the number of nodes in the slab.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the slab is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the capacity of the slab.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    /// Returns `true` if the slab is at capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.0.is_full()
    }

    /// Creates a new detached list node.
    ///
    /// The node is allocated in the slab but not linked to any list.
    ///
    /// # Errors
    ///
    /// Returns `Err(Full)` if the slab is at capacity.
    #[inline]
    pub fn create_node(&self, data: T) -> Result<DetachedListNode<T, Self>, Full<T>> {
        let slot = self
            .0
            .slab_try_insert(Node::detached(data))
            .map_err(|e| Full(e.0.data))?;
        let key = slot.leak();
        Ok(DetachedListNode {
            slab: *self,
            key,
            _marker: PhantomData,
        })
    }
}

/// Unbounded (growable) slab for list nodes.
///
/// Wraps an `unbounded::Slab` to provide controlled access. Users allocate nodes
/// via `create_node()`, which is infallible since the slab grows as needed.
///
/// This type is `Copy` - it's just a pointer to the underlying storage.
#[derive(Debug)]
pub struct UnboundedListSlab<T>(unbounded::Slab<Node<T>>);

impl<T> Clone for UnboundedListSlab<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for UnboundedListSlab<T> {}

impl<T> UnboundedListSlab<T> {
    /// Creates a new unbounded slab.
    #[inline]
    pub fn new() -> Self {
        Self(unbounded::Slab::new())
    }

    /// Creates a new unbounded slab with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self(unbounded::Slab::with_capacity(capacity))
    }

    /// Returns the number of nodes in the slab.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the slab is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the current capacity of the slab.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    /// Creates a new detached list node.
    ///
    /// The node is allocated in the slab but not linked to any list.
    /// This is infallible - the slab grows as needed.
    #[inline]
    pub fn create_node(&self, data: T) -> DetachedListNode<T, Self> {
        let slot = self
            .0
            .slab_try_insert(Node::detached(data))
            .unwrap_or_else(|_| unreachable!("unbounded slab should never be full"));
        let key = slot.leak();
        DetachedListNode {
            slab: *self,
            key,
            _marker: PhantomData,
        }
    }
}

impl<T> Default for UnboundedListSlab<T> {
    fn default() -> Self {
        Self::new()
    }
}

// -----------------------------------------------------------------------------
// ListSlabOps implementations
// -----------------------------------------------------------------------------

impl<T> ListSlabOps<T> for BoundedListSlab<T> {
    type Slot = bounded::Slot<Node<T>>;

    #[inline]
    fn contains_key(&self, key: Key) -> bool {
        self.0.contains_key(key)
    }

    #[inline]
    fn is_full(&self) -> bool {
        self.0.is_full()
    }

    #[inline]
    unsafe fn get(&self, key: Key) -> Option<&Node<T>> {
        if self.0.contains_key(key) {
            Some(unsafe { self.0.get_by_key(key) })
        } else {
            None
        }
    }

    #[inline]
    unsafe fn get_unchecked_mut(&self, key: Key) -> &mut Node<T> {
        unsafe { self.0.get_by_key_mut(key) }
    }

    #[inline]
    fn try_remove(&self, key: Key) -> Option<Node<T>> {
        if self.0.contains_key(key) {
            Some(unsafe { self.0.remove_by_key(key) })
        } else {
            None
        }
    }

    #[inline]
    unsafe fn remove_unchecked(&self, key: Key) -> Node<T> {
        unsafe { self.0.remove_by_key(key) }
    }
}

impl<T> ListSlabOps<T> for UnboundedListSlab<T> {
    type Slot = unbounded::Slot<Node<T>>;

    #[inline]
    fn contains_key(&self, key: Key) -> bool {
        self.0.contains_key(key)
    }

    #[inline]
    fn is_full(&self) -> bool {
        false // Never full
    }

    #[inline]
    unsafe fn get(&self, key: Key) -> Option<&Node<T>> {
        if self.0.contains_key(key) {
            Some(unsafe { self.0.get_by_key(key) })
        } else {
            None
        }
    }

    #[inline]
    unsafe fn get_unchecked_mut(&self, key: Key) -> &mut Node<T> {
        unsafe { self.0.get_by_key_mut(key) }
    }

    #[inline]
    fn try_remove(&self, key: Key) -> Option<Node<T>> {
        if self.0.contains_key(key) {
            Some(unsafe { self.0.remove_by_key(key) })
        } else {
            None
        }
    }

    #[inline]
    unsafe fn remove_unchecked(&self, key: Key) -> Node<T> {
        unsafe { self.0.remove_by_key(key) }
    }
}

// =============================================================================
// Id - Unique list identifier
// =============================================================================

thread_local! {
    static NEXT_ID: Cell<u32> = const { Cell::new(0) };
}

/// Unique identifier for a list instance.
///
/// Each list gets a unique ID on creation. Nodes track which list owns them
/// to catch cross-list operations.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Id(u32);

impl Id {
    /// Sentinel value indicating no owner (detached state).
    pub const NONE: Id = Id(u32::MAX);

    /// Returns `true` if this is the NONE sentinel.
    #[inline]
    pub fn is_none(self) -> bool {
        self.0 == u32::MAX
    }

    /// Returns `true` if this is not the NONE sentinel.
    #[inline]
    pub fn is_some(self) -> bool {
        self.0 != u32::MAX
    }
}

fn next_id() -> Id {
    NEXT_ID.with(|c| {
        let id = c.get();
        c.set(id.wrapping_add(1));
        Id(id)
    })
}

// =============================================================================
// Node - Internal linked list node
// =============================================================================

/// A node in the linked list.
///
/// Wraps user data with prev/next links and ownership tracking.
/// This type is exposed for slab typing but internals are not public.
#[derive(Debug)]
pub struct Node<T> {
    pub(crate) data: T,
    pub(crate) prev: Key,
    pub(crate) next: Key,
    pub(crate) owner: Id,
}

impl<T> Node<T> {
    /// Creates a new detached node with no owner.
    #[inline]
    fn detached(data: T) -> Self {
        Self {
            data,
            prev: Key::NONE,
            next: Key::NONE,
            owner: Id::NONE,
        }
    }
}

// =============================================================================
// DetachedListNode - Unlinked handle
// =============================================================================

/// Handle to data in slab, NOT linked to any list.
///
/// This is the "unlinked" state. You can:
/// - Link it to a list via `list.link_back()` / `list.link_front()`
/// - Extract the data via `take()`
///
/// Dropping this removes the data from the slab (correct cleanup).
pub struct DetachedListNode<T, S>
where
    S: ListSlabOps<T>,
{
    slab: S,
    key: Key,
    _marker: PhantomData<T>,
}

impl<T, S> DetachedListNode<T, S>
where
    S: ListSlabOps<T>,
{
    /// Extracts the owned data, removing from slab. Consumes handle.
    #[inline]
    pub fn take(self) -> T {
        // Use ManuallyDrop to prevent Drop from running (which would double-free)
        let this = ManuallyDrop::new(self);
        unsafe { this.slab.remove_unchecked(this.key) }.data
    }

    /// Returns the key for internal use.
    #[inline]
    pub(crate) fn key(&self) -> Key {
        self.key
    }

    /// Converts to a linked slot. Internal use only.
    #[inline]
    pub(crate) fn into_slot(self) -> ListSlot<T, S> {
        let this = ManuallyDrop::new(self);
        ListSlot {
            slab: this.slab,
            key: this.key,
            _marker: PhantomData,
        }
    }
}

impl<T, S> Drop for DetachedListNode<T, S>
where
    S: ListSlabOps<T>,
{
    fn drop(&mut self) {
        // Remove from slab on drop (correct cleanup for detached node)
        self.slab.try_remove(self.key);
    }
}

// =============================================================================
// ListSlot - Linked handle (OPAQUE)
// =============================================================================

/// Handle to data in slab, linked to a list.
///
/// **OPAQUE** - This type has NO data access methods.
/// Access data via `list.read(&slot, f)` or `list.write(&mut slot, f)`.
///
/// # User Invariant
///
/// You must unlink this slot before dropping it. Dropping while linked
/// removes the slab entry, creating dangling prev/next pointers in the list.
/// The safe API will panic on subsequent access; the unchecked API has UB.
pub struct ListSlot<T, S>
where
    S: ListSlabOps<T>,
{
    slab: S,
    key: Key,
    _marker: PhantomData<T>,
}

impl<T, S> ListSlot<T, S>
where
    S: ListSlabOps<T>,
{
    /// Returns the key for internal use.
    #[inline]
    pub(crate) fn key(&self) -> Key {
        self.key
    }

    /// Converts to detached. Internal use only (after unlink).
    #[inline]
    pub(crate) fn into_detached(self) -> DetachedListNode<T, S> {
        let this = ManuallyDrop::new(self);
        DetachedListNode {
            slab: this.slab,
            key: this.key,
            _marker: PhantomData,
        }
    }
}

impl<T, S> Drop for ListSlot<T, S>
where
    S: ListSlabOps<T>,
{
    fn drop(&mut self) {
        // Slot owns the slab entry - dropping removes it.
        // If still linked, this creates dangling prev/next pointers in the list.
        // Safe API will catch this on next access via slab.get() returning None.
        // This is a user invariant violation, not a bug in our code.
        self.slab.try_remove(self.key);
    }
}

// =============================================================================
// Detached - Transitionary guard for pop operations
// =============================================================================

/// Transitionary guard for popped elements.
///
/// Holds a reference to the popped data so you can identify which slot
/// to return from your index. Call `take()` or `try_take()` to complete
/// the type-state transition.
///
/// # User Invariant
///
/// You must consume this guard via `take()` or `try_take()`.
/// Dropping without consuming orphans the popped node in the slab.
pub struct Detached<'a, T, S>
where
    S: ListSlabOps<T>,
{
    list: &'a mut List<T, S>,
    key: Key,
    _marker: PhantomData<&'a T>,
}

impl<T, S> Detached<'_, T, S>
where
    S: ListSlabOps<T>,
{
    /// Take the slot back using the reference to identify it.
    ///
    /// The closure receives `&T` to identify which slot to retrieve.
    /// Return your `ListSlot` (removed from your HashMap).
    ///
    /// # Panics
    ///
    /// Panics if the closure panics. Use `try_take` for fallible lookups.
    #[inline]
    pub fn take<F>(self, f: F) -> DetachedListNode<T, S>
    where
        F: FnOnce(&T) -> ListSlot<T, S>,
    {
        let this = ManuallyDrop::new(self);
        // SAFETY: Read-only access, no mutable refs exist to this slot
        let node =
            unsafe { this.list.slab.get(this.key) }.expect("detached node was removed from slab");
        let slot = f(&node.data);
        slot.into_detached()
    }

    /// Fallible version for when the slot lookup might fail.
    ///
    /// Returns `None` if the closure returns `None` (slot not found).
    ///
    /// # Warning
    ///
    /// If this returns `None`, the popped node is orphaned in the slab.
    /// This is a memory leak and indicates a bug in your index tracking.
    #[inline]
    pub fn try_take<F>(self, f: F) -> Option<DetachedListNode<T, S>>
    where
        F: FnOnce(&T) -> Option<ListSlot<T, S>>,
    {
        let this = ManuallyDrop::new(self);
        // SAFETY: Read-only access, no mutable refs exist to this slot
        let data = match unsafe { this.list.slab.get(this.key) } {
            Some(node) => &node.data,
            None => return None,
        };
        f(data).map(ListSlot::into_detached)
    }
}

impl<T, S> Drop for Detached<'_, T, S>
where
    S: ListSlabOps<T>,
{
    fn drop(&mut self) {
        // Trivial drop - no panic.
        // If not consumed, the popped node is orphaned in slab (leak).
        // We don't remove it here because take()/try_take() might still work.
    }
}

// =============================================================================
// List
// =============================================================================

/// A doubly-linked list backed by a slab allocator.
///
/// # Type Parameters
///
/// - `T`: Element type
/// - `S`: Slab type (e.g., `BoundedListSlab<T>` or `UnboundedListSlab<T>`)
///
/// # Access Pattern
///
/// All data access goes through closures:
///
/// ```ignore
/// list.read(&slot, |data| data.field);      // Shared access
/// list.write(&mut slot, |data| data.x = 1); // Exclusive access
/// ```
///
/// This ensures the borrow checker prevents aliasing.
#[derive(Debug)]
pub struct List<T, S>
where
    S: ListSlabOps<T>,
{
    slab: S,
    head: Key,
    tail: Key,
    len: usize,
    id: Id,
    _marker: PhantomData<T>,
}

impl<T, S> List<T, S>
where
    S: ListSlabOps<T>,
{
    /// Creates a new list backed by the given slab.
    #[inline]
    pub fn new(slab: S) -> Self {
        Self {
            slab,
            head: Key::NONE,
            tail: Key::NONE,
            len: 0,
            id: next_id(),
            _marker: PhantomData,
        }
    }

    /// Returns the unique ID of this list.
    #[inline]
    pub fn id(&self) -> Id {
        self.id
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

    /// Returns `true` if the slab is at capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.slab.is_full()
    }

    // =========================================================================
    // Link operations (DetachedListNode -> ListSlot)
    // =========================================================================

    /// Links a detached node to the back of the list.
    ///
    /// Consumes the `DetachedListNode` and returns a `ListSlot`.
    #[inline]
    pub fn link_back(&mut self, node: DetachedListNode<T, S>) -> ListSlot<T, S> {
        let key = node.key();

        // Set up links
        {
            let n = unsafe { self.slab.get_unchecked_mut(key) };
            n.prev = self.tail;
            n.next = Key::NONE;
            n.owner = self.id;
        }

        // Update tail's next pointer
        if self.tail.is_some() {
            assert!(
                self.slab.contains_key(self.tail),
                "list tail is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.slab.get_unchecked_mut(self.tail) }.next = key;
        } else {
            self.head = key;
        }

        self.tail = key;
        self.len += 1;

        node.into_slot()
    }

    /// Links a detached node to the front of the list.
    ///
    /// Consumes the `DetachedListNode` and returns a `ListSlot`.
    #[inline]
    pub fn link_front(&mut self, node: DetachedListNode<T, S>) -> ListSlot<T, S> {
        let key = node.key();

        // Set up links
        {
            let n = unsafe { self.slab.get_unchecked_mut(key) };
            n.prev = Key::NONE;
            n.next = self.head;
            n.owner = self.id;
        }

        // Update head's prev pointer
        if self.head.is_some() {
            assert!(
                self.slab.contains_key(self.head),
                "list head is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.slab.get_unchecked_mut(self.head) }.prev = key;
        } else {
            self.tail = key;
        }

        self.head = key;
        self.len += 1;

        node.into_slot()
    }

    // =========================================================================
    // Unlink operations (ListSlot -> DetachedListNode)
    // =========================================================================

    /// Unlinks a slot from the list.
    ///
    /// Consumes the `ListSlot` and returns a `DetachedListNode`.
    ///
    /// # Panics
    ///
    /// Panics if the slot is invalid or doesn't belong to this list.
    #[inline]
    pub fn unlink(&mut self, slot: ListSlot<T, S>) -> DetachedListNode<T, S> {
        let key = slot.key();

        // Validate and get links
        // SAFETY: Read-only access, no mutable refs exist to this slot
        let (prev, next) = {
            let node = unsafe { self.slab.get(key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            (node.prev, node.next)
        };

        // Validate all neighbors upfront before any mutations.
        if prev.is_some() {
            assert!(
                self.slab.contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        if next.is_some() {
            assert!(
                self.slab.contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }

        // All keys validated - now do mutations
        if prev.is_some() {
            unsafe { self.slab.get_unchecked_mut(prev) }.next = next;
        } else {
            self.head = next;
        }

        if next.is_some() {
            unsafe { self.slab.get_unchecked_mut(next) }.prev = prev;
        } else {
            self.tail = prev;
        }

        // Clear ownership
        unsafe { self.slab.get_unchecked_mut(key) }.owner = Id::NONE;

        self.len -= 1;

        slot.into_detached()
    }

    // =========================================================================
    // Read access (closure-based)
    // =========================================================================

    /// Reads the data at the slot via closure.
    ///
    /// # Panics
    ///
    /// Panics if the slot is invalid or doesn't belong to this list.
    #[inline]
    pub fn read<F, R>(&self, slot: &ListSlot<T, S>, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        // SAFETY: Read-only access, no mutable refs exist to this slot
        let node = unsafe { self.slab.get(slot.key()) }
            .expect("slot is invalid (was it dropped without unlinking?)");
        assert_eq!(node.owner, self.id, "slot belongs to different list");
        f(&node.data)
    }

    /// Reads the front element via closure.
    #[inline]
    pub fn front<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&T) -> R,
    {
        if self.head.is_none() {
            return None;
        }
        // SAFETY: Read-only access, no mutable refs exist; head is validated
        let node = unsafe { self.slab.get(self.head) }
            .expect("list head is invalid (internal corruption)");
        Some(f(&node.data))
    }

    /// Reads the back element via closure.
    #[inline]
    pub fn back<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&T) -> R,
    {
        if self.tail.is_none() {
            return None;
        }
        // SAFETY: Read-only access, no mutable refs exist; tail is validated
        let node = unsafe { self.slab.get(self.tail) }
            .expect("list tail is invalid (internal corruption)");
        Some(f(&node.data))
    }

    // =========================================================================
    // Write access (closure-based)
    // =========================================================================

    /// Writes to the data at the slot via closure.
    ///
    /// Takes `&mut slot` to emphasize mutation and prevent aliasing.
    ///
    /// # Panics
    ///
    /// Panics if the slot is invalid or doesn't belong to this list.
    #[inline]
    pub fn write<F, R>(&mut self, slot: &mut ListSlot<T, S>, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let key = slot.key();
        // Validate first
        // SAFETY: Read-only access for validation, no mutable refs yet
        {
            let node = unsafe { self.slab.get(key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
        }
        // SAFETY: Key is validated above, getting exclusive access
        let node = unsafe { self.slab.get_unchecked_mut(key) };
        f(&mut node.data)
    }

    /// Writes to the front element via closure.
    #[inline]
    pub fn front_mut<F, R>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        if self.head.is_none() {
            return None;
        }
        let node = unsafe { self.slab.get_unchecked_mut(self.head) };
        Some(f(&mut node.data))
    }

    /// Writes to the back element via closure.
    #[inline]
    pub fn back_mut<F, R>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        if self.tail.is_none() {
            return None;
        }
        let node = unsafe { self.slab.get_unchecked_mut(self.tail) };
        Some(f(&mut node.data))
    }

    // =========================================================================
    // Position checks
    // =========================================================================

    /// Returns `true` if the slot is at the head of the list.
    ///
    /// # Panics
    ///
    /// Panics if the slot is invalid or doesn't belong to this list.
    #[inline]
    pub fn is_head(&self, slot: &ListSlot<T, S>) -> bool {
        let key = slot.key();
        // Validate ownership
        let node = unsafe { self.slab.get(key) }
            .expect("slot is invalid (was it dropped without unlinking?)");
        assert_eq!(node.owner, self.id, "slot belongs to different list");
        key == self.head
    }

    /// Returns `true` if the slot is at the tail of the list.
    ///
    /// # Panics
    ///
    /// Panics if the slot is invalid or doesn't belong to this list.
    #[inline]
    pub fn is_tail(&self, slot: &ListSlot<T, S>) -> bool {
        let key = slot.key();
        // Validate ownership
        let node = unsafe { self.slab.get(key) }
            .expect("slot is invalid (was it dropped without unlinking?)");
        assert_eq!(node.owner, self.id, "slot belongs to different list");
        key == self.tail
    }

    // =========================================================================
    // Relative link operations (insert relative to existing slot)
    // =========================================================================

    /// Links a detached node immediately after an existing slot.
    ///
    /// Consumes the `DetachedListNode` and returns a `ListSlot`.
    ///
    /// # Panics
    ///
    /// Panics if `after` is invalid or doesn't belong to this list.
    #[inline]
    pub fn link_after(
        &mut self,
        after: &ListSlot<T, S>,
        node: DetachedListNode<T, S>,
    ) -> ListSlot<T, S> {
        let after_key = after.key();
        let new_key = node.key();

        // Validate `after` and get its next pointer
        let next = {
            let n = unsafe { self.slab.get(after_key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(n.owner, self.id, "slot belongs to different list");
            n.next
        };

        // Set up new node's links
        {
            let n = unsafe { self.slab.get_unchecked_mut(new_key) };
            n.prev = after_key;
            n.next = next;
            n.owner = self.id;
        }

        // Update `after`'s next pointer
        unsafe { self.slab.get_unchecked_mut(after_key) }.next = new_key;

        // Update next's prev pointer (or tail if inserting at end)
        if next.is_some() {
            assert!(
                self.slab.contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.slab.get_unchecked_mut(next) }.prev = new_key;
        } else {
            self.tail = new_key;
        }

        self.len += 1;

        node.into_slot()
    }

    /// Links a detached node immediately before an existing slot.
    ///
    /// Consumes the `DetachedListNode` and returns a `ListSlot`.
    ///
    /// # Panics
    ///
    /// Panics if `before` is invalid or doesn't belong to this list.
    #[inline]
    pub fn link_before(
        &mut self,
        before: &ListSlot<T, S>,
        node: DetachedListNode<T, S>,
    ) -> ListSlot<T, S> {
        let before_key = before.key();
        let new_key = node.key();

        // Validate `before` and get its prev pointer
        let prev = {
            let n = unsafe { self.slab.get(before_key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(n.owner, self.id, "slot belongs to different list");
            n.prev
        };

        // Set up new node's links
        {
            let n = unsafe { self.slab.get_unchecked_mut(new_key) };
            n.prev = prev;
            n.next = before_key;
            n.owner = self.id;
        }

        // Update `before`'s prev pointer
        unsafe { self.slab.get_unchecked_mut(before_key) }.prev = new_key;

        // Update prev's next pointer (or head if inserting at start)
        if prev.is_some() {
            assert!(
                self.slab.contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.slab.get_unchecked_mut(prev) }.next = new_key;
        } else {
            self.head = new_key;
        }

        self.len += 1;

        node.into_slot()
    }

    // =========================================================================
    // Move operations (reposition without unlinking)
    // =========================================================================

    /// Moves a linked slot to the front of the list.
    ///
    /// This is more efficient than unlink + link_front as it doesn't
    /// change ownership or require type-state transitions.
    ///
    /// # Panics
    ///
    /// Panics if the slot is invalid or doesn't belong to this list.
    #[inline]
    pub fn move_to_front(&mut self, slot: &ListSlot<T, S>) {
        let key = slot.key();

        // Validate slot and check if already at front
        let (prev, next) = {
            let node = unsafe { self.slab.get(key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            if node.prev.is_none() {
                return; // Already at front
            }
            (node.prev, node.next)
        };

        // Validate all neighbors upfront before any mutations.
        // prev is guaranteed Some (we returned early if not).
        // self.head is guaranteed Some and != key (if slot was head, prev would be None).
        assert!(
            self.slab.contains_key(prev),
            "prev neighbor is invalid (was a slot dropped without unlinking?)"
        );
        if next.is_some() {
            assert!(
                self.slab.contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        assert!(
            self.slab.contains_key(self.head),
            "list head is invalid (was a slot dropped without unlinking?)"
        );

        // All keys validated - now do mutations
        unsafe { self.slab.get_unchecked_mut(prev) }.next = next;

        if next.is_some() {
            unsafe { self.slab.get_unchecked_mut(next) }.prev = prev;
        } else {
            self.tail = prev;
        }

        {
            let n = unsafe { self.slab.get_unchecked_mut(key) };
            n.prev = Key::NONE;
            n.next = self.head;
        }

        unsafe { self.slab.get_unchecked_mut(self.head) }.prev = key;
        self.head = key;
    }

    /// Moves a linked slot to the back of the list.
    ///
    /// This is more efficient than unlink + link_back as it doesn't
    /// change ownership or require type-state transitions.
    ///
    /// # Panics
    ///
    /// Panics if the slot is invalid or doesn't belong to this list.
    #[inline]
    pub fn move_to_back(&mut self, slot: &ListSlot<T, S>) {
        let key = slot.key();

        // Validate slot and check if already at back
        let (prev, next) = {
            let node = unsafe { self.slab.get(key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            if node.next.is_none() {
                return; // Already at back
            }
            (node.prev, node.next)
        };

        // Validate all neighbors upfront before any mutations.
        // next is guaranteed Some (we returned early if not).
        // self.tail is guaranteed Some and != key (if slot was tail, next would be None).
        assert!(
            self.slab.contains_key(next),
            "next neighbor is invalid (was a slot dropped without unlinking?)"
        );
        if prev.is_some() {
            assert!(
                self.slab.contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        assert!(
            self.slab.contains_key(self.tail),
            "list tail is invalid (was a slot dropped without unlinking?)"
        );

        // All keys validated - now do mutations
        unsafe { self.slab.get_unchecked_mut(next) }.prev = prev;

        if prev.is_some() {
            unsafe { self.slab.get_unchecked_mut(prev) }.next = next;
        } else {
            self.head = next;
        }

        {
            let n = unsafe { self.slab.get_unchecked_mut(key) };
            n.prev = self.tail;
            n.next = Key::NONE;
        }

        unsafe { self.slab.get_unchecked_mut(self.tail) }.next = key;
        self.tail = key;
    }

    /// Moves `slot` to immediately before `target`.
    ///
    /// Both slots must belong to this list.
    ///
    /// # Panics
    ///
    /// Panics if either slot is invalid or doesn't belong to this list.
    #[inline]
    pub fn move_before(&mut self, slot: &ListSlot<T, S>, target: &ListSlot<T, S>) {
        let slot_key = slot.key();
        let target_key = target.key();

        // Same slot - no-op
        if slot_key == target_key {
            return;
        }

        // Validate slot and get its links
        let (slot_prev, slot_next) = {
            let node = unsafe { self.slab.get(slot_key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            (node.prev, node.next)
        };

        // Validate target and get its prev
        let target_prev = {
            let node = unsafe { self.slab.get(target_key) }
                .expect("target is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "target belongs to different list");
            node.prev
        };

        // Already immediately before target - no-op
        if slot_next == target_key {
            return;
        }

        // Validate all neighbors upfront before any mutations
        if slot_prev.is_some() {
            assert!(
                self.slab.contains_key(slot_prev),
                "slot's prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        if slot_next.is_some() {
            assert!(
                self.slab.contains_key(slot_next),
                "slot's next neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        if target_prev.is_some() {
            assert!(
                self.slab.contains_key(target_prev),
                "target's prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }

        // All keys validated - now do mutations
        if slot_prev.is_some() {
            unsafe { self.slab.get_unchecked_mut(slot_prev) }.next = slot_next;
        } else {
            self.head = slot_next;
        }
        if slot_next.is_some() {
            unsafe { self.slab.get_unchecked_mut(slot_next) }.prev = slot_prev;
        } else {
            self.tail = slot_prev;
        }

        {
            let n = unsafe { self.slab.get_unchecked_mut(slot_key) };
            n.prev = target_prev;
            n.next = target_key;
        }

        unsafe { self.slab.get_unchecked_mut(target_key) }.prev = slot_key;

        if target_prev.is_some() {
            unsafe { self.slab.get_unchecked_mut(target_prev) }.next = slot_key;
        } else {
            self.head = slot_key;
        }
    }

    /// Moves `slot` to immediately after `target`.
    ///
    /// Both slots must belong to this list.
    ///
    /// # Panics
    ///
    /// Panics if either slot is invalid or doesn't belong to this list.
    #[inline]
    pub fn move_after(&mut self, slot: &ListSlot<T, S>, target: &ListSlot<T, S>) {
        let slot_key = slot.key();
        let target_key = target.key();

        // Same slot - no-op
        if slot_key == target_key {
            return;
        }

        // Validate slot and get its links
        let (slot_prev, slot_next) = {
            let node = unsafe { self.slab.get(slot_key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            (node.prev, node.next)
        };

        // Validate target and get its next
        let target_next = {
            let node = unsafe { self.slab.get(target_key) }
                .expect("target is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "target belongs to different list");
            node.next
        };

        // Already immediately after target - no-op
        if slot_prev == target_key {
            return;
        }

        // Validate all neighbors upfront before any mutations.
        if slot_prev.is_some() {
            assert!(
                self.slab.contains_key(slot_prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        if slot_next.is_some() {
            assert!(
                self.slab.contains_key(slot_next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        if target_next.is_some() {
            assert!(
                self.slab.contains_key(target_next),
                "target's next neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }

        // All keys validated - now do mutations

        // Remove slot from current position
        if slot_prev.is_some() {
            unsafe { self.slab.get_unchecked_mut(slot_prev) }.next = slot_next;
        } else {
            self.head = slot_next;
        }
        if slot_next.is_some() {
            unsafe { self.slab.get_unchecked_mut(slot_next) }.prev = slot_prev;
        } else {
            self.tail = slot_prev;
        }

        // Insert slot after target
        {
            let n = unsafe { self.slab.get_unchecked_mut(slot_key) };
            n.prev = target_key;
            n.next = target_next;
        }

        // Update target's next
        unsafe { self.slab.get_unchecked_mut(target_key) }.next = slot_key;

        // Update target's old next (or tail)
        if target_next.is_some() {
            unsafe { self.slab.get_unchecked_mut(target_next) }.prev = slot_key;
        } else {
            self.tail = slot_key;
        }
    }

    // =========================================================================
    // Pop operations (returns Detached guard)
    // =========================================================================

    /// Pops the front element.
    ///
    /// Returns a `Detached` guard. Call `take()` or `try_take()` to complete
    /// the transition and get your slot back.
    #[inline]
    pub fn pop_front(&mut self) -> Option<Detached<'_, T, S>> {
        if self.head.is_none() {
            return None;
        }

        let key = self.head;

        // Get next before we modify
        // SAFETY: Read-only access, head is validated as is_some()
        let node =
            unsafe { self.slab.get(key) }.expect("list head is invalid (internal corruption)");
        let next = node.next;

        // Update head
        self.head = next;
        if next.is_some() {
            assert!(
                self.slab.contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.slab.get_unchecked_mut(next) }.prev = Key::NONE;
        } else {
            self.tail = Key::NONE;
        }

        // Clear ownership (node is now detached)
        unsafe { self.slab.get_unchecked_mut(key) }.owner = Id::NONE;

        self.len -= 1;

        Some(Detached {
            list: self,
            key,
            _marker: PhantomData,
        })
    }

    /// Pops the back element.
    ///
    /// Returns a `Detached` guard. Call `take()` or `try_take()` to complete
    /// the transition and get your slot back.
    #[inline]
    pub fn pop_back(&mut self) -> Option<Detached<'_, T, S>> {
        if self.tail.is_none() {
            return None;
        }

        let key = self.tail;

        // Get prev before we modify
        // SAFETY: Read-only access, tail is validated as is_some()
        let node =
            unsafe { self.slab.get(key) }.expect("list tail is invalid (internal corruption)");
        let prev = node.prev;

        // Update tail
        self.tail = prev;
        if prev.is_some() {
            assert!(
                self.slab.contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.slab.get_unchecked_mut(prev) }.next = Key::NONE;
        } else {
            self.head = Key::NONE;
        }

        // Clear ownership (node is now detached)
        unsafe { self.slab.get_unchecked_mut(key) }.owner = Id::NONE;

        self.len -= 1;

        Some(Detached {
            list: self,
            key,
            _marker: PhantomData,
        })
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // =========================================================================
    // Order Queue Workflow Tests
    // =========================================================================

    #[derive(Debug, Clone, PartialEq)]
    struct Order {
        id: u64,
        price: f64,
        qty: u32,
    }

    impl Order {
        fn new(id: u64, price: f64, qty: u32) -> Self {
            Self { id, price, qty }
        }
    }

    #[test]
    fn order_queue_basic_workflow() {
        // Setup
        let slab = BoundedListSlab::<Order>::with_capacity(100);
        let mut queue: List<Order, _> = List::new(slab);
        let mut index: HashMap<u64, ListSlot<Order, _>> = HashMap::new();

        // Insert orders
        for i in 1..=5 {
            let order = Order::new(i, 100.0 + i as f64, 10 * i as u32);
            let order_id = order.id;
            let detached = slab.create_node(order).unwrap();
            let slot = queue.link_back(detached);
            index.insert(order_id, slot);
        }

        assert_eq!(queue.len(), 5);
        assert_eq!(index.len(), 5);

        // Read order 3
        let slot = index.get(&3).unwrap();
        let price = queue.read(slot, |o| o.price);
        assert_eq!(price, 103.0);

        // Modify order 3
        let slot = index.get_mut(&3).unwrap();
        queue.write(slot, |o| {
            o.qty -= 5;
        });

        // Verify modification
        let slot = index.get(&3).unwrap();
        let qty = queue.read(slot, |o| o.qty);
        assert_eq!(qty, 25); // 30 - 5

        // Cancel order 2 (unlink from queue)
        let slot = index.remove(&2).unwrap();
        let detached = queue.unlink(slot);
        let order = detached.take();
        assert_eq!(order.id, 2);
        assert_eq!(queue.len(), 4);

        // Pop front (order 1)
        let detached = queue.pop_front().unwrap();
        let node = detached.take(|o| index.remove(&o.id).unwrap());
        let order = node.take();
        assert_eq!(order.id, 1);
        assert_eq!(queue.len(), 3);

        // Verify queue order: 3, 4, 5
        let front_id = queue.front(|o| o.id);
        assert_eq!(front_id, Some(3));

        let back_id = queue.back(|o| o.id);
        assert_eq!(back_id, Some(5));
    }

    #[test]
    fn order_queue_move_to_front() {
        let slab = BoundedListSlab::<Order>::with_capacity(100);
        let mut queue: List<Order, _> = List::new(slab);
        let mut index: HashMap<u64, ListSlot<Order, _>> = HashMap::new();

        // Insert orders 1, 2, 3
        for i in 1..=3 {
            let order = Order::new(i, 100.0, 10);
            let order_id = order.id;
            let detached = slab.create_node(order).unwrap();
            let slot = queue.link_back(detached);
            index.insert(order_id, slot);
        }

        // Move order 3 to front
        let slot = index.remove(&3).unwrap();
        let detached = queue.unlink(slot);
        let slot = queue.link_front(detached);
        index.insert(3, slot);

        // Verify order: 3 at front, 2 at back (1 is in middle)
        assert_eq!(queue.front(|o| o.id), Some(3));
        assert_eq!(queue.back(|o| o.id), Some(2));
        assert_eq!(queue.len(), 3);
    }

    #[test]
    fn order_queue_partial_fills() {
        let slab = BoundedListSlab::<Order>::with_capacity(100);
        let mut queue: List<Order, _> = List::new(slab);
        let mut index: HashMap<u64, ListSlot<Order, _>> = HashMap::new();

        // Insert order with qty 100
        let order = Order::new(1, 100.0, 100);
        let detached = slab.create_node(order).unwrap();
        let slot = queue.link_back(detached);
        index.insert(1, slot);

        // Partial fills
        for fill in [20, 30, 25] {
            let slot = index.get_mut(&1).unwrap();
            queue.write(slot, |o| {
                o.qty -= fill;
            });
        }

        // Check remaining qty
        let slot = index.get(&1).unwrap();
        let remaining = queue.read(slot, |o| o.qty);
        assert_eq!(remaining, 25); // 100 - 20 - 30 - 25

        // Final fill - remove order
        let slot = index.get_mut(&1).unwrap();
        let should_remove = queue.write(slot, |o| {
            o.qty -= 25;
            o.qty == 0
        });
        assert!(should_remove);

        let slot = index.remove(&1).unwrap();
        let detached = queue.unlink(slot);
        let order = detached.take();
        assert_eq!(order.qty, 0);
    }

    // =========================================================================
    // Connection Pool / Free List Tests
    // =========================================================================

    #[derive(Debug, Clone)]
    struct Session {
        id: u32,
        subscriptions: u32,
        max_subscriptions: u32,
    }

    impl Session {
        fn new(id: u32, max: u32) -> Self {
            Self {
                id,
                subscriptions: 0,
                max_subscriptions: max,
            }
        }

        fn is_full(&self) -> bool {
            self.subscriptions >= self.max_subscriptions
        }

        fn can_subscribe(&self) -> bool {
            self.subscriptions < self.max_subscriptions
        }
    }

    /// Tracks sessions that have available subscription capacity
    #[test]
    fn connection_pool_available_sessions() {
        let slab = BoundedListSlab::<Session>::with_capacity(100);
        let mut available: List<Session, _> = List::new(slab);
        let mut sessions: HashMap<u32, ListSlot<Session, _>> = HashMap::new();

        // Create 3 sessions with max 2 subscriptions each
        for id in 1..=3 {
            let session = Session::new(id, 2);
            let detached = slab.create_node(session).unwrap();
            let slot = available.link_back(detached);
            sessions.insert(id, slot);
        }

        // Subscribe to symbol on first available session
        fn subscribe(
            available: &mut List<Session, BoundedListSlab<Session>>,
            sessions: &mut HashMap<u32, ListSlot<Session, BoundedListSlab<Session>>>,
        ) -> Option<u32> {
            // Get first available session
            let session_id = available.front(|s| s.id)?;

            // Add subscription
            let slot = sessions.get_mut(&session_id)?;
            let is_now_full = available.write(slot, |s| {
                s.subscriptions += 1;
                s.is_full()
            });

            // If full, remove from available list
            if is_now_full {
                let slot = sessions.remove(&session_id)?;
                let detached = available.unlink(slot);
                // In real code, we'd store this somewhere else (full_sessions map)
                // For test, we just drop it
                let _ = detached.take();
            }

            Some(session_id)
        }

        // Subscribe 6 times (should use all 3 sessions x 2 slots each)
        let mut assignments = vec![];
        for _ in 0..6 {
            if let Some(id) = subscribe(&mut available, &mut sessions) {
                assignments.push(id);
            }
        }

        // Should have assigned: 1, 1, 2, 2, 3, 3 (each session gets 2)
        // After each session hits max, it's removed from available
        assert_eq!(assignments.len(), 6);
        assert!(available.is_empty()); // All sessions now full

        // 7th subscribe should fail - no available sessions
        assert!(subscribe(&mut available, &mut sessions).is_none());
    }

    #[test]
    fn connection_pool_return_to_available() {
        let slab = BoundedListSlab::<Session>::with_capacity(100);
        let mut available: List<Session, _> = List::new(slab);

        // One session, max 2 subscriptions
        let session = Session::new(1, 2);
        let detached = slab.create_node(session).unwrap();
        let mut slot = available.link_back(detached);

        // Fill it up
        available.write(&mut slot, |s| s.subscriptions = 2);
        assert!(available.read(&slot, |s| s.is_full()));

        // Unlink (session is now full, not in available list)
        let detached = available.unlink(slot);

        // Simulate unsubscribe - session has capacity again
        // Re-link to available list
        let mut slot = available.link_back(detached);
        available.write(&mut slot, |s| s.subscriptions = 1);

        assert!(!available.is_empty());
        assert!(available.read(&slot, |s| s.can_subscribe()));
    }

    // =========================================================================
    // Basic API Tests
    // =========================================================================

    #[test]
    fn basic_link_unlink() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let s1 = list.link_back(d1);
        let s2 = list.link_back(d2);
        let s3 = list.link_back(d3);

        assert_eq!(list.len(), 3);
        assert_eq!(list.read(&s1, |x| *x), 1);
        assert_eq!(list.read(&s2, |x| *x), 2);
        assert_eq!(list.read(&s3, |x| *x), 3);

        // Unlink middle
        let d2 = list.unlink(s2);
        assert_eq!(list.len(), 2);
        assert_eq!(d2.take(), 2);

        // Remaining: 1, 3
        assert_eq!(list.front(|x| *x), Some(1));
        assert_eq!(list.back(|x| *x), Some(3));
    }

    #[test]
    fn pop_with_detached_take() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);
        let mut index: HashMap<u64, ListSlot<u64, _>> = HashMap::new();

        for i in 1..=3 {
            let detached = slab.create_node(i).unwrap();
            let slot = list.link_back(detached);
            index.insert(i, slot);
        }

        // Pop front and find in index
        let detached = list.pop_front().unwrap();
        let node = detached.take(|&val| index.remove(&val).unwrap());
        assert_eq!(node.take(), 1);

        assert_eq!(list.len(), 2);
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn front_back_mut() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        // Must hold slots - dropping removes from slab (user invariant)
        let _s1 = list.link_back(d1);
        let _s2 = list.link_back(d2);

        list.front_mut(|x| *x = 10);
        list.back_mut(|x| *x = 20);

        assert_eq!(list.front(|x| *x), Some(10));
        assert_eq!(list.back(|x| *x), Some(20));
    }

    #[test]
    #[should_panic(expected = "slot belongs to different list")]
    fn cross_list_read_panics() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list1: List<u64, _> = List::new(slab);
        let list2: List<u64, _> = List::new(slab);

        let detached = slab.create_node(42).unwrap();
        let slot = list1.link_back(detached);

        // This should panic - slot belongs to list1, not list2
        let _ = list2.read(&slot, |x| *x);
    }

    #[test]
    fn empty_list_operations() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        assert!(list.front(|x| *x).is_none());
        assert!(list.back(|x| *x).is_none());
        assert!(list.front_mut(|x| *x).is_none());
        assert!(list.back_mut(|x| *x).is_none());
        assert!(list.pop_front().is_none());
        assert!(list.pop_back().is_none());
    }

    // =========================================================================
    // Additional Coverage Tests
    // =========================================================================

    #[test]
    fn pop_back_works() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);
        let mut index: HashMap<u64, ListSlot<u64, _>> = HashMap::new();

        for i in 1..=3 {
            let detached = slab.create_node(i).unwrap();
            let slot = list.link_back(detached);
            index.insert(i, slot);
        }

        // Pop from back
        let detached = list.pop_back().unwrap();
        let node = detached.take(|&val| index.remove(&val).unwrap());
        assert_eq!(node.take(), 3);

        assert_eq!(list.len(), 2);
        assert_eq!(list.back(|x| *x), Some(2));
    }

    #[test]
    fn link_front_standalone() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        // Link all to front - should result in order 3, 2, 1
        let _s1 = list.link_front(d1);
        let _s2 = list.link_front(d2);
        let _s3 = list.link_front(d3);

        assert_eq!(list.front(|x| *x), Some(3));
        assert_eq!(list.back(|x| *x), Some(1));
    }

    #[test]
    fn detached_try_take_success() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);
        let mut index: HashMap<u64, ListSlot<u64, _>> = HashMap::new();

        let detached = slab.create_node(42).unwrap();
        let slot = list.link_back(detached);
        index.insert(42, slot);

        let detached = list.pop_front().unwrap();
        let maybe_node = detached.try_take(|&val| index.remove(&val));
        assert!(maybe_node.is_some());
        assert_eq!(maybe_node.unwrap().take(), 42);
    }

    #[test]
    fn detached_try_take_failure() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);
        let mut index: HashMap<u64, ListSlot<u64, _>> = HashMap::new();

        let detached = slab.create_node(42).unwrap();
        let slot = list.link_back(detached);
        index.insert(42, slot);

        let detached = list.pop_front().unwrap();
        // Lookup with wrong key - simulates index out of sync
        let maybe_node = detached.try_take(|_| index.remove(&999));
        assert!(maybe_node.is_none());
        // Note: this orphans the node in slab (user invariant violation)
        // The slot for 42 is still in index but node is orphaned
    }

    #[test]
    fn single_element_unlink() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let detached = slab.create_node(1).unwrap();
        let slot = list.link_back(detached);

        assert_eq!(list.len(), 1);
        assert_eq!(list.front(|x| *x), Some(1));
        assert_eq!(list.back(|x| *x), Some(1));

        let detached = list.unlink(slot);
        assert_eq!(detached.take(), 1);

        assert!(list.is_empty());
        assert!(list.front(|x| *x).is_none());
        assert!(list.back(|x| *x).is_none());
    }

    #[test]
    fn single_element_pop_front() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);
        let mut index: HashMap<u64, ListSlot<u64, _>> = HashMap::new();

        let detached = slab.create_node(1).unwrap();
        let slot = list.link_back(detached);
        index.insert(1, slot);

        let detached = list.pop_front().unwrap();
        let node = detached.take(|&val| index.remove(&val).unwrap());
        assert_eq!(node.take(), 1);

        assert!(list.is_empty());
    }

    #[test]
    fn single_element_pop_back() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);
        let mut index: HashMap<u64, ListSlot<u64, _>> = HashMap::new();

        let detached = slab.create_node(1).unwrap();
        let slot = list.link_back(detached);
        index.insert(1, slot);

        let detached = list.pop_back().unwrap();
        let node = detached.take(|&val| index.remove(&val).unwrap());
        assert_eq!(node.take(), 1);

        assert!(list.is_empty());
    }

    #[test]
    fn detached_take_without_linking() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);

        // Create detached and immediately take - never linked to any list
        let detached = slab.create_node(42).unwrap();
        let value = detached.take();
        assert_eq!(value, 42);

        // Slab should be empty now
        assert!(slab.is_empty());
    }

    #[test]
    fn slab_capacity_exhaustion() {
        let slab = BoundedListSlab::<u64>::with_capacity(2);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();

        let _s1 = list.link_back(d1);
        let _s2 = list.link_back(d2);

        // Slab is full
        assert!(list.is_full());

        // Third insert should fail
        let result = slab.create_node(3);
        assert!(result.is_err());

        // The Full error contains the value we tried to insert
        match result {
            Err(full) => assert_eq!(full.0, 3),
            Ok(_) => panic!("expected Full error"),
        }
    }

    #[test]
    #[should_panic(expected = "list head is invalid")]
    fn dropped_slot_corrupts_list() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let detached = slab.create_node(42).unwrap();
        let slot = list.link_back(detached);

        // Drop the slot while still linked - violates user invariant
        // This removes the entry from slab but list.head still points to it
        drop(slot);

        // Now try to access via front - should panic
        // because head points to a removed slab entry
        let _ = list.front(|x| *x);
    }

    #[test]
    fn dropped_slot_doesnt_affect_other_slots() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let detached1 = slab.create_node(1).unwrap();
        let detached2 = slab.create_node(2).unwrap();

        let slot1 = list.link_back(detached1);
        let slot2 = list.link_back(detached2);

        // Drop first slot while linked - corrupts list structure
        // but slot2's data is still valid in slab
        drop(slot1);

        // slot2 can still be read (its slab entry is intact)
        // Note: list structure is now corrupt (head points to removed entry)
        // but direct slot access works because slot2's key is still valid
        assert_eq!(list.read(&slot2, |x| *x), 2);
    }

    #[test]
    #[should_panic(expected = "list tail is invalid")]
    fn dropped_tail_panics_on_link_back() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let detached1 = slab.create_node(1).unwrap();
        let detached2 = slab.create_node(2).unwrap();
        let detached3 = slab.create_node(3).unwrap();

        let _slot1 = list.link_back(detached1);
        let slot2 = list.link_back(detached2);

        // Drop tail while linked - corrupts list
        drop(slot2);

        // Now try to link_back - should panic because tail is invalid
        let _slot3 = list.link_back(detached3);
    }

    #[test]
    #[should_panic(expected = "prev neighbor is invalid")]
    fn dropped_neighbor_panics_on_unlink() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let detached1 = slab.create_node(1).unwrap();
        let detached2 = slab.create_node(2).unwrap();
        let detached3 = slab.create_node(3).unwrap();

        let _slot1 = list.link_back(detached1);
        let slot2 = list.link_back(detached2);
        let slot3 = list.link_back(detached3);

        // Drop middle slot while linked
        drop(slot2);

        // Now try to unlink slot3 - should panic because its prev (slot2) is invalid
        let _ = list.unlink(slot3);
    }

    #[test]
    fn unlink_head() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let s1 = list.link_back(d1);
        let _s2 = list.link_back(d2);
        let _s3 = list.link_back(d3);

        // Unlink head
        let detached = list.unlink(s1);
        assert_eq!(detached.take(), 1);

        assert_eq!(list.len(), 2);
        assert_eq!(list.front(|x| *x), Some(2));
        assert_eq!(list.back(|x| *x), Some(3));
    }

    #[test]
    fn unlink_tail() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let _s1 = list.link_back(d1);
        let _s2 = list.link_back(d2);
        let s3 = list.link_back(d3);

        // Unlink tail
        let detached = list.unlink(s3);
        assert_eq!(detached.take(), 3);

        assert_eq!(list.len(), 2);
        assert_eq!(list.front(|x| *x), Some(1));
        assert_eq!(list.back(|x| *x), Some(2));
    }

    // =========================================================================
    // Position Check Tests
    // =========================================================================

    #[test]
    fn is_head_is_tail_single_element() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let s1 = list.link_back(d1);

        // Single element is both head and tail
        assert!(list.is_head(&s1));
        assert!(list.is_tail(&s1));
    }

    #[test]
    fn is_head_is_tail_multiple_elements() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let s1 = list.link_back(d1);
        let s2 = list.link_back(d2);
        let s3 = list.link_back(d3);

        // Check positions
        assert!(list.is_head(&s1));
        assert!(!list.is_tail(&s1));

        assert!(!list.is_head(&s2));
        assert!(!list.is_tail(&s2));

        assert!(!list.is_head(&s3));
        assert!(list.is_tail(&s3));
    }

    // =========================================================================
    // Relative Link Tests
    // =========================================================================

    #[test]
    fn link_after_middle() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d3 = slab.create_node(3).unwrap();
        let d2 = slab.create_node(2).unwrap();

        // Insert 1, then 3, then insert 2 after 1
        let s1 = list.link_back(d1);
        let s3 = list.link_back(d3);
        let s2 = list.link_after(&s1, d2);

        // Order should be: 1, 2, 3
        assert_eq!(list.len(), 3);
        assert_eq!(list.front(|x| *x), Some(1));
        assert_eq!(list.back(|x| *x), Some(3));

        // Verify middle is 2 by checking slot s2 value
        assert_eq!(list.read(&s2, |x| *x), 2);

        // Verify structure: s1 is head, s3 is tail, s2 is in between
        assert!(list.is_head(&s1));
        assert!(list.is_tail(&s3));
        assert!(!list.is_head(&s2));
        assert!(!list.is_tail(&s2));
    }

    #[test]
    fn link_after_tail() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();

        let s1 = list.link_back(d1);
        let s2 = list.link_after(&s1, d2);

        // 2 should now be tail
        assert!(list.is_tail(&s2));
        assert_eq!(list.back(|x| *x), Some(2));
    }

    #[test]
    fn link_before_middle() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d3 = slab.create_node(3).unwrap();
        let d2 = slab.create_node(2).unwrap();

        // Insert 1, then 3, then insert 2 before 3
        let s1 = list.link_back(d1);
        let s3 = list.link_back(d3);
        let s2 = list.link_before(&s3, d2);

        // Order should be: 1, 2, 3
        assert_eq!(list.len(), 3);
        assert_eq!(list.front(|x| *x), Some(1));
        assert_eq!(list.back(|x| *x), Some(3));
        assert_eq!(list.read(&s2, |x| *x), 2);

        // Verify structure
        assert!(list.is_head(&s1));
        assert!(list.is_tail(&s3));
    }

    #[test]
    fn link_before_head() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d2 = slab.create_node(2).unwrap();
        let d1 = slab.create_node(1).unwrap();

        let s2 = list.link_back(d2);
        let s1 = list.link_before(&s2, d1);

        // 1 should now be head
        assert!(list.is_head(&s1));
        assert_eq!(list.front(|x| *x), Some(1));
    }

    // =========================================================================
    // Move Operation Tests
    // =========================================================================

    #[test]
    fn move_to_front_from_middle() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let s1 = list.link_back(d1);
        let s2 = list.link_back(d2);
        let s3 = list.link_back(d3);

        // Move 2 to front: order becomes 2, 1, 3
        list.move_to_front(&s2);

        // Verify positions
        assert!(list.is_head(&s2));
        assert!(list.is_tail(&s3));
        assert!(!list.is_head(&s1));
        assert!(!list.is_tail(&s1));

        // Verify values
        assert_eq!(list.front(|x| *x), Some(2));
        assert_eq!(list.back(|x| *x), Some(3));
        assert_eq!(list.read(&s1, |x| *x), 1);
    }

    #[test]
    fn move_to_front_from_tail() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let s1 = list.link_back(d1);
        let _s2 = list.link_back(d2);
        let s3 = list.link_back(d3);

        // Move 3 to front: order becomes 3, 1, 2
        list.move_to_front(&s3);

        assert!(list.is_head(&s3));
        assert!(list.is_tail(&_s2));
        assert!(!list.is_head(&s1));
    }

    #[test]
    fn move_to_front_already_at_front() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();

        let s1 = list.link_back(d1);
        let _s2 = list.link_back(d2);

        // Move head to front - should be no-op
        list.move_to_front(&s1);

        assert!(list.is_head(&s1));
        assert_eq!(list.front(|x| *x), Some(1));
        assert_eq!(list.back(|x| *x), Some(2));
    }

    #[test]
    fn move_to_back_from_middle() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let _s1 = list.link_back(d1);
        let s2 = list.link_back(d2);
        let _s3 = list.link_back(d3);

        // Move 2 to back: order becomes 1, 3, 2
        list.move_to_back(&s2);

        assert!(list.is_tail(&s2));
        assert_eq!(list.back(|x| *x), Some(2));
    }

    #[test]
    fn move_to_back_from_head() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let s1 = list.link_back(d1);
        let s2 = list.link_back(d2);
        let _s3 = list.link_back(d3);

        // Move 1 to back: order becomes 2, 3, 1
        list.move_to_back(&s1);

        assert!(list.is_tail(&s1));
        assert!(list.is_head(&s2));
    }

    #[test]
    fn move_to_back_already_at_back() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();

        let _s1 = list.link_back(d1);
        let s2 = list.link_back(d2);

        // Move tail to back - should be no-op
        list.move_to_back(&s2);

        assert!(list.is_tail(&s2));
        assert_eq!(list.front(|x| *x), Some(1));
        assert_eq!(list.back(|x| *x), Some(2));
    }

    #[test]
    fn move_before_basic() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let s1 = list.link_back(d1);
        let s2 = list.link_back(d2);
        let s3 = list.link_back(d3);

        // Move 3 before 2: order becomes 1, 3, 2
        list.move_before(&s3, &s2);

        // Verify positions
        assert!(list.is_head(&s1));
        assert!(list.is_tail(&s2));
        assert!(!list.is_head(&s3));
        assert!(!list.is_tail(&s3));

        // Verify values at endpoints
        assert_eq!(list.front(|x| *x), Some(1));
        assert_eq!(list.back(|x| *x), Some(2));
        assert_eq!(list.read(&s3, |x| *x), 3);
    }

    #[test]
    fn move_before_to_head() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let s1 = list.link_back(d1);
        let _s2 = list.link_back(d2);
        let s3 = list.link_back(d3);

        // Move 3 before 1 (head): order becomes 3, 1, 2
        list.move_before(&s3, &s1);

        assert!(list.is_head(&s3));
        assert_eq!(list.front(|x| *x), Some(3));
    }

    #[test]
    fn move_before_same_slot() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();

        let s1 = list.link_back(d1);
        let _s2 = list.link_back(d2);

        // Move 1 before 1 - should be no-op
        list.move_before(&s1, &s1);

        assert!(list.is_head(&s1));
        assert_eq!(list.front(|x| *x), Some(1));
    }

    #[test]
    fn move_before_already_before() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();

        let s1 = list.link_back(d1);
        let s2 = list.link_back(d2);

        // Move 1 before 2 - already in that position, should be no-op
        list.move_before(&s1, &s2);

        assert!(list.is_head(&s1));
        assert!(list.is_tail(&s2));
    }

    #[test]
    fn move_after_basic() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let s1 = list.link_back(d1);
        let s2 = list.link_back(d2);
        let s3 = list.link_back(d3);

        // Move 1 after 2: order becomes 2, 1, 3
        list.move_after(&s1, &s2);

        // Verify positions
        assert!(list.is_head(&s2));
        assert!(list.is_tail(&s3));
        assert!(!list.is_head(&s1));
        assert!(!list.is_tail(&s1));

        // Verify values at endpoints
        assert_eq!(list.front(|x| *x), Some(2));
        assert_eq!(list.back(|x| *x), Some(3));
        assert_eq!(list.read(&s1, |x| *x), 1);
    }

    #[test]
    fn move_after_to_tail() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();
        let d3 = slab.create_node(3).unwrap();

        let s1 = list.link_back(d1);
        let _s2 = list.link_back(d2);
        let s3 = list.link_back(d3);

        // Move 1 after 3 (tail): order becomes 2, 3, 1
        list.move_after(&s1, &s3);

        assert!(list.is_tail(&s1));
        assert_eq!(list.back(|x| *x), Some(1));
    }

    #[test]
    fn move_after_same_slot() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();

        let s1 = list.link_back(d1);
        let _s2 = list.link_back(d2);

        // Move 1 after 1 - should be no-op
        list.move_after(&s1, &s1);

        assert!(list.is_head(&s1));
    }

    #[test]
    fn move_after_already_after() {
        let slab = BoundedListSlab::<u64>::with_capacity(16);
        let mut list: List<u64, _> = List::new(slab);

        let d1 = slab.create_node(1).unwrap();
        let d2 = slab.create_node(2).unwrap();

        let s1 = list.link_back(d1);
        let s2 = list.link_back(d2);

        // Move 2 after 1 - already in that position, should be no-op
        list.move_after(&s2, &s1);

        assert!(list.is_head(&s1));
        assert!(list.is_tail(&s2));
    }
}
