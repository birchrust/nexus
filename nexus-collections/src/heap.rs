//! Min-heap (pairing heap) with `RcSlot`-based stable handles.
//!
//! # Design
//!
//! A pairing heap — each node stores parent, leftmost-child, and sibling
//! pointers. This gives O(1) push, O(log n) amortized pop.
//! Two-pass pairing merge for optimal amortized bounds.
//!
//! # Ownership Model
//!
//! Same as [`List`](crate::list::List):
//! - User holds `RcSlot<HeapNode<T>, A>` — their ownership token + data access
//! - Heap stores raw pointers and maintains its own strong reference per node
//! - Every heap-linked node has `strong_count >= 2` (user + heap)
//! - Pop transfers the heap's ref to the caller (no net refcount change)
//!
//! # Immutable Data
//!
//! Heap node data is immutable once created. To change priority, unlink
//! the old node and push a new one.

use std::cell::Cell;
use std::marker::PhantomData;
use std::ptr;

use nexus_slab::alloc::{Alloc, RcSlot};
use nexus_slab::{BoundedAlloc, Full, RcInner, SlotCell, UnboundedAlloc};

use crate::next_collection_id;

// =============================================================================
// NodePtr
// =============================================================================

/// Raw pointer to a slab-allocated heap node.
type NodePtr<T> = *mut SlotCell<RcInner<HeapNode<T>>>;

// =============================================================================
// HeapNode<T>
// =============================================================================

/// A node in a pairing heap.
///
/// Contains `Cell`-based parent/child/sibling links for interior mutability
/// and immutable user data.
///
/// # Data Access
///
/// Access user data via `RcSlot`'s auto-deref:
///
/// ```ignore
/// let deadline = handle.data().deadline;
/// ```
pub struct HeapNode<T> {
    parent: Cell<NodePtr<T>>,
    child: Cell<NodePtr<T>>,
    next: Cell<NodePtr<T>>,
    prev: Cell<NodePtr<T>>,
    owner: Cell<usize>,
    data: T,
}

impl<T> HeapNode<T> {
    /// Creates a new detached node wrapping the given value.
    #[inline]
    pub fn new(value: T) -> Self {
        HeapNode {
            parent: Cell::new(ptr::null_mut()),
            child: Cell::new(ptr::null_mut()),
            next: Cell::new(ptr::null_mut()),
            prev: Cell::new(ptr::null_mut()),
            owner: Cell::new(0),
            data: value,
        }
    }

    /// Returns `true` if this node is linked to a heap.
    #[inline]
    pub fn is_linked(&self) -> bool {
        self.owner.get() != 0
    }

    /// Returns a reference to the user data.
    #[inline]
    pub fn data(&self) -> &T {
        &self.data
    }

    /// Consumes the node, returning the user data.
    ///
    /// Used internally by the `heap_allocator!` macro's error path to recover
    /// the value from a `Full<HeapNode<T>>` into `Full<T>`.
    #[doc(hidden)]
    #[inline]
    pub fn into_data(self) -> T {
        self.data
    }

    // Internal accessors

    #[inline]
    fn parent_ptr(&self) -> NodePtr<T> {
        self.parent.get()
    }

    #[inline]
    fn child_ptr(&self) -> NodePtr<T> {
        self.child.get()
    }

    #[inline]
    fn next_ptr(&self) -> NodePtr<T> {
        self.next.get()
    }

    #[inline]
    fn prev_ptr(&self) -> NodePtr<T> {
        self.prev.get()
    }

    #[inline]
    fn set_parent(&self, ptr: NodePtr<T>) {
        self.parent.set(ptr);
    }

    #[inline]
    fn set_child(&self, ptr: NodePtr<T>) {
        self.child.set(ptr);
    }

    #[inline]
    fn set_next(&self, ptr: NodePtr<T>) {
        self.next.set(ptr);
    }

    #[inline]
    fn set_prev(&self, ptr: NodePtr<T>) {
        self.prev.set(ptr);
    }

    #[inline]
    fn owner_id(&self) -> usize {
        self.owner.get()
    }

