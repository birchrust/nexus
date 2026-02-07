//! Doubly-linked list with `RcSlot`-based ownership.
//!
//! # Design
//!
//! - **User holds `RcSlot<ListNode<T>, A>`** — their ownership token + data access
//! - **List stores raw pointers** and maintains its own strong reference per node
//! - **Guard-based data access** via [`ExclusiveCell`] — no closures needed
//!
//! # Ownership Model
//!
//! Every linked node has `strong_count >= 2`:
//! - One from the user's `RcSlot` handle
//! - One from the list's internal bookkeeping
//!
//! Unlinking releases the list's ref. The user's handle remains valid.
//! Dropping the list releases all its refs. If the user already dropped
//! their handle, the node is cleaned up. If not, the user still has access.
//!
//! # Example
//!
//! ```ignore
//! use nexus_collections::list_allocator;
//!
//! struct Order { id: u64, price: f64 }
//!
//! mod orders {
//!     use super::*;
//!     list_allocator!(Order, bounded);
//! }
//!
//! orders::Allocator::builder().capacity(1000).build().unwrap();
//!
//! let mut list = orders::List::new();
//! let handle = orders::create_node(Order { id: 1, price: 100.0 }).unwrap();
//! list.link_back(&handle);
//!
//! // Read via auto-deref: RcSlot → ListNode → exclusive()
//! assert_eq!(handle.exclusive().price, 100.0);
//!
//! // Write
//! handle.exclusive_mut().price = 200.0;
//! ```

use std::cell::Cell;
use std::marker::PhantomData;
use std::ptr;

use nexus_slab::alloc::{Alloc, RcSlot};
use nexus_slab::{RcInner, SlotCell};

use crate::ExclusiveCell;
use crate::exclusive::{ExMut, ExRef};

// =============================================================================
// ListId
// =============================================================================

thread_local! {
    static NEXT_LIST_ID: Cell<usize> = const { Cell::new(0) };
}

/// Unique identifier for a list instance.
///
/// Each list gets a unique ID on creation. Nodes track which list owns them
/// to catch cross-list operations at runtime.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ListId(usize);

impl ListId {
    /// Sentinel value indicating no owner (detached state).
    pub const NONE: ListId = ListId(usize::MAX);

    /// Returns `true` if this is the NONE sentinel.
    #[inline]
    pub fn is_none(self) -> bool {
        self.0 == usize::MAX
    }

    /// Returns `true` if this is not the NONE sentinel.
    #[inline]
    pub fn is_some(self) -> bool {
        self.0 != usize::MAX
    }
}

fn next_list_id() -> ListId {
    NEXT_LIST_ID.with(|c| {
        let id = c.get();
        c.set(id.wrapping_add(1));
        ListId(id)
    })
}

// =============================================================================
// NodePtr
// =============================================================================

/// Raw pointer to a slab-allocated list node.
///
/// Stored in prev/next links. Raw pointers because `RcSlot` is not `Copy`
/// and cannot be stored in `Cell`.
type NodePtr<T> = *mut SlotCell<RcInner<ListNode<T>>>;

// =============================================================================
// ListNode<T>
// =============================================================================

/// A node in a doubly-linked list.
///
/// Contains `Cell`-based prev/next/owner links for interior mutability
/// and an [`ExclusiveCell`] for user data access.
///
/// # Data Access
///
/// Access user data through the convenience methods, reachable via
/// `RcSlot`'s auto-deref:
///
/// ```ignore
/// handle.exclusive().price       // shared borrow
/// handle.exclusive_mut().price = 5.0  // mutable borrow
/// ```
pub struct ListNode<T> {
    prev: Cell<NodePtr<T>>,
    next: Cell<NodePtr<T>>,
    owner: Cell<ListId>,
    data: ExclusiveCell<T>,
}

impl<T> ListNode<T> {
    /// Creates a new detached node wrapping the given value.
    #[inline]
    pub fn new(value: T) -> Self {
        ListNode {
            prev: Cell::new(ptr::null_mut()),
            next: Cell::new(ptr::null_mut()),
            owner: Cell::new(ListId::NONE),
            data: ExclusiveCell::new(value),
        }
    }

    /// Returns `true` if this node is linked to a list.
    #[inline]
    pub fn is_linked(&self) -> bool {
        self.owner.get().is_some()
    }

