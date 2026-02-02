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
//! Use the [`create_list!`](crate::create_list) macro to create a typed list:
//!
//! ```ignore
//! use nexus_collections::create_list;
//!
//! // Define an allocator for Order lists
//! create_list!(orders, Order);
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
use core::marker::PhantomData;
use core::mem::ManuallyDrop;

use crate::internal::ListStorage;
use nexus_slab::Key;

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
pub struct DetachedListNode<T: 'static, A>
where
    A: ListStorage<T>,
{
    key: Key,
    _marker: PhantomData<(T, A)>,
}

impl<T: 'static, A> DetachedListNode<T, A>
where
    A: ListStorage<T>,
{
    /// Creates a new detached node from a key.
    ///
    /// # Safety
    ///
    /// The key must be valid and point to an occupied slot containing
    /// a detached node (owner == Id::NONE).
    #[doc(hidden)]
    #[inline]
    pub unsafe fn from_key(key: Key) -> Self {
        Self {
            key,
            _marker: PhantomData,
        }
    }

    /// Extracts the owned data, removing from slab. Consumes handle.
    #[inline]
    pub fn take(self) -> T {
        // Use ManuallyDrop to prevent Drop from running (which would double-free)
        let this = ManuallyDrop::new(self);
        unsafe { A::remove_unchecked(this.key) }.data
    }

    /// Returns the key for internal use.
    #[inline]
    pub(crate) fn key(&self) -> Key {
        self.key
    }

    /// Converts to a linked slot. Internal use only.
    #[inline]
    pub(crate) fn into_slot(self) -> ListSlot<T, A> {
        let this = ManuallyDrop::new(self);
        ListSlot {
            key: this.key,
            _marker: PhantomData,
        }
    }
}