    #[inline]
    fn set_owner(&self, id: usize) {
        self.owner.set(id);
    }

}

// =============================================================================
// node_deref — navigate raw pointer to *const HeapNode<T>
// =============================================================================

/// Dereferences a `NodePtr<T>` to get `*const HeapNode<T>`.
///
/// # Safety
///
/// - `ptr` must be non-null and point to an occupied `SlotCell` with `strong > 0`.
/// - The returned pointer is only valid as long as the caller (or the heap)
///   holds a strong reference to the node.
#[inline]
unsafe fn node_deref<T>(ptr: NodePtr<T>) -> *const HeapNode<T> {
    // SlotCell.value: ManuallyDrop<MaybeUninit<RcInner<HeapNode<T>>>>
    // → assume_init_ref() → &RcInner<HeapNode<T>>
    // → .value() → &HeapNode<T> → cast to raw pointer
    //
    // SAFETY: Caller guarantees ptr is non-null and points to an occupied slot
    // with strong > 0.
    unsafe { (*ptr).value.assume_init_ref() }.value() as *const HeapNode<T>
}

// =============================================================================
// Cold panic helpers — extracted to prevent stack frame bloat on happy path
// =============================================================================

#[cold]
#[inline(never)]
fn panic_wrong_heap() -> ! {
    panic!("node is not linked to this heap")
}

#[cold]
#[inline(never)]
fn panic_already_linked() -> ! {
    panic!("node is already linked to a collection")
}

// =============================================================================
// link — merge two heap roots
// =============================================================================

/// Links two non-null heap roots. The smaller root wins; the loser is
/// prepended as the winner's leftmost child.
///
/// After link, the loser's parent/prev/next/child are fully correct.
/// The winner's `child` is updated but its parent/prev/next are **not
/// modified** — the caller must handle those if they may be stale.
///
/// # Safety
///
/// - Both `a` and `b` must be non-null, valid heap nodes with `strong > 0`.
#[inline]
unsafe fn link<T: Ord>(a: NodePtr<T>, b: NodePtr<T>) -> NodePtr<T> {
    debug_assert!(!a.is_null() && !b.is_null());

    // SAFETY: both pointers are valid heap nodes with strong > 0.
    // Data is immutable — no aliasing concern.
    let a_node = unsafe { &*node_deref(a) };
    let b_node = unsafe { &*node_deref(b) };

    let (winner, winner_node, loser, loser_node) = if a_node.data() <= b_node.data() {
        (a, a_node, b, b_node)
    } else {
        (b, b_node, a, a_node)
    };

    // Prepend loser as leftmost child of winner
    // SAFETY: both are valid nodes with strong > 0
    unsafe {
        let old_child = winner_node.child_ptr();

        loser_node.set_next(old_child);
        loser_node.set_prev(ptr::null_mut());
        loser_node.set_parent(winner);

        if !old_child.is_null() {
            (*node_deref(old_child)).set_prev(loser);
        }

        winner_node.set_child(loser);
    }

    winner
}

// =============================================================================
// cut — detach node from its parent's child list
// =============================================================================

/// Removes a node from its parent's child/sibling list.
///
/// After cut, the node's parent/prev/next are all null. The node's
/// children are untouched.
///
/// # Safety
///
/// - `ptr` must be non-null and point to a valid linked node (not root).
#[inline]
unsafe fn cut<T>(ptr: NodePtr<T>) {
    // SAFETY: caller guarantees ptr is non-null and valid
    unsafe {
        let node = node_deref(ptr);
        let parent = (*node).parent_ptr();
        let prev = (*node).prev_ptr();
        let next = (*node).next_ptr();

        if prev.is_null() {
            // Leftmost child — update parent's child pointer
            if !parent.is_null() {
                (*node_deref(parent)).set_child(next);
            }
        } else {
            (*node_deref(prev)).set_next(next);
        }

        if !next.is_null() {
            (*node_deref(next)).set_prev(prev);
        }

        (*node).set_parent(ptr::null_mut());
        (*node).set_prev(ptr::null_mut());
        (*node).set_next(ptr::null_mut());
    }
}

// =============================================================================
// merge_pairs — two-pass pairing merge
// =============================================================================

