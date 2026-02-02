//! Doubly-linked list with TLS-based storage.
//!
//! # Design Overview
//!
//! - **Slab owns data** - All node data lives in a TLS slab
//! - **List manages links** - Just prev/next pointer bookkeeping
//! - **Slots are opaque** - No data access methods, just identity
//! - **Access via closures** - `list.read(&slot, f)`, `list.write(&mut slot, f)`
//! - **Borrow checker enforces safety** - `write(&mut self, &mut slot, f)` signature
//!
//! # Creating a List
//!
//! Use the [`list_allocator!`](crate::create_list) macro to create a typed list:
//!
//! ```ignore
//! use nexus_collections::create_list;
//!
//! // Define an allocator for Order lists
//! list_allocator!(orders, Order);
//!
//! // Initialize at startup
//! orders::init().bounded(1000).build();
//!
//! // Create and use the list
//! let mut list = orders::List::new();
//! let detached = orders::create_node(Order { id: 1, price: 100.0 })?;
//! let slot = list.link_back(detached);
//! let price = list.read(&slot, |o| o.price);
//! ```
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

use core::cell::Cell;
use core::mem::ManuallyDrop;

use nexus_slab::{Key, VTable};

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
    /// The user data.
    pub data: T,
    /// Previous node key (Key::NONE if head).
    pub prev: Key,
    /// Next node key (Key::NONE if tail).
    pub next: Key,
    /// List owner ID (Id::NONE if detached).
    pub owner: Id,
}

impl<T> Node<T> {
    /// Creates a new detached node with no owner.
    #[inline]
    pub fn detached(data: T) -> Self {
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
pub struct DetachedListNode<T: 'static> {
    key: Key,
    vtable: *const VTable<Node<T>>,
}

impl<T: 'static> DetachedListNode<T> {
    /// Creates a new detached node from a key and vtable pointer.
    ///
    /// # Safety
    ///
    /// - The key must be valid and point to an occupied slot containing
    ///   a detached node (owner == Id::NONE).
    /// - The vtable pointer must be valid for the lifetime of the allocator.
    #[doc(hidden)]
    #[inline]
    pub unsafe fn from_key_vtable(key: Key, vtable: *const VTable<Node<T>>) -> Self {
        Self { key, vtable }
    }

    /// Extracts the owned data, removing from slab. Consumes handle.
    #[inline]
    pub fn take(self) -> T {
        // Use ManuallyDrop to prevent Drop from running (which would double-free)
        let this = ManuallyDrop::new(self);
        // SAFETY: Key is valid, vtable is valid
        let slot_cell = unsafe { (*this.vtable).slot_ptr(this.key) };
        let node = unsafe { std::ptr::read((*slot_cell).value_ptr()) };
        unsafe { (*this.vtable).free(this.key) };
        node.data
    }

    /// Returns the key for internal use.
    #[inline]
    pub(crate) fn key(&self) -> Key {
        self.key
    }

    /// Converts to a linked slot. Internal use only.
    #[inline]
    pub(crate) fn into_slot(self) -> ListSlot<T> {
        let this = ManuallyDrop::new(self);
        ListSlot {
            key: this.key,
            vtable: this.vtable,
        }
    }
}

impl<T: 'static> Drop for DetachedListNode<T> {
    fn drop(&mut self) {
        // Remove from slab on drop (correct cleanup for detached node)
        // Check if key is valid first
        if unsafe { (*self.vtable).contains_key(self.key) } {
            // Read and drop the value
            let slot_cell = unsafe { (*self.vtable).slot_ptr(self.key) };
            unsafe { std::ptr::drop_in_place((*slot_cell).value_ptr_mut()) };
            unsafe { (*self.vtable).free(self.key) };
        }
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
pub struct ListSlot<T: 'static> {
    key: Key,
    vtable: *const VTable<Node<T>>,
}

impl<T: 'static> ListSlot<T> {
    /// Returns the key for internal use.
    #[inline]
    pub(crate) fn key(&self) -> Key {
        self.key
    }

    /// Converts to detached. Internal use only (after unlink).
    #[inline]
    pub(crate) fn into_detached(self) -> DetachedListNode<T> {
        let this = ManuallyDrop::new(self);
        DetachedListNode {
            key: this.key,
            vtable: this.vtable,
        }
    }
}