impl<T: 'static, A> Drop for DetachedListNode<T, A>
where
    A: ListStorage<T>,
{
    fn drop(&mut self) {
        // Remove from slab on drop (correct cleanup for detached node)
        A::try_remove(self.key);
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
pub struct ListSlot<T: 'static, A>
where
    A: ListStorage<T>,
{
    key: Key,
    _marker: PhantomData<(T, A)>,
}

impl<T: 'static, A> ListSlot<T, A>
where
    A: ListStorage<T>,
{
    /// Returns the key for internal use.
    #[inline]
    pub(crate) fn key(&self) -> Key {
        self.key
    }

    /// Converts to detached. Internal use only (after unlink).
    #[inline]
    pub(crate) fn into_detached(self) -> DetachedListNode<T, A> {
        let this = ManuallyDrop::new(self);
        DetachedListNode {
            key: this.key,
            _marker: PhantomData,
        }
    }
}

impl<T: 'static, A> Drop for ListSlot<T, A>
where
    A: ListStorage<T>,
{
    fn drop(&mut self) {
        // Slot owns the slab entry - dropping removes it.
        // If still linked, this creates dangling prev/next pointers in the list.
        // Safe API will catch this on next access via get() returning None.
        // This is a user invariant violation, not a bug in our code.
        A::try_remove(self.key);
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
pub struct Detached<'a, T: 'static, A>
where
    A: ListStorage<T>,
{
    // Held for potential future re-linking operations
    #[allow(dead_code)]
    list: &'a mut List<T, A>,
    key: Key,
    _marker: PhantomData<&'a T>,
}

impl<T: 'static, A> Detached<'_, T, A>
where
    A: ListStorage<T>,
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
    pub fn take<F>(self, f: F) -> DetachedListNode<T, A>
    where
        F: FnOnce(&T) -> ListSlot<T, A>,
    {
        let this = ManuallyDrop::new(self);
        // SAFETY: Read-only access, no mutable refs exist to this slot
        let node = unsafe { A::get(this.key) }.expect("detached node was removed from slab");
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
    pub fn try_take<F>(self, f: F) -> Option<DetachedListNode<T, A>>
    where
        F: FnOnce(&T) -> Option<ListSlot<T, A>>,
    {
        let this = ManuallyDrop::new(self);
        // SAFETY: Read-only access, no mutable refs exist to this slot
        let data = match unsafe { A::get(this.key) } {
            Some(node) => &node.data,
            None => return None,
        };
        f(data).map(ListSlot::into_detached)
    }
}

impl<T: 'static, A> Drop for Detached<'_, T, A>
where
    A: ListStorage<T>,
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

/// A doubly-linked list backed by a TLS slab allocator.
///
/// # Type Parameters
///
/// - `T`: Element type
/// - `A`: Storage marker (generated by `create_list!` macro)
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
pub struct List<T: 'static, A>
where
    A: ListStorage<T>,
{
    head: Key,
    tail: Key,
    len: usize,
    id: Id,
    _marker: PhantomData<(T, A)>,
}

impl<T: 'static, A> List<T, A>
where
    A: ListStorage<T>,
{
    /// Creates a new empty list.
    #[inline]
    pub fn new() -> Self {
        Self {
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

    // =========================================================================
    // Link operations (DetachedListNode -> ListSlot)
    // =========================================================================

    /// Links a detached node to the back of the list.
    ///
    /// Consumes the `DetachedListNode` and returns a `ListSlot`.
    #[inline]
    pub fn link_back(&mut self, node: DetachedListNode<T, A>) -> ListSlot<T, A> {
        let key = node.key();

        // Set up links
        {
            let n = unsafe { A::get_unchecked_mut(key) };
            n.prev = self.tail;
            n.next = Key::NONE;
            n.owner = self.id;
        }

        // Update tail's next pointer
        if self.tail.is_some() {
            assert!(
                A::contains_key(self.tail),
                "list tail is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { A::get_unchecked_mut(self.tail) }.next = key;
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
    pub fn link_front(&mut self, node: DetachedListNode<T, A>) -> ListSlot<T, A> {
        let key = node.key();

        // Set up links
        {
            let n = unsafe { A::get_unchecked_mut(key) };
            n.prev = Key::NONE;
            n.next = self.head;
            n.owner = self.id;
        }

        // Update head's prev pointer
        if self.head.is_some() {
            assert!(
                A::contains_key(self.head),
                "list head is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { A::get_unchecked_mut(self.head) }.prev = key;
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
    pub fn unlink(&mut self, slot: ListSlot<T, A>) -> DetachedListNode<T, A> {
        let key = slot.key();

        // Validate and get links
        let (prev, next) = {
            let node = unsafe { A::get(key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            (node.prev, node.next)
        };

        // Validate all neighbors upfront before any mutations.
        if prev.is_some() {
            assert!(
                A::contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        if next.is_some() {
            assert!(
                A::contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }

        // All keys validated - now do mutations
        if prev.is_some() {
            unsafe { A::get_unchecked_mut(prev) }.next = next;
        } else {
            self.head = next;
        }

        if next.is_some() {
            unsafe { A::get_unchecked_mut(next) }.prev = prev;
        } else {
            self.tail = prev;
        }

        // Clear ownership
        unsafe { A::get_unchecked_mut(key) }.owner = Id::NONE;

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
    pub fn read<F, R>(&self, slot: &ListSlot<T, A>, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        // SAFETY: Read-only access, no mutable refs exist to this slot
        let node = unsafe { A::get(slot.key()) }
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
        let node =
            unsafe { A::get(self.head) }.expect("list head is invalid (internal corruption)");
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
        let node =
            unsafe { A::get(self.tail) }.expect("list tail is invalid (internal corruption)");
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
    pub fn write<F, R>(&mut self, slot: &mut ListSlot<T, A>, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let key = slot.key();
        // Validate first
        {
            let node = unsafe { A::get(key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
        }
        // SAFETY: Key is validated above, getting exclusive access
        let node = unsafe { A::get_unchecked_mut(key) };
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
        let node = unsafe { A::get_unchecked_mut(self.head) };
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
        let node = unsafe { A::get_unchecked_mut(self.tail) };
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
    pub fn is_head(&self, slot: &ListSlot<T, A>) -> bool {
        let key = slot.key();
        // Validate ownership
        let node =
            unsafe { A::get(key) }.expect("slot is invalid (was it dropped without unlinking?)");
        assert_eq!(node.owner, self.id, "slot belongs to different list");
        key == self.head
    }

    /// Returns `true` if the slot is at the tail of the list.
    ///
    /// # Panics
    ///
    /// Panics if the slot is invalid or doesn't belong to this list.
    #[inline]
    pub fn is_tail(&self, slot: &ListSlot<T, A>) -> bool {
        let key = slot.key();
        // Validate ownership
        let node =
            unsafe { A::get(key) }.expect("slot is invalid (was it dropped without unlinking?)");
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
        after: &ListSlot<T, A>,
        node: DetachedListNode<T, A>,
    ) -> ListSlot<T, A> {
        let after_key = after.key();
        let new_key = node.key();

        // Validate `after` and get its next pointer
        let next = {
            let n = unsafe { A::get(after_key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(n.owner, self.id, "slot belongs to different list");
            n.next
        };

        // Set up new node's links
        {
            let n = unsafe { A::get_unchecked_mut(new_key) };
            n.prev = after_key;
            n.next = next;
            n.owner = self.id;
        }

        // Update `after`'s next pointer
        unsafe { A::get_unchecked_mut(after_key) }.next = new_key;

        // Update next's prev pointer (or tail if inserting at end)
        if next.is_some() {
            assert!(
                A::contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { A::get_unchecked_mut(next) }.prev = new_key;
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
        before: &ListSlot<T, A>,
        node: DetachedListNode<T, A>,
    ) -> ListSlot<T, A> {
        let before_key = before.key();
        let new_key = node.key();

        // Validate `before` and get its prev pointer
        let prev = {
            let n = unsafe { A::get(before_key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(n.owner, self.id, "slot belongs to different list");
            n.prev
        };

        // Set up new node's links
        {
            let n = unsafe { A::get_unchecked_mut(new_key) };
            n.prev = prev;
            n.next = before_key;
            n.owner = self.id;
        }

        // Update `before`'s prev pointer
        unsafe { A::get_unchecked_mut(before_key) }.prev = new_key;

        // Update prev's next pointer (or head if inserting at start)
        if prev.is_some() {
            assert!(
                A::contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { A::get_unchecked_mut(prev) }.next = new_key;
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
    pub fn move_to_front(&mut self, slot: &ListSlot<T, A>) {
        let key = slot.key();

        // Validate slot and check if already at front
        let (prev, next) = {
            let node = unsafe { A::get(key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            if node.prev.is_none() {
                return; // Already at front
            }
            (node.prev, node.next)
        };

        // Validate all neighbors upfront before any mutations.
        assert!(
            A::contains_key(prev),
            "prev neighbor is invalid (was a slot dropped without unlinking?)"
        );
        if next.is_some() {
            assert!(
                A::contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        assert!(
            A::contains_key(self.head),
            "list head is invalid (internal corruption)"
        );

        // All keys validated - now do mutations

        // Remove from current position
        unsafe { A::get_unchecked_mut(prev) }.next = next;
        if next.is_some() {
            unsafe { A::get_unchecked_mut(next) }.prev = prev;
        } else {
            self.tail = prev;
        }

        // Insert at front
        {
            let n = unsafe { A::get_unchecked_mut(key) };
            n.prev = Key::NONE;
            n.next = self.head;
        }
        unsafe { A::get_unchecked_mut(self.head) }.prev = key;
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
    pub fn move_to_back(&mut self, slot: &ListSlot<T, A>) {
        let key = slot.key();

        // Validate slot and check if already at back
        let (prev, next) = {
            let node = unsafe { A::get(key) }
                .expect("slot is invalid (was it dropped without unlinking?)");
            assert_eq!(node.owner, self.id, "slot belongs to different list");
            if node.next.is_none() {
                return; // Already at back
            }
            (node.prev, node.next)
        };

        // Validate all neighbors upfront before any mutations.
        assert!(
            A::contains_key(next),
            "next neighbor is invalid (was a slot dropped without unlinking?)"
        );
        if prev.is_some() {
            assert!(
                A::contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
        }
        assert!(
            A::contains_key(self.tail),
            "list tail is invalid (internal corruption)"
        );

        // All keys validated - now do mutations

        // Remove from current position
        unsafe { A::get_unchecked_mut(next) }.prev = prev;
        if prev.is_some() {
            unsafe { A::get_unchecked_mut(prev) }.next = next;
        } else {
            self.head = next;
        }

        // Insert at back
        {
            let n = unsafe { A::get_unchecked_mut(key) };
            n.prev = self.tail;
            n.next = Key::NONE;
        }
        unsafe { A::get_unchecked_mut(self.tail) }.next = key;
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
    pub fn pop_front(&mut self) -> Option<Detached<'_, T, A>> {
        if self.head.is_none() {
            return None;
        }

        let key = self.head;

        // Get next before we modify
        let node = unsafe { A::get(key) }.expect("list head is invalid (internal corruption)");
        let next = node.next;

        // Update head
        self.head = next;
        if next.is_some() {
            assert!(
                A::contains_key(next),
                "next neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { A::get_unchecked_mut(next) }.prev = Key::NONE;
        } else {
            self.tail = Key::NONE;
        }

        // Clear ownership (node is now detached)
        unsafe { A::get_unchecked_mut(key) }.owner = Id::NONE;

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
    pub fn pop_back(&mut self) -> Option<Detached<'_, T, A>> {
        if self.tail.is_none() {
            return None;
        }

        let key = self.tail;

        // Get prev before we modify
        let node = unsafe { A::get(key) }.expect("list tail is invalid (internal corruption)");
        let prev = node.prev;

        // Update tail
        self.tail = prev;
        if prev.is_some() {
            assert!(
                A::contains_key(prev),
                "prev neighbor is invalid (was a slot dropped without unlinking?)"
            );
            unsafe { A::get_unchecked_mut(prev) }.next = Key::NONE;
        } else {
            self.head = Key::NONE;
        }

        // Clear ownership (node is now detached)
        unsafe { A::get_unchecked_mut(key) }.owner = Id::NONE;

        self.len -= 1;

        Some(Detached {
            list: self,
            key,
            _marker: PhantomData,
        })
    }
}

impl<T: 'static, A> Default for List<T, A>
where
    A: ListStorage<T>,
{
    fn default() -> Self {
        Self::new()
    }
}