/// Two-pass pairing merge of a sibling list.
///
/// Pass 1: pair consecutive siblings left-to-right, accumulate in reverse.
/// Pass 2: merge the reversed list left-to-right (= right-to-left original).
///
/// Returns the new root, or null if input is null.
///
/// # Safety
///
/// - `first` may be null (returns null).
/// - If non-null, must point to the first node in a valid sibling list.
/// - All nodes in the list must have `strong > 0`.
#[inline]
unsafe fn merge_pairs<T: Ord>(first: NodePtr<T>) -> NodePtr<T> {
    if first.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: first is non-null, caller guarantees valid sibling list
    unsafe {
        let first_node = &*node_deref(first);
        if first_node.next_ptr().is_null() {
            // Single child — clear stale metadata and return
            first_node.set_parent(ptr::null_mut());
            first_node.set_prev(ptr::null_mut());
            return first;
        }

        // Pass 1: left-to-right pairing, build reversed list via `next`.
        // link() does not read parent/prev/next from inputs, so we skip
        // the pre-link null-writes and clean up the winner afterward.
        let mut reversed: NodePtr<T> = ptr::null_mut();
        let mut current = first;

        while !current.is_null() {
            let a = current;
            let a_node = &*node_deref(a);
            let b = a_node.next_ptr();

            if b.is_null() {
                // Odd one out — prepend to reversed list
                a_node.set_parent(ptr::null_mut());
                a_node.set_prev(ptr::null_mut());
                a_node.set_next(reversed);
                reversed = a;
                break;
            }

            let b_node = &*node_deref(b);
            current = b_node.next_ptr();

            let winner = link(a, b);
            // link sets loser's fields correctly; clean up winner's stale metadata
            let winner_node = &*node_deref(winner);
            winner_node.set_parent(ptr::null_mut());
            winner_node.set_prev(ptr::null_mut());
            winner_node.set_next(reversed);
            reversed = winner;
        }

        // Pass 2: merge reversed list left-to-right.
        // All nodes in reversed list have null parent/prev from pass 1.
        let mut result = reversed;
        let result_node = &*node_deref(result);
        let mut current = result_node.next_ptr();
        result_node.set_next(ptr::null_mut());

        while !current.is_null() {
            let current_node = &*node_deref(current);
            let next = current_node.next_ptr();
            current_node.set_next(ptr::null_mut());
            result = link(result, current);
            current = next;
        }

        (&*node_deref(result)).set_parent(ptr::null_mut());
        result
    }
}

// =============================================================================
// clear_all — iterative tree teardown
// =============================================================================

/// Iteratively clears all nodes starting from `root`, releasing the heap's
/// strong reference for each.
///
/// Uses O(1) auxiliary space by splicing each node's child list into the
/// traversal list.
///
/// # Safety
///
/// - `root` may be null (no-op).
/// - If non-null, all reachable nodes must be valid with allocator `A`.
unsafe fn clear_all<T: 'static, A: Alloc<Item = RcInner<HeapNode<T>>>>(mut current: NodePtr<T>) {
    // SAFETY: caller guarantees all reachable nodes have strong > 0
    unsafe {
        while !current.is_null() {
            let node = node_deref(current);
            let first_child = (*node).child_ptr();
            let next_sibling = (*node).next_ptr();

            // Splice children into the traversal list so we visit them
            // before continuing with the next sibling
            if !first_child.is_null() && !next_sibling.is_null() {
                let mut tail = first_child;
                loop {
                    let t = (*node_deref(tail)).next_ptr();
                    if t.is_null() {
                        break;
                    }
                    tail = t;
                }
                (*node_deref(tail)).set_next(next_sibling);
            }

            let next_to_visit = if first_child.is_null() {
                next_sibling
            } else {
                first_child
            };

            // Clear and release
            (*node).set_parent(ptr::null_mut());
            (*node).set_child(ptr::null_mut());
            (*node).set_next(ptr::null_mut());
            (*node).set_prev(ptr::null_mut());
            (*node).set_owner(0);
            RcSlot::<HeapNode<T>, A>::decrement_strong_count(current);

            current = next_to_visit;
        }
    }
}