    /// Returns a reference to the underlying [`ExclusiveCell`].
    #[inline]
    pub fn data(&self) -> &ExclusiveCell<T> {
        &self.data
    }

    /// Consumes the node, returning the user data.
    ///
    /// Used internally by the `list_allocator!` macro's error path to recover
    /// the value from a `Full<ListNode<T>>` into `Full<T>`. Not intended for
    /// direct use.
    #[doc(hidden)]
    #[inline]
    pub fn into_data(self) -> T {
        // Only callable with an owned ListNode<T> — not reachable through
        // RcSlot (which derefs to &ListNode<T>). The sole call site is the
        // list_allocator! macro's Full error path, where allocation failed
        // and no RcSlot was ever created.
        self.data.into_inner()
    }

    /// Exclusive shared borrow of user data.
    #[inline]
    pub fn exclusive(&self) -> ExRef<'_, T> {
        self.data.borrow()
    }

    /// Exclusive mutable borrow of user data.
    #[inline]
    pub fn exclusive_mut(&self) -> ExMut<'_, T> {
        self.data.borrow_mut()
    }

    /// Try exclusive shared borrow.
    #[inline]
    pub fn try_exclusive(&self) -> Option<ExRef<'_, T>> {
        self.data.try_borrow()
    }

    /// Try exclusive mutable borrow.
    #[inline]
    pub fn try_exclusive_mut(&self) -> Option<ExMut<'_, T>> {
        self.data.try_borrow_mut()
    }

    // Internal accessors for list operations

    #[inline]
    fn prev_ptr(&self) -> NodePtr<T> {
        self.prev.get()
    }

    #[inline]
    fn next_ptr(&self) -> NodePtr<T> {
        self.next.get()
    }

    #[inline]
    fn owner_id(&self) -> ListId {
        self.owner.get()
    }

    #[inline]
    fn set_prev(&self, ptr: NodePtr<T>) {
        self.prev.set(ptr);
    }

    #[inline]
    fn set_next(&self, ptr: NodePtr<T>) {
        self.next.set(ptr);
    }

    #[inline]
    fn set_owner(&self, id: ListId) {
        self.owner.set(id);
    }
}

// =============================================================================
// node_deref — navigate raw pointer to *const ListNode<T>
// =============================================================================

/// Dereferences a `NodePtr<T>` to get `*const ListNode<T>`.
///
/// # Safety
///
/// - `ptr` must be non-null and point to an occupied `SlotCell` with `strong > 0`.
/// - The returned pointer is only valid as long as the caller (or the list)
///   holds a strong reference to the node.
#[inline]
unsafe fn node_deref<T>(ptr: NodePtr<T>) -> *const ListNode<T> {
    // SlotCell.value: ManuallyDrop<MaybeUninit<RcInner<ListNode<T>>>>
    // → assume_init_ref() → &RcInner<ListNode<T>>
    // → .value() → &ListNode<T> → cast to raw pointer
    //
    // SAFETY: Caller guarantees ptr is non-null and points to an occupied slot
    // with strong > 0.
    unsafe { (*ptr).value.assume_init_ref() }.value() as *const ListNode<T>
}

// =============================================================================
// List<T, A>
// =============================================================================

/// A doubly-linked list backed by slab-allocated `RcSlot` nodes.
///
/// # Ownership Model
///
/// - User holds `RcSlot<ListNode<T>, A>` handles — their ownership token
/// - List stores raw pointers and maintains its own strong reference per node
/// - Every linked node has `strong_count >= 2` (user + list)
/// - Unlinking decrements the list's ref; the user's handle remains valid
///
/// # Type Parameters
///
/// - `T`: Element type stored in nodes
/// - `A`: Allocator type (from `bounded_rc_allocator!` or `unbounded_rc_allocator!`)
pub struct List<T: 'static, A: Alloc<Item = RcInner<ListNode<T>>>> {
    head: NodePtr<T>,
    tail: NodePtr<T>,
    len: usize,
    id: ListId,
    _marker: PhantomData<A>,
}