impl<T: 'static> Drop for ListSlot<T> {
    fn drop(&mut self) {
        // Slot owns the slab entry - dropping removes it.
        // If still linked, this creates dangling prev/next pointers in the list.
        // Safe API will catch this on next access via get() returning None.
        // This is a user invariant violation, not a bug in our code.
        if unsafe { (*self.vtable).contains_key(self.key) } {
            // Read and drop the value
            let slot_cell = unsafe { (*self.vtable).slot_ptr(self.key) };
            unsafe { std::ptr::drop_in_place((*slot_cell).value_ptr_mut()) };
            unsafe { (*self.vtable).free(self.key) };
        }
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
pub struct Detached<T: 'static> {
    key: Key,
    vtable: *const VTable<Node<T>>,
}

impl<T: 'static> Detached<T> {
    /// Creates a new Detached guard.
    ///
    /// # Safety
    ///
    /// - The key must point to a valid, unlinked node in the slab.
    /// - The vtable pointer must be valid.
    #[inline]
    pub(crate) unsafe fn new(key: Key, vtable: *const VTable<Node<T>>) -> Self {
        Self { key, vtable }
    }

    /// Take the slot back using the reference to identify it.
    ///
    /// The closure receives `&T` to identify which slot to retrieve.
    /// Return your `ListSlot` (removed from your HashMap).
    ///
    /// # Panics
    ///
    /// Panics if the closure panics. Use `try_take` for fallible lookups.
    #[inline]
    pub fn take<F>(self, f: F) -> DetachedListNode<T>
    where
        F: FnOnce(&T) -> ListSlot<T>,
    {
        let this = ManuallyDrop::new(self);
        // SAFETY: Read-only access, no mutable refs exist to this slot
        let slot_cell = unsafe { (*this.vtable).slot_ptr(this.key) };
        assert!(
            unsafe { (*slot_cell).is_occupied() },
            "detached node was removed from slab"
        );
        let node = unsafe { (*slot_cell).get_value() };
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
    pub fn try_take<F>(self, f: F) -> Option<DetachedListNode<T>>
    where
        F: FnOnce(&T) -> Option<ListSlot<T>>,
    {
        let this = ManuallyDrop::new(self);
        // SAFETY: Read-only access, no mutable refs exist to this slot
        if !unsafe { (*this.vtable).contains_key(this.key) } {
            return None;
        }
        let slot_cell = unsafe { (*this.vtable).slot_ptr(this.key) };
        let node = unsafe { (*slot_cell).get_value() };
        f(&node.data).map(ListSlot::into_detached)
    }
}

impl<T: 'static> Drop for Detached<T> {
    fn drop(&mut self) {
        // Trivial drop - no panic.
        // If not consumed, the popped node is orphaned in slab (leak).
        // We don't remove it here because take()/try_take() might still work.
    }
}

// =============================================================================
// List
// =============================================================================

/// A doubly-linked list backed by a TLS slab allocator.
///
/// # Type Parameters
///
/// - `T`: Element type
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
pub struct List<T: 'static> {
    head: Key,
    tail: Key,
    len: usize,
    id: Id,
    vtable: *const VTable<Node<T>>,
}