// =============================================================================
// Heap<T, A>
// =============================================================================

/// A min-heap (pairing heap) backed by slab-allocated `RcSlot` nodes.
///
/// # Complexity
///
/// | Operation  | Time                |
/// |------------|---------------------|
/// | link/push  | O(1)                |
/// | peek       | O(1)                |
/// | pop        | O(log n) amortized  |
/// | unlink     | O(log n) amortized  |
/// | contains   | O(1)                |
///
/// # Ownership Model
///
/// - User holds `RcSlot<HeapNode<T>, A>` handles — their ownership token
/// - Heap stores raw pointers and maintains its own strong reference per node
/// - Every linked node has `strong_count >= 2` (user + heap)
/// - Unlinking releases the heap's ref; the user's handle remains valid
///
/// # Type Parameters
///
/// - `T`: Element type (must implement `Ord`)
/// - `A`: Allocator type (generated by [`heap_allocator!`](crate::heap_allocator))
pub struct Heap<T: 'static + Ord, A: Alloc<Item = RcInner<HeapNode<T>>>> {
    root: NodePtr<T>,
    len: usize,
    id: usize,
    _marker: PhantomData<A>,
}

impl<T: 'static + Ord, A: Alloc<Item = RcInner<HeapNode<T>>>> Heap<T, A> {
    /// Creates a new empty heap.
    ///
    /// Takes a ZST allocator by value for type inference. The value is not
    /// stored — all methods use `A`'s associated functions directly.
    #[inline]
    #[allow(unused_variables, clippy::needless_pass_by_value)]
    pub fn new(alloc: A) -> Self {
        Heap {
            root: ptr::null_mut(),
            len: 0,
            id: next_collection_id(),
            _marker: PhantomData,
        }
    }

    /// Returns the number of elements in the heap.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the heap has no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns a reference to the minimum (root) node, or `None` if empty.
    ///
    /// Call `.data()` on the node to access user data.
    #[inline]
    pub fn peek(&self) -> Option<&HeapNode<T>> {
        if self.root.is_null() {
            return None;
        }
        // SAFETY: root is non-null (checked), heap holds strong ref.
        // Reference lifetime bounded by &self.
        Some(unsafe { &*node_deref(self.root) })
    }

    /// Returns `true` if the handle is linked to this heap.
    #[inline]
    pub fn contains(&self, handle: &RcSlot<HeapNode<T>, A>) -> bool {
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(handle.as_ptr()) };
        node.owner_id() == self.id
    }

    // =========================================================================
    // Link / Pop / Unlink / Clear
    // =========================================================================

    /// Links an existing node into the heap. O(1).
    ///
    /// The heap acquires its own strong reference to the node.
    ///
    /// # Panics
    ///
    /// Panics if the node is already linked to a collection.
    #[inline]
    pub fn link(&mut self, handle: &RcSlot<HeapNode<T>, A>) {
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(handle.as_ptr()) };
        if node.is_linked() {
            panic_already_linked();
        }
        // SAFETY: caller verified node is not linked
        unsafe { self.link_unchecked(handle) };
    }

    /// Links an existing node into the heap without verifying it is unlinked. O(1).
    ///
    /// The heap acquires its own strong reference to the node.
    ///
    /// # Safety
    ///
    /// The node must not be currently linked to any collection. Double-linking
    /// corrupts heap structure and causes use-after-free on drop.
    #[inline]
    pub unsafe fn link_unchecked(&mut self, handle: &RcSlot<HeapNode<T>, A>) {
        let ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(ptr) };

        node.set_owner(self.id);

        // SAFETY: ptr from a live RcSlot; heap acquires its own ref
        unsafe { RcSlot::<HeapNode<T>, A>::increment_strong_count(ptr) };

        if self.root.is_null() {
            self.root = ptr;
        } else {
            // SAFETY: both root and ptr are valid nodes with strong > 0
            self.root = unsafe { link(self.root, ptr) };
        }

        self.len += 1;
    }

    /// Removes and returns the minimum (root) node. O(log n) amortized.
    ///
    /// The returned handle carries the heap's strong reference — no net
    /// refcount change.
    #[inline]
    pub fn pop(&mut self) -> Option<RcSlot<HeapNode<T>, A>> {
        if self.root.is_null() {
            return None;
        }

        let root_ptr = self.root;

        // SAFETY: root is non-null, heap holds strong ref
        unsafe {
            let root = &*node_deref(root_ptr);
            let first_child = root.child_ptr();

            // Only child needs clearing — parent/next/prev are null by invariant
            root.set_child(ptr::null_mut());
            root.set_owner(0);

            // Merge root's children into new root
            self.root = merge_pairs(first_child);
        }

        self.len -= 1;

        // SAFETY: transferring heap's strong ref into the returned RcSlot
        Some(unsafe { RcSlot::from_raw(root_ptr) })
    }

    /// Removes a node from the heap by handle. O(log n) amortized.
    ///
    /// The user's handle remains valid. The heap releases its strong reference.
    ///
    /// # Panics
    ///
    /// Panics if the node is not linked to this heap.
    #[inline]
    pub fn unlink(&mut self, handle: &RcSlot<HeapNode<T>, A>) {
        let ptr = handle.as_ptr();
        // SAFETY: handle is a live RcSlot, strong >= 1
        let node = unsafe { &*node_deref(ptr) };
        if node.owner_id() != self.id {
            panic_wrong_heap();
        }
        self.unlink_ptr(ptr);
    }

    /// Removes a node from the heap without verifying ownership.
    ///
    /// The user's handle remains valid. The heap releases its strong reference.
    ///
    /// # Safety
    ///
    /// The node must be currently linked to this heap. Unlinking a node from
    /// the wrong heap causes use-after-free (spurious strong count decrement).
    #[inline]
    pub unsafe fn unlink_unchecked(&mut self, handle: &RcSlot<HeapNode<T>, A>) {
        self.unlink_ptr(handle.as_ptr());
    }

    /// Internal: unlink by raw pointer. Detaches the node and decrements
    /// the heap's strong reference.
    #[inline]
    fn unlink_ptr(&mut self, ptr: NodePtr<T>) {
        if ptr == self.root {
            // SAFETY: root is valid, heap holds strong ref
            unsafe {
                let node = &*node_deref(ptr);
                let first_child = node.child_ptr();
                // Only child needs clearing — parent/next/prev are null by invariant
                node.set_child(ptr::null_mut());
                node.set_owner(0);

                self.root = merge_pairs(first_child);
            }
            self.len -= 1;
            // SAFETY: heap owns a strong ref from push time
            unsafe { RcSlot::<HeapNode<T>, A>::decrement_strong_count(ptr) };
            return;
        }

        // Cut node from its parent's child list
        // SAFETY: ptr is a valid linked node (not root, so has a parent)
        unsafe { cut(ptr) };

        // Merge node's children back into heap
        // SAFETY: ptr is valid, child list (if any) contains valid nodes
        unsafe {
            let node = &*node_deref(ptr);
            let first_child = node.child_ptr();
            node.set_child(ptr::null_mut());
            node.set_owner(0);

            if !first_child.is_null() {
                let merged = merge_pairs(first_child);
                self.root = link(self.root, merged);
            }
        }

        self.len -= 1;
        // SAFETY: heap owns a strong ref from push time
        unsafe { RcSlot::<HeapNode<T>, A>::decrement_strong_count(ptr) };
    }

    /// Unlinks all nodes from the heap.
    ///
    /// Each node is detached and the heap's strong reference is released.
    /// User handles remain valid.
    #[inline]
    pub fn clear(&mut self) {
        if self.root.is_null() {
            return;
        }
        // SAFETY: root is non-null, all nodes in the tree are valid
        unsafe { clear_all::<T, A>(self.root) };
        self.root = ptr::null_mut();
        self.len = 0;
    }

    // =========================================================================
    // Iterators
    // =========================================================================

    /// Returns a draining iterator that pops elements in sorted order.
    ///
    /// When dropped, any remaining elements are cleared from the heap.
    #[inline]
    pub fn drain(&mut self) -> Drain<'_, T, A> {
        Drain { heap: self }
    }

    /// Returns an iterator that pops elements while the predicate is true.
    ///
    /// The predicate receives `&HeapNode<T>` — call `.data()` to read data.
    /// When dropped, remaining elements stay in the heap.
    #[inline]
    pub fn drain_while<F: FnMut(&HeapNode<T>) -> bool>(
        &mut self,
        pred: F,
    ) -> DrainWhile<'_, T, A, F> {
        DrainWhile { heap: self, pred }
    }
}