impl<T: 'static, A: Alloc<Item = RcInner<ListNode<T>>>> List<T, A> {
    /// Creates a new empty list.
    #[inline]
    pub fn new() -> Self {
        List {
            head: ptr::null_mut(),
            tail: ptr::null_mut(),
            len: 0,
            id: next_list_id(),
            _marker: PhantomData,
        }
    }

    /// Returns the unique ID of this list.
    #[inline]
    pub fn id(&self) -> ListId {
        self.id
    }

    /// Returns the number of linked elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the list has no linked elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    // =========================================================================
    // Link operations
    // =========================================================================

    /// Links a node to the back of the list.
    ///
    /// The list acquires its own strong reference to the node.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if the node is already linked to a list.
    /// In release builds, this is an unchecked precondition.
    #[inline]
    pub fn link_back(&mut self, handle: &RcSlot<ListNode<T>, A>) {
        let ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(ptr) };
        debug_assert!(!node.is_linked(), "node is already linked to a list");

        node.set_prev(self.tail);
        node.set_next(ptr::null_mut());
        node.set_owner(self.id);

        if self.tail.is_null() {
            self.head = ptr;
        } else {
            // SAFETY: tail is non-null (checked above), list holds strong ref
            unsafe { (*node_deref(self.tail)).set_next(ptr) };
        }

        self.tail = ptr;
        self.len += 1;

        // SAFETY: ptr from a live RcSlot, strong >= 1; list acquires its own ref
        unsafe { RcSlot::<ListNode<T>, A>::increment_strong_count(ptr) };
    }

    /// Links a node to the front of the list.
    ///
    /// The list acquires its own strong reference to the node.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if the node is already linked to a list.
    /// In release builds, this is an unchecked precondition.
    #[inline]
    pub fn link_front(&mut self, handle: &RcSlot<ListNode<T>, A>) {
        let ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(ptr) };
        debug_assert!(!node.is_linked(), "node is already linked to a list");

        node.set_prev(ptr::null_mut());
        node.set_next(self.head);
        node.set_owner(self.id);

        if self.head.is_null() {
            self.tail = ptr;
        } else {
            // SAFETY: head is non-null (checked above), list holds strong ref
            unsafe { (*node_deref(self.head)).set_prev(ptr) };
        }

        self.head = ptr;
        self.len += 1;

        // SAFETY: ptr from a live RcSlot, strong >= 1; list acquires its own ref
        unsafe { RcSlot::<ListNode<T>, A>::increment_strong_count(ptr) };
    }

    /// Links a node immediately after an existing node.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `handle` is already linked or `after` is
    /// not in this list. In release builds, these are unchecked preconditions.
    #[inline]
    pub fn link_after(&mut self, after: &RcSlot<ListNode<T>, A>, handle: &RcSlot<ListNode<T>, A>) {
        let after_ptr = after.as_ptr();
        // SAFETY: after is a live RcSlot, strong >= 1
        let after_node = unsafe { &*node_deref(after_ptr) };
        debug_assert_eq!(
            after_node.owner_id(),
            self.id,
            "anchor node does not belong to this list"
        );

        let new_ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let new_node = unsafe { &*node_deref(new_ptr) };
        debug_assert!(!new_node.is_linked(), "node is already linked to a list");

        let next_ptr = after_node.next_ptr();

        new_node.set_prev(after_ptr);
        new_node.set_next(next_ptr);
        new_node.set_owner(self.id);

        after_node.set_next(new_ptr);

        if next_ptr.is_null() {
            self.tail = new_ptr;
        } else {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(next_ptr)).set_prev(new_ptr) };
        }

        self.len += 1;
        // SAFETY: ptr from a live RcSlot, strong >= 1; list acquires its own ref
        unsafe { RcSlot::<ListNode<T>, A>::increment_strong_count(new_ptr) };
    }

    /// Links a node immediately before an existing node.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `handle` is already linked or `before` is
    /// not in this list. In release builds, these are unchecked preconditions.
    #[inline]
    pub fn link_before(
        &mut self,
        before: &RcSlot<ListNode<T>, A>,
        handle: &RcSlot<ListNode<T>, A>,
    ) {
        let before_ptr = before.as_ptr();
        // SAFETY: before is a live RcSlot, strong >= 1
        let before_node = unsafe { &*node_deref(before_ptr) };
        debug_assert_eq!(
            before_node.owner_id(),
            self.id,
            "anchor node does not belong to this list"
        );

        let new_ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let new_node = unsafe { &*node_deref(new_ptr) };
        debug_assert!(!new_node.is_linked(), "node is already linked to a list");

        let prev_ptr = before_node.prev_ptr();

        new_node.set_prev(prev_ptr);
        new_node.set_next(before_ptr);
        new_node.set_owner(self.id);

        before_node.set_prev(new_ptr);

        if prev_ptr.is_null() {
            self.head = new_ptr;
        } else {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(prev_ptr)).set_next(new_ptr) };
        }

        self.len += 1;
        // SAFETY: ptr from a live RcSlot, strong >= 1; list acquires its own ref
        unsafe { RcSlot::<ListNode<T>, A>::increment_strong_count(new_ptr) };
    }

    // =========================================================================
    // Unlink
    // =========================================================================

    /// Unlinks a node from the list.
    ///
    /// The user's handle remains valid. The list releases its strong reference.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if the node is not linked to this list.
    /// In release builds, this is an unchecked precondition.
    #[inline]
    pub fn unlink(&mut self, handle: &RcSlot<ListNode<T>, A>) {
        let ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(ptr) };
        debug_assert_eq!(
            node.owner_id(),
            self.id,
            "node does not belong to this list"
        );

        self.unlink_ptr(ptr);
    }

    /// Internal: unlink by raw pointer. Clears node metadata and decrements
    /// the list's strong reference.
    ///
    /// # Preconditions
    ///
    /// - `ptr` must be non-null
    /// - `ptr` must point to a node currently linked to this list
    /// - The list holds a strong reference to this node
    #[inline]
    fn unlink_ptr(&mut self, ptr: NodePtr<T>) {
        // SAFETY: caller guarantees ptr is non-null and points to a linked node;
        // list holds a strong ref for all linked nodes
        let node = unsafe { &*node_deref(ptr) };
        let prev = node.prev_ptr();
        let next = node.next_ptr();

        if prev.is_null() {
            self.head = next;
        } else {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(prev)).set_next(next) };
        }

        if next.is_null() {
            self.tail = prev;
        } else {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(next)).set_prev(prev) };
        }

        node.set_prev(ptr::null_mut());
        node.set_next(ptr::null_mut());
        node.set_owner(ListId::NONE);

        self.len -= 1;

        // SAFETY: list owns a strong ref from link time; ptr is valid
        unsafe { RcSlot::<ListNode<T>, A>::decrement_strong_count(ptr) };
    }

    /// Unlinks all nodes from the list.
    ///
    /// Each node is detached and the list's strong reference is released.
    /// User handles remain valid.
    #[inline]
    pub fn clear(&mut self) {
        let mut current = self.head;
        while !current.is_null() {
            // SAFETY: current is non-null, list holds strong ref for all linked nodes
            let node = unsafe { &*node_deref(current) };
            let next = node.next_ptr();

            node.set_prev(ptr::null_mut());
            node.set_next(ptr::null_mut());
            node.set_owner(ListId::NONE);

            // SAFETY: list owns a strong ref for each linked node
            unsafe { RcSlot::<ListNode<T>, A>::decrement_strong_count(current) };

            current = next;
        }

        self.head = ptr::null_mut();
        self.tail = ptr::null_mut();
        self.len = 0;
    }

    // =========================================================================
    // Pop
    // =========================================================================

    /// Removes and returns the front node as an `RcSlot`.
    ///
    /// The returned handle carries the list's strong reference — no net
    /// refcount change. The node is detached.
    #[inline]
    pub fn pop_front(&mut self) -> Option<RcSlot<ListNode<T>, A>> {
        if self.head.is_null() {
            return None;
        }

        let ptr = self.head;
        // SAFETY: head is non-null (checked above), list holds strong ref
        let node = unsafe { &*node_deref(ptr) };
        let next = node.next_ptr();

        self.head = next;
        if next.is_null() {
            self.tail = ptr::null_mut();
        } else {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(next)).set_prev(ptr::null_mut()) };
        }

        node.set_prev(ptr::null_mut());
        node.set_next(ptr::null_mut());
        node.set_owner(ListId::NONE);

        self.len -= 1;

        // SAFETY: transferring list's strong ref into the returned RcSlot;
        // ptr is valid, list owned this ref since link time
        Some(unsafe { RcSlot::from_raw(ptr) })
    }

    /// Removes and returns the back node as an `RcSlot`.
    #[inline]
    pub fn pop_back(&mut self) -> Option<RcSlot<ListNode<T>, A>> {
        if self.tail.is_null() {
            return None;
        }

        let ptr = self.tail;
        // SAFETY: tail is non-null (checked above), list holds strong ref
        let node = unsafe { &*node_deref(ptr) };
        let prev = node.prev_ptr();

        self.tail = prev;
        if prev.is_null() {
            self.head = ptr::null_mut();
        } else {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(prev)).set_next(ptr::null_mut()) };
        }

        node.set_prev(ptr::null_mut());
        node.set_next(ptr::null_mut());
        node.set_owner(ListId::NONE);

        self.len -= 1;

        // SAFETY: transferring list's strong ref into the returned RcSlot;
        // ptr is valid, list owned this ref since link time
        Some(unsafe { RcSlot::from_raw(ptr) })
    }

    // =========================================================================
    // Peek
    // =========================================================================

    /// Returns a reference to the front node, or `None` if empty.
    ///
    /// Call `.exclusive()` on the node to access user data.
    #[inline]
    pub fn front(&self) -> Option<&ListNode<T>> {
        if self.head.is_null() {
            return None;
        }
        // SAFETY: head is non-null (checked above), list holds strong ref.
        // Reference lifetime bounded by &self.
        Some(unsafe { &*node_deref(self.head) })
    }

    /// Returns a reference to the back node, or `None` if empty.
    ///
    /// Call `.exclusive()` on the node to access user data.
    #[inline]
    pub fn back(&self) -> Option<&ListNode<T>> {
        if self.tail.is_null() {
            return None;
        }
        // SAFETY: tail is non-null (checked above), list holds strong ref.
        // Reference lifetime bounded by &self.
        Some(unsafe { &*node_deref(self.tail) })
    }

    // =========================================================================
    // Position checks
    // =========================================================================

    /// Returns `true` if the handle is linked to this list.
    #[inline]
    pub fn contains(&self, handle: &RcSlot<ListNode<T>, A>) -> bool {
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(handle.as_ptr()) };
        node.owner_id() == self.id
    }

    /// Returns `true` if the handle is at the head of this list.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if the node is not in this list.
    /// In release builds, this is an unchecked precondition.
    #[inline]
    pub fn is_head(&self, handle: &RcSlot<ListNode<T>, A>) -> bool {
        let ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(ptr) };
        debug_assert_eq!(
            node.owner_id(),
            self.id,
            "node does not belong to this list"
        );
        ptr == self.head
    }

    /// Returns `true` if the handle is at the tail of this list.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if the node is not in this list.
    /// In release builds, this is an unchecked precondition.
    #[inline]
    pub fn is_tail(&self, handle: &RcSlot<ListNode<T>, A>) -> bool {
        let ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(ptr) };
        debug_assert_eq!(
            node.owner_id(),
            self.id,
            "node does not belong to this list"
        );
        ptr == self.tail
    }

    // =========================================================================
    // Move operations
    // =========================================================================

    /// Moves a node to the front without changing ownership or refcounts.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if the node is not in this list.
    /// In release builds, this is an unchecked precondition.
    #[inline]
    pub fn move_to_front(&mut self, handle: &RcSlot<ListNode<T>, A>) {
        let ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(ptr) };
        debug_assert_eq!(
            node.owner_id(),
            self.id,
            "node does not belong to this list"
        );

        if ptr == self.head {
            return;
        }

        let prev = node.prev_ptr();
        let next = node.next_ptr();

        // Remove from current position
        if !prev.is_null() {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(prev)).set_next(next) };
        }
        if next.is_null() {
            self.tail = prev;
        } else {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(next)).set_prev(prev) };
        }

        // Insert at front
        node.set_prev(ptr::null_mut());
        node.set_next(self.head);
        if !self.head.is_null() {
            // SAFETY: head is non-null (checked), list holds strong ref
            unsafe { (*node_deref(self.head)).set_prev(ptr) };
        }
        self.head = ptr;
    }

    /// Moves a node to the back without changing ownership or refcounts.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if the node is not in this list.
    /// In release builds, this is an unchecked precondition.
    #[inline]
    pub fn move_to_back(&mut self, handle: &RcSlot<ListNode<T>, A>) {
        let ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(ptr) };
        debug_assert_eq!(
            node.owner_id(),
            self.id,
            "node does not belong to this list"
        );

        if ptr == self.tail {
            return;
        }

        let prev = node.prev_ptr();
        let next = node.next_ptr();

        // Remove from current position
        if !next.is_null() {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(next)).set_prev(prev) };
        }
        if prev.is_null() {
            self.head = next;
        } else {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(prev)).set_next(next) };
        }

        // Insert at back
        node.set_next(ptr::null_mut());
        node.set_prev(self.tail);
        if !self.tail.is_null() {
            // SAFETY: tail is non-null (checked), list holds strong ref
            unsafe { (*node_deref(self.tail)).set_next(ptr) };
        }
        self.tail = ptr;
    }

    // =========================================================================
    // Cursor entry points
    // =========================================================================

    /// Creates a cursor positioned before the first element.
    ///
    /// Call `advance()` to move to the head, or `advance_back()` to move to
    /// the tail.
    #[inline]
    pub fn cursor(&mut self) -> Cursor<'_, T, A> {
        Cursor {
            list: self,
            position: Position::BeforeStart,
        }
    }

    /// Creates a cursor positioned after the last element.
    ///
    /// Call `advance_back()` to move to the tail, or `advance()` to stay
    /// at `AfterEnd`.
    #[inline]
    pub fn cursor_back(&mut self) -> Cursor<'_, T, A> {
        Cursor {
            list: self,
            position: Position::AfterEnd,
        }
    }
}