impl<T: 'static> List<T> {
    /// Creates a new empty list with the given vtable.
    ///
    /// # Safety
    ///
    /// The vtable pointer must be valid for the lifetime of the allocator.
    #[doc(hidden)]
    #[inline]
    pub unsafe fn with_vtable(vtable: *const VTable<Node<T>>) -> Self {
        Self {
            head: Key::NONE,
            tail: Key::NONE,
            len: 0,
            id: next_id(),
            vtable,
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

    // =========================================================================
    // Internal helpers
    // =========================================================================

    /// Gets a reference to a node by key (checked).
    #[inline]
    fn get_node(&self, key: Key) -> Option<&Node<T>> {
        if !unsafe { (*self.vtable).contains_key(key) } {
            return None;
        }
        let slot_cell = unsafe { (*self.vtable).slot_ptr(key) };
        Some(unsafe { (*slot_cell).get_value() })
    }

    /// Gets a mutable reference to a node by key (unchecked).
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn get_node_unchecked_mut(&self, key: Key) -> &mut Node<T> {
        unsafe {
            let slot_cell = (*self.vtable).slot_ptr(key);
            (*slot_cell).get_value_mut()
        }
    }

    /// Checks if a key is valid.
    #[inline]
    fn contains_key(&self, key: Key) -> bool {
        unsafe { (*self.vtable).contains_key(key) }
    }

    // =========================================================================
    // Link operations (DetachedListNode -> ListSlot)
    // =========================================================================

    /// Links a detached node to the back of the list.
    ///
    /// Consumes the `DetachedListNode` and returns a `ListSlot`.
    #[inline]
    pub fn link_back(&mut self, node: DetachedListNode<T>) -> ListSlot<T> {
        let key = node.key();

        // Set up links
        {
            let n = unsafe { self.get_node_unchecked_mut(key) };
            n.prev = self.tail;
            n.next = Key::NONE;
            n.owner = self.id;
        }

        // Update tail's next pointer
        if self.tail.is_some() {
            assert!(
                self.contains_key(self.tail),
                "list tail is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.get_node_unchecked_mut(self.tail) }.next = key;
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
    pub fn link_front(&mut self, node: DetachedListNode<T>) -> ListSlot<T> {
        let key = node.key();

        // Set up links
        {
            let n = unsafe { self.get_node_unchecked_mut(key) };
            n.prev = Key::NONE;
            n.next = self.head;
            n.owner = self.id;
        }

        // Update head's prev pointer
        if self.head.is_some() {
            assert!(
                self.contains_key(self.head),
                "list head is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.get_node_unchecked_mut(self.head) }.prev = key;
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
    pub fn unlink(&mut self, slot: ListSlot<T>) -> DetachedListNode<T> {
        let key = slot.key();

        // Validate and get links
        let (prev, next) = {
            let node = self
                .get_node(key)
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            (node.prev, node.next)
        };

        // Validate all neighbors upfront before any mutations.
        if prev.is_some() {
            assert!(
                self.contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        if next.is_some() {
            assert!(
                self.contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }

        // All keys validated - now do mutations
        if prev.is_some() {
            unsafe { self.get_node_unchecked_mut(prev) }.next = next;
        } else {
            self.head = next;
        }

        if next.is_some() {
            unsafe { self.get_node_unchecked_mut(next) }.prev = prev;
        } else {
            self.tail = prev;
        }

        // Clear ownership
        unsafe { self.get_node_unchecked_mut(key) }.owner = Id::NONE;

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
    pub fn read<F, R>(&self, slot: &ListSlot<T>, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        // SAFETY: Read-only access, no mutable refs exist to this slot
        let node = self
            .get_node(slot.key())
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
        let node = self
            .get_node(self.head)
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
        let node = self
            .get_node(self.tail)
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
    pub fn write<F, R>(&mut self, slot: &mut ListSlot<T>, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let key = slot.key();
        // Validate first
        {
            let node = self
                .get_node(key)
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
        }
        // SAFETY: Key is validated above, getting exclusive access
        let node = unsafe { self.get_node_unchecked_mut(key) };
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
        let node = unsafe { self.get_node_unchecked_mut(self.head) };
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
        let node = unsafe { self.get_node_unchecked_mut(self.tail) };
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
    pub fn is_head(&self, slot: &ListSlot<T>) -> bool {
        let key = slot.key();
        // Validate ownership
        let node = self
            .get_node(key)
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
    pub fn is_tail(&self, slot: &ListSlot<T>) -> bool {
        let key = slot.key();
        // Validate ownership
        let node = self
            .get_node(key)
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
    pub fn link_after(&mut self, after: &ListSlot<T>, node: DetachedListNode<T>) -> ListSlot<T> {
        let after_key = after.key();
        let new_key = node.key();

        // Validate `after` and get its next pointer
        let next = {
            let n = self
                .get_node(after_key)
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(n.owner, self.id, "slot belongs to different list");
            n.next
        };

        // Set up new node's links
        {
            let n = unsafe { self.get_node_unchecked_mut(new_key) };
            n.prev = after_key;
            n.next = next;
            n.owner = self.id;
        }

        // Update `after`'s next pointer
        unsafe { self.get_node_unchecked_mut(after_key) }.next = new_key;

        // Update next's prev pointer (or tail if inserting at end)
        if next.is_some() {
            assert!(
                self.contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.get_node_unchecked_mut(next) }.prev = new_key;
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
    pub fn link_before(&mut self, before: &ListSlot<T>, node: DetachedListNode<T>) -> ListSlot<T> {
        let before_key = before.key();
        let new_key = node.key();

        // Validate `before` and get its prev pointer
        let prev = {
            let n = self
                .get_node(before_key)
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(n.owner, self.id, "slot belongs to different list");
            n.prev
        };

        // Set up new node's links
        {
            let n = unsafe { self.get_node_unchecked_mut(new_key) };
            n.prev = prev;
            n.next = before_key;
            n.owner = self.id;
        }

        // Update `before`'s prev pointer
        unsafe { self.get_node_unchecked_mut(before_key) }.prev = new_key;

        // Update prev's next pointer (or head if inserting at start)
        if prev.is_some() {
            assert!(
                self.contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.get_node_unchecked_mut(prev) }.next = new_key;
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
    pub fn move_to_front(&mut self, slot: &ListSlot<T>) {
        let key = slot.key();

        // Validate slot and check if already at front
        let (prev, next) = {
            let node = self
                .get_node(key)
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            if node.prev.is_none() {
                return; // Already at front
            }
            (node.prev, node.next)
        };

        // Validate all neighbors upfront before any mutations.
        assert!(
            self.contains_key(prev),
            "prev neighbor is invalid (was a slot dropped without unlinking?)"
        );
        if next.is_some() {
            assert!(
                self.contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        assert!(
            self.contains_key(self.head),
            "list head is invalid (internal corruption)"
        );

        // All keys validated - now do mutations

        // Remove from current position
        unsafe { self.get_node_unchecked_mut(prev) }.next = next;
        if next.is_some() {
            unsafe { self.get_node_unchecked_mut(next) }.prev = prev;
        } else {
            self.tail = prev;
        }

        // Insert at front
        {
            let n = unsafe { self.get_node_unchecked_mut(key) };
            n.prev = Key::NONE;
            n.next = self.head;
        }
        unsafe { self.get_node_unchecked_mut(self.head) }.prev = key;
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
    pub fn move_to_back(&mut self, slot: &ListSlot<T>) {
        let key = slot.key();

        // Validate slot and check if already at back
        let (prev, next) = {
            let node = self
                .get_node(key)
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            if node.next.is_none() {
                return; // Already at back
            }
            (node.prev, node.next)
        };

        // Validate all neighbors upfront before any mutations.
        assert!(
            self.contains_key(next),
            "next neighbor is invalid (was a slot dropped without unlinking?)"
        );
        if prev.is_some() {
            assert!(
                self.contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        assert!(
            self.contains_key(self.tail),
            "list tail is invalid (internal corruption)"
        );

        // All keys validated - now do mutations

        // Remove from current position
        unsafe { self.get_node_unchecked_mut(next) }.prev = prev;
        if prev.is_some() {
            unsafe { self.get_node_unchecked_mut(prev) }.next = next;
        } else {
            self.head = next;
        }

        // Insert at back
        {
            let n = unsafe { self.get_node_unchecked_mut(key) };
            n.prev = self.tail;
            n.next = Key::NONE;
        }
        unsafe { self.get_node_unchecked_mut(self.tail) }.next = key;
        self.tail = key;
    }

    // =========================================================================
    // Pop operations (returns Detached guard)
    // =========================================================================

    /// Pops the front element.
    ///
    /// Returns a `Detached` guard. Call `take()` or `try_take()` to complete
    /// the transition and get your slot back.
    #[inline]
    pub fn pop_front(&mut self) -> Option<Detached<T>> {
        if self.head.is_none() {
            return None;
        }

        let key = self.head;

        // Get next before we modify
        let node = self
            .get_node(key)
            .expect("list head is invalid (internal corruption)");
        let next = node.next;

        // Update head
        self.head = next;
        if next.is_some() {
            assert!(
                self.contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.get_node_unchecked_mut(next) }.prev = Key::NONE;
        } else {
            self.tail = Key::NONE;
        }

        // Clear ownership (node is now detached)
        unsafe { self.get_node_unchecked_mut(key) }.owner = Id::NONE;

        self.len -= 1;

        // SAFETY: key is valid and node is now unlinked
        Some(unsafe { Detached::new(key, self.vtable) })
    }

    /// Pops the back element.
    ///
    /// Returns a `Detached` guard. Call `take()` or `try_take()` to complete
    /// the transition and get your slot back.
    #[inline]
    pub fn pop_back(&mut self) -> Option<Detached<T>> {
        if self.tail.is_none() {
            return None;
        }

        let key = self.tail;

        // Get prev before we modify
        let node = self
            .get_node(key)
            .expect("list tail is invalid (internal corruption)");
        let prev = node.prev;

        // Update tail
        self.tail = prev;
        if prev.is_some() {
            assert!(
                self.contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { self.get_node_unchecked_mut(prev) }.next = Key::NONE;
        } else {
            self.head = Key::NONE;
        }

        // Clear ownership (node is now detached)
        unsafe { self.get_node_unchecked_mut(key) }.owner = Id::NONE;

        self.len -= 1;

        // SAFETY: key is valid and node is now unlinked
        Some(unsafe { Detached::new(key, self.vtable) })
    }

    // =========================================================================
    // Cursor operations
    // =========================================================================

    /// Creates a cursor positioned before the first element.
    ///
    /// Call `cursor.next()` to get a guard to the first element.
    #[inline]
    pub fn cursor(&mut self) -> Cursor<'_, T> {
        Cursor {
            list: self,
            position: Position::BeforeStart,
        }
    }

    /// Creates a cursor positioned after the last element.
    ///
    /// Call `cursor.prev()` to get a guard to the last element.
    #[inline]
    pub fn cursor_back(&mut self) -> Cursor<'_, T> {
        Cursor {
            list: self,
            position: Position::AfterEnd,
        }
    }
}

// Note: Default impl removed - List requires vtable from list_allocator! macro

// =============================================================================
// Cursor - Positional traversal with modification support
// =============================================================================

/// Position state for cursor traversal.
#[derive(Clone, Copy, Debug)]
enum Position {
    /// Before the first element. `next()` returns the head.
    BeforeStart,
    /// After the last element. `prev()` returns the tail.
    AfterEnd,
    /// At a specific element. `next()`/`prev()` follow links.
    At(Key),
    /// In a gap after removal. Stores neighbors for bidirectional traversal.
    Gap { prev: Key, next: Key },
}

/// Cursor for list traversal with modification support.
///
/// A cursor provides positional iteration over the list with the ability to
/// read, write, and remove elements. All guard operations consume the guard,
/// requiring `next()` or `prev()` to continue iteration.
///
/// # Position States
///
/// - `BeforeStart`: Before the first element (initial state from `cursor()`)
/// - `AfterEnd`: After the last element (initial state from `cursor_back()`)
/// - `At(key)`: Positioned at an element
/// - `Gap`: After removal, tracks neighbors for continued traversal
///
/// # Example
///
/// ```ignore
/// let mut cursor = list.cursor();
/// while let Some(guard) = cursor.next() {
///     if let Some(detached) = guard.write_remove_if(|order| {
///         order.qty -= fill_qty;
///         order.qty == 0  // Remove if fully filled
///     }) {
///         let node = detached.take(|o| index.remove(&o.id).unwrap());
///         process(node.take());
///     }
/// }
/// ```
pub struct Cursor<'a, T: 'static> {
    list: &'a mut List<T>,
    position: Position,
}

impl<'a, T: 'static> Cursor<'a, T> {
    /// Move forward and return a guard to the new position.
    ///
    /// Returns `None` if at the end of the list.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<CursorGuard<'_, 'a, T>> {
        let next_key = match self.position {
            Position::BeforeStart => self.list.head,
            Position::AfterEnd => return None,
            Position::At(key) => {
                // Get next from current position
                self.list.get_node(key)?.next
            }
            Position::Gap { next, .. } => next,
        };

        if next_key.is_none() {
            self.position = Position::AfterEnd;
            return None;
        }

        // Validate the key exists
        if !self.list.contains_key(next_key) {
            self.position = Position::AfterEnd;
            return None;
        }

        self.position = Position::At(next_key);
        Some(CursorGuard {
            cursor: self,
            key: next_key,
        })
    }

    /// Move backward and return a guard to the new position.
    ///
    /// Returns `None` if at the start of the list.
    #[inline]
    pub fn prev(&mut self) -> Option<CursorGuard<'_, 'a, T>> {
        let prev_key = match self.position {
            Position::BeforeStart => return None,
            Position::AfterEnd => self.list.tail,
            Position::At(key) => {
                // Get prev from current position
                self.list.get_node(key)?.prev
            }
            Position::Gap { prev, .. } => prev,
        };

        if prev_key.is_none() {
            self.position = Position::BeforeStart;
            return None;
        }

        // Validate the key exists
        if !self.list.contains_key(prev_key) {
            self.position = Position::BeforeStart;
            return None;
        }

        self.position = Position::At(prev_key);
        Some(CursorGuard {
            cursor: self,
            key: prev_key,
        })
    }
}

/// Guard for cursor position.
///
/// All operations consume the guard. After any operation, call `cursor.next()`
/// or `cursor.prev()` to continue iteration.
///
/// # User Invariant
///
/// You should consume this guard via one of its methods. Dropping without
/// consuming leaves the cursor in an inconsistent state.
pub struct CursorGuard<'cursor, 'list, T: 'static> {
    cursor: &'cursor mut Cursor<'list, T>,
    key: Key,
}

impl<T: 'static> CursorGuard<'_, '_, T> {
    /// Read the element via closure. Cursor stays at current position.
    #[inline]
    pub fn read<F, R>(self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        // Position stays At(key)
        let node = self
            .cursor
            .list
            .get_node(self.key)
            .expect("cursor position is invalid");
        f(&node.data)
    }

    /// Write to the element via closure. Cursor stays at current position.
    #[inline]
    pub fn write<F, R>(self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        // Position stays At(key)
        let node = unsafe { self.cursor.list.get_node_unchecked_mut(self.key) };
        f(&mut node.data)
    }

    /// Skip without accessing. Cursor stays at current position.
    #[inline]
    pub fn skip(self) {
        // Position stays At(key) - nothing to do
    }

    /// Remove the element. Cursor enters Gap state.
    ///
    /// Returns a `Detached` guard to complete index cleanup.
    #[inline]
    pub fn remove(self) -> Detached<T> {
        let key = self.key;
        let list = &mut *self.cursor.list;

        // Get neighbors before unlinking
        let node = list.get_node(key).expect("cursor position is invalid");
        let prev = node.prev;
        let next = node.next;

        // Unlink the node (similar to unlink() but we update cursor position)
        // Update prev's next pointer
        if prev.is_some() {
            unsafe { list.get_node_unchecked_mut(prev) }.next = next;
        } else {
            list.head = next;
        }

        // Update next's prev pointer
        if next.is_some() {
            unsafe { list.get_node_unchecked_mut(next) }.prev = prev;
        } else {
            list.tail = prev;
        }

        // Clear ownership
        unsafe { list.get_node_unchecked_mut(key) }.owner = Id::NONE;

        list.len -= 1;

        // Update cursor to Gap state
        self.cursor.position = Position::Gap { prev, next };

        // SAFETY: key is valid and node is now unlinked
        unsafe { Detached::new(key, list.vtable) }
    }

    /// Read, then remove if predicate returns true.
    ///
    /// If removed, cursor enters Gap state and returns `Some(Detached)`.
    /// If not removed, cursor stays at current position and returns `None`.
    #[inline]
    pub fn read_remove_if<F>(self, f: F) -> Option<Detached<T>>
    where
        F: FnOnce(&T) -> bool,
    {
        let key = self.key;
        let list = &mut *self.cursor.list;

        let node = list.get_node(key).expect("cursor position is invalid");
        let should_remove = f(&node.data);

        if should_remove {
            let prev = node.prev;
            let next = node.next;

            // Unlink
            if prev.is_some() {
                unsafe { list.get_node_unchecked_mut(prev) }.next = next;
            } else {
                list.head = next;
            }

            if next.is_some() {
                unsafe { list.get_node_unchecked_mut(next) }.prev = prev;
            } else {
                list.tail = prev;
            }

            unsafe { list.get_node_unchecked_mut(key) }.owner = Id::NONE;
            list.len -= 1;

            self.cursor.position = Position::Gap { prev, next };
            Some(unsafe { Detached::new(key, list.vtable) })
        } else {
            // Stay at current position
            None
        }
    }

    /// Write, then remove if predicate returns true.
    ///
    /// If removed, cursor enters Gap state and returns `Some(Detached)`.
    /// If not removed, cursor stays at current position and returns `None`.
    #[inline]
    pub fn write_remove_if<F>(self, f: F) -> Option<Detached<T>>
    where
        F: FnOnce(&mut T) -> bool,
    {
        let key = self.key;
        let list = &mut *self.cursor.list;

        // Get neighbors first (need them if we remove)
        let node = list.get_node(key).expect("cursor position is invalid");
        let prev = node.prev;
        let next = node.next;

        // Write and check predicate
        let node_mut = unsafe { list.get_node_unchecked_mut(key) };
        let should_remove = f(&mut node_mut.data);

        if should_remove {
            // Unlink
            if prev.is_some() {
                unsafe { list.get_node_unchecked_mut(prev) }.next = next;
            } else {
                list.head = next;
            }

            if next.is_some() {
                unsafe { list.get_node_unchecked_mut(next) }.prev = prev;
            } else {
                list.tail = prev;
            }

            unsafe { list.get_node_unchecked_mut(key) }.owner = Id::NONE;
            list.len -= 1;

            self.cursor.position = Position::Gap { prev, next };
            Some(unsafe { Detached::new(key, list.vtable) })
        } else {
            // Stay at current position
            None
        }
    }
}