// =============================================================================
// Push methods — bounded allocators
// =============================================================================

impl<T: 'static + Ord, A: BoundedAlloc<Item = RcInner<HeapNode<T>>>> Heap<T, A> {
    /// Allocates a new node and inserts it into the heap. Returns the handle.
    ///
    /// Returns `Err(Full(value))` if the allocator is at capacity.
    #[inline]
    pub fn try_push(&mut self, value: T) -> Result<RcSlot<HeapNode<T>, A>, Full<T>> {
        let handle = RcSlot::try_new(HeapNode::new(value))
            .map_err(|full| Full(full.into_inner().into_data()))?;
        // SAFETY: freshly allocated node is not linked to any collection
        unsafe { self.link_unchecked(&handle) };
        Ok(handle)
    }
}

// =============================================================================
// Push methods — unbounded allocators
// =============================================================================

impl<T: 'static + Ord, A: UnboundedAlloc<Item = RcInner<HeapNode<T>>>> Heap<T, A> {
    /// Allocates a new node and inserts it into the heap. Returns the handle.
    #[inline]
    pub fn push(&mut self, value: T) -> RcSlot<HeapNode<T>, A> {
        let handle = RcSlot::new(HeapNode::new(value));
        // SAFETY: freshly allocated node is not linked to any collection
        unsafe { self.link_unchecked(&handle) };
        handle
    }
}