impl<T: 'static, A: Alloc<Item = RcInner<ListNode<T>>>> Default for List<T, A> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: 'static, A: Alloc<Item = RcInner<ListNode<T>>>> Drop for List<T, A> {
    fn drop(&mut self) {
        self.clear();
    }
}

// =============================================================================
// Cursor
// =============================================================================

/// Position state for cursor traversal.
///
/// # Invariants
///
/// - `At(ptr)`: `ptr` is non-null and points to a node linked to the cursor's
///   list. The list holds a strong ref for all linked nodes, and `&mut List`
///   prevents concurrent mutation, so the pointer is valid for the cursor's
///   lifetime.
/// - `BeforeStart` / `AfterEnd`: sentinel states with no pointer.
enum Position<T> {
    /// Before the first element.
    BeforeStart,
    /// After the last element.
    AfterEnd,
    /// At a specific node.
    At(NodePtr<T>),
}

#[cold]
#[inline(never)]
fn panic_cursor_not_at_node() -> ! {
    panic!("cursor is not positioned at a node")
}

/// Cursor for positional traversal with modification support.
///
/// # Position States
///
/// - `BeforeStart`: Before the first element (from `list.cursor()`)
/// - `AfterEnd`: After the last element (from `list.cursor_back()`)
/// - `At(ptr)`: Positioned at an element
///
/// # Traversal API
///
/// - `advance()` / `advance_back()`: Move forward/backward, return `bool`
/// - `current()`: Peek at the current node without refcount traffic
/// - `remove()`: Remove the current node, auto-advance to next
///
/// # Example
///
/// ```ignore
/// let mut cursor = list.cursor();
/// cursor.advance(); // move to head
/// loop {
///     match cursor.current() {
///         None => break,
///         Some(node) if node.exclusive().price > 100.0 => {
///             cursor.remove(); // auto-advances to next
///         }
///         Some(_) => {
///             cursor.advance();
///         }
///     }
/// }
/// ```
pub struct Cursor<'a, T: 'static, A: Alloc<Item = RcInner<ListNode<T>>>> {
    list: &'a mut List<T, A>,
    position: Position<T>,
}

impl<T: 'static, A: Alloc<Item = RcInner<ListNode<T>>>> Cursor<'_, T, A> {
    /// Moves the cursor forward to the next node.
    ///
    /// Returns `true` if the cursor is now at a valid node, `false` if it
    /// moved past the end.
    #[inline]
    pub fn advance(&mut self) -> bool {
        let next_ptr = match self.position {
            Position::BeforeStart => self.list.head,
            Position::AfterEnd => return false,
            Position::At(ptr) => {
                // SAFETY: At(ptr) invariant — node is linked, list holds strong ref
                unsafe { (*node_deref(ptr)).next_ptr() }
            }
        };

        if next_ptr.is_null() {
            self.position = Position::AfterEnd;
            return false;
        }

        self.position = Position::At(next_ptr);
        true
    }

    /// Moves the cursor backward to the previous node.
    ///
    /// Returns `true` if the cursor is now at a valid node, `false` if it
    /// moved before the start.
    #[inline]
    pub fn advance_back(&mut self) -> bool {
        let prev_ptr = match self.position {
            Position::BeforeStart => return false,
            Position::AfterEnd => self.list.tail,
            Position::At(ptr) => {
                // SAFETY: At(ptr) invariant — node is linked, list holds strong ref
                unsafe { (*node_deref(ptr)).prev_ptr() }
            }
        };

        if prev_ptr.is_null() {
            self.position = Position::BeforeStart;
            return false;
        }

        self.position = Position::At(prev_ptr);
        true
    }