impl<T: 'static + Ord, A: Alloc<Item = RcInner<HeapNode<T>>>> Drop for Heap<T, A> {
    fn drop(&mut self) {
        self.clear();
    }
}

// =============================================================================
// Drain
// =============================================================================

/// Draining iterator that pops elements in sorted (min-first) order.
///
/// When dropped, any remaining elements are cleared from the heap.
pub struct Drain<'a, T: 'static + Ord, A: Alloc<Item = RcInner<HeapNode<T>>>> {
    heap: &'a mut Heap<T, A>,
}

impl<T: 'static + Ord, A: Alloc<Item = RcInner<HeapNode<T>>>> Iterator for Drain<'_, T, A> {
    type Item = RcSlot<HeapNode<T>, A>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.heap.pop()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.heap.len(), Some(self.heap.len()))
    }
}

impl<T: 'static + Ord, A: Alloc<Item = RcInner<HeapNode<T>>>> ExactSizeIterator
    for Drain<'_, T, A>
{
}

impl<T: 'static + Ord, A: Alloc<Item = RcInner<HeapNode<T>>>> Drop for Drain<'_, T, A> {
    fn drop(&mut self) {
        self.heap.clear();
    }
}

// =============================================================================
// DrainWhile
// =============================================================================

/// Iterator that pops elements while a predicate holds.
///
/// When dropped, remaining elements stay in the heap.
pub struct DrainWhile<
    'a,
    T: 'static + Ord,
    A: Alloc<Item = RcInner<HeapNode<T>>>,
    F: FnMut(&HeapNode<T>) -> bool,
> {
    heap: &'a mut Heap<T, A>,
    pred: F,
}

impl<T: 'static + Ord, A: Alloc<Item = RcInner<HeapNode<T>>>, F: FnMut(&HeapNode<T>) -> bool>
    Iterator for DrainWhile<'_, T, A, F>
{
    type Item = RcSlot<HeapNode<T>, A>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.heap.is_empty() {
            return None;
        }
        // SAFETY: heap is non-empty, root is valid, heap holds strong ref.
        let node = unsafe { &*node_deref(self.heap.root) };
        if !(self.pred)(node) {
            return None;
        }
        self.heap.pop()
    }
}