    /// Returns a reference to the current node, or `None` if the cursor is
    /// at `BeforeStart` or `AfterEnd`.
    ///
    /// No refcount traffic — borrows directly from the cursor.
    #[inline]
    pub fn current(&self) -> Option<&ListNode<T>> {
        match self.position {
            Position::At(ptr) => {
                // SAFETY: At(ptr) invariant — node is linked, list holds strong ref.
                // Reference lifetime bounded by &self.
                Some(unsafe { &*node_deref(ptr) })
            }
            _ => None,
        }
    }

    /// Removes the current node from the list. The cursor auto-advances to
    /// the next node (or `AfterEnd` if the removed node was the tail).
    ///
    /// Returns the removed node as an `RcSlot` (transfers the list's strong
    /// reference — no net refcount change).
    ///
    /// After removal, `current()` returns the next node. For backward
    /// scan-and-remove, use `advance_back()` instead of `advance()` after
    /// removal, since `advance()` would skip the node that `remove()` landed on.
    ///
    /// # Panics
    ///
    /// Panics if the cursor is not positioned at a node.
    #[inline]
    pub fn remove(&mut self) -> RcSlot<ListNode<T>, A> {
        let Position::At(ptr) = self.position else {
            panic_cursor_not_at_node();
        };

        // SAFETY: At(ptr) invariant — node is linked, list holds strong ref
        let node = unsafe { &*node_deref(ptr) };
        let prev = node.prev_ptr();
        let next = node.next_ptr();

        if prev.is_null() {
            self.list.head = next;
        } else {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(prev)).set_next(next) };
        }

        if next.is_null() {
            self.list.tail = prev;
        } else {
            // SAFETY: linked node, list holds strong ref for all linked nodes
            unsafe { (*node_deref(next)).set_prev(prev) };
        }

        node.set_prev(ptr::null_mut());
        node.set_next(ptr::null_mut());
        node.set_owner(ListId::NONE);
        self.list.len -= 1;

        // Auto-advance to next node (std semantics)
        if next.is_null() {
            self.position = Position::AfterEnd;
        } else {
            self.position = Position::At(next);
        }

        // SAFETY: transferring list's strong ref into returned RcSlot;
        // list owned this ref since link time
        unsafe { RcSlot::from_raw(ptr) }
    }

    /// Removes the current node if the predicate returns `true`.
    ///
    /// The predicate receives `&ListNode<T>` — call `.exclusive()` to read data.
    ///
    /// If removed: returns `Some(RcSlot)`, cursor auto-advances to next node.
    /// If not removed: returns `None`, cursor stays at the current node.
    ///
    /// # Panics
    ///
    /// Panics if the cursor is not positioned at a node.
    #[inline]
    pub fn remove_if(
        &mut self,
        f: impl FnOnce(&ListNode<T>) -> bool,
    ) -> Option<RcSlot<ListNode<T>, A>> {
        let Position::At(ptr) = self.position else {
            panic_cursor_not_at_node();
        };

        // SAFETY: At(ptr) invariant — node is linked, list holds strong ref
        let node = unsafe { &*node_deref(ptr) };
        if f(node) { Some(self.remove()) } else { None }
    }
}
