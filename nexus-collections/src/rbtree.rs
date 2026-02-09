//! Red-black tree sorted map with internal slab allocation.
//!
//! # Design
//!
//! A self-balancing BST providing deterministic O(log n) worst case for insert,
//! lookup, and removal. At most 2 rotations per insert, 3 per delete. Slab-allocated
//! nodes via `nexus-slab` for zero allocation after init.
//!
//! # Allocation Model
//!
//! Same as [`BTree`](crate::btree::BTree) — the tree takes a ZST allocator
//! at construction time and handles all node allocation/deallocation internally.
//! The user only sees keys and values.
//!
//! - Bounded allocators: `try_insert` returns `Err(Full((K, V)))` when full
//! - Unbounded allocators: `insert` always succeeds (grows as needed)
//!
//! # Example
//!
//! ```ignore
//! mod levels {
//!     nexus_collections::rbtree_allocator!(u64, String, bounded);
//! }
//!
//! levels::Allocator::builder().capacity(1000).build().unwrap();
//!
//! let mut map = levels::RbTree::new(levels::Allocator);
//! map.try_insert(100, "hello".into()).unwrap();
//!
//! assert_eq!(map.get(&100), Some(&"hello".into()));
//! ```

use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::ptr;

use nexus_slab::{Alloc, BoundedAlloc, Full, Slot, SlotCell, UnboundedAlloc};

// =============================================================================
// Color constants — packed into the LSB of parent pointer
// =============================================================================

const COLOR_RED: usize = 0;
const COLOR_BLACK: usize = 1;
const COLOR_MASK: usize = 1;
const PARENT_MASK: usize = !1;

// =============================================================================
// NodePtr
// =============================================================================

/// Raw pointer to a slab-allocated RB tree node.
type NodePtr<K, V> = *mut SlotCell<RbNode<K, V>>;

// =============================================================================
// RbNode<K, V>
// =============================================================================

/// A node in a red-black tree sorted map.
///
/// Color is packed into the LSB of the parent pointer (slab nodes are at
/// least 8-byte aligned, guaranteeing the low 3 bits are zero). This
/// shrinks the node by 8 bytes vs a separate color field.
///
/// For `K=u64, V=u64`: key(8) + left(8) + right(8) + parent_color(8) +
/// value(8) = 40 bytes — fits in a single cache line.
#[repr(C)]
pub struct RbNode<K, V> {
    key: K,
    left: Cell<NodePtr<K, V>>,
    right: Cell<NodePtr<K, V>>,
    /// Parent pointer with color packed in the LSB.
    /// Bit 0: 0 = red, 1 = black. Bits 1..63: parent address.
    parent_color: Cell<usize>,
    value: V,
}

impl<K, V> RbNode<K, V> {
    /// Creates a new detached red node with the given key and value.
    ///
    /// All link pointers are null. Color is red (new insertions are red).
    #[inline]
    pub fn new(key: K, value: V) -> Self {
        RbNode {
            key,
            left: Cell::new(ptr::null_mut()),
            right: Cell::new(ptr::null_mut()),
            parent_color: Cell::new(COLOR_RED), // null parent, red
            value,
        }
    }

    /// Returns a reference to the key.
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Returns a reference to the value.
    #[inline]
    pub fn value(&self) -> &V {
        &self.value
    }

    /// Returns a mutable reference to the value.
    #[inline]
    pub fn value_mut(&mut self) -> &mut V {
        &mut self.value
    }

    /// Consumes the node, returning `(key, value)`.
    #[doc(hidden)]
    #[inline]
    pub fn into_data(self) -> (K, V) {
        (self.key, self.value)
    }
}

// =============================================================================
// Packed parent/color helpers
// =============================================================================

/// Extracts the parent pointer from a node's packed parent_color field.
#[inline]
fn get_parent<K, V>(ptr: NodePtr<K, V>) -> NodePtr<K, V> {
    // SAFETY: ptr is non-null, caller guarantees it points to an occupied slot.
    let packed = unsafe { (*node_deref(ptr)).parent_color.get() };
    ptr::with_exposed_provenance_mut(packed & PARENT_MASK)
}

/// Sets the parent pointer, preserving the existing color.
#[inline]
fn set_parent<K, V>(ptr: NodePtr<K, V>, parent: NodePtr<K, V>) {
    // SAFETY: ptr is non-null, caller guarantees it points to an occupied slot.
    let node = unsafe { &*node_deref(ptr) };
    let color = node.parent_color.get() & COLOR_MASK;
    let parent_bits = parent.expose_provenance();
    node.parent_color.set(parent_bits | color);
}

/// Sets both parent and color in one write.
#[inline]
fn set_parent_color<K, V>(ptr: NodePtr<K, V>, parent: NodePtr<K, V>, color: usize) {
    // SAFETY: ptr is non-null, caller guarantees it points to an occupied slot.
    let node = unsafe { &*node_deref(ptr) };
    let parent_bits = parent.expose_provenance();
    node.parent_color.set(parent_bits | color);
}

// =============================================================================
// node_deref — navigate raw pointer to RbNode
// =============================================================================

/// Dereferences a `NodePtr<K, V>` to `*const RbNode<K, V>`.
///
/// # Safety
///
/// - `ptr` must be non-null and point to an occupied `SlotCell`.
#[inline]
unsafe fn node_deref<K, V>(ptr: NodePtr<K, V>) -> *const RbNode<K, V> {
    // SAFETY: Caller guarantees ptr is non-null and points to an occupied slot.
    // Use addr_of! to avoid creating an intermediate reference.
    // ManuallyDrop<MaybeUninit<T>> has the same layout as T.
    unsafe { std::ptr::addr_of!((*ptr).value).cast() }
}

/// Dereferences a `NodePtr<K, V>` to `*mut RbNode<K, V>`.
///
/// # Safety
///
/// - `ptr` must be non-null and point to an occupied `SlotCell`.
/// - The caller must ensure no other reference to the same node exists.
#[inline]
unsafe fn node_deref_mut<K, V>(ptr: NodePtr<K, V>) -> *mut RbNode<K, V> {
    // SAFETY: Caller guarantees ptr is non-null, occupied, and unaliased.
    // Use addr_of_mut! to avoid implicit DerefMut on ManuallyDrop union field.
    // ManuallyDrop<MaybeUninit<T>> has the same layout as T.
    unsafe { std::ptr::addr_of_mut!((*ptr).value).cast() }
}

// =============================================================================
// Color helpers — null is black (sentinel convention)
// =============================================================================

/// Returns `true` if the node is red. Null nodes are black.
#[inline]
fn is_red<K, V>(ptr: NodePtr<K, V>) -> bool {
    if ptr.is_null() {
        return false;
    }
    // SAFETY: ptr is non-null; caller ensures it points to an occupied slot.
    unsafe { (*node_deref(ptr)).parent_color.get() & COLOR_MASK == COLOR_RED }
}

/// Sets the color of a node, preserving its parent pointer. No-op if null.
#[inline]
fn set_color<K, V>(ptr: NodePtr<K, V>, color: usize) {
    if !ptr.is_null() {
        // SAFETY: ptr is non-null; caller ensures it points to an occupied slot.
        let node = unsafe { &*node_deref(ptr) };
        let packed = node.parent_color.get();
        node.parent_color.set((packed & PARENT_MASK) | color);
    }
}

// =============================================================================
// Tree navigation helpers
// =============================================================================

/// Returns the leftmost (minimum) node in the subtree rooted at `ptr`.
///
/// # Safety
///
/// - `ptr` must be non-null and point to an occupied slot.
#[inline]
unsafe fn tree_minimum<K, V>(mut ptr: NodePtr<K, V>) -> NodePtr<K, V> {
    // SAFETY: ptr is non-null on entry; loop maintains this invariant.
    loop {
        let left = unsafe { (*node_deref(ptr)).left.get() };
        if left.is_null() {
            return ptr;
        }
        ptr = left;
    }
}

/// Returns the rightmost (maximum) node in the subtree rooted at `ptr`.
///
/// # Safety
///
/// - `ptr` must be non-null and point to an occupied slot.
#[inline]
unsafe fn tree_maximum<K, V>(mut ptr: NodePtr<K, V>) -> NodePtr<K, V> {
    // SAFETY: ptr is non-null on entry; loop maintains this invariant.
    loop {
        let right = unsafe { (*node_deref(ptr)).right.get() };
        if right.is_null() {
            return ptr;
        }
        ptr = right;
    }
}

/// Returns the in-order successor of `ptr`, or null if `ptr` is the maximum.
///
/// O(1) amortized over a full traversal.
///
/// # Safety
///
/// - `ptr` must be non-null and point to an occupied slot in the tree.
#[inline]
unsafe fn successor<K, V>(ptr: NodePtr<K, V>) -> NodePtr<K, V> {
    // SAFETY: ptr is non-null.
    let node = unsafe { &*node_deref(ptr) };

    // If right child exists, successor is the leftmost in right subtree.
    let right = node.right.get();
    if !right.is_null() {
        return unsafe { tree_minimum(right) };
    }

    // Walk up while we're the right child.
    let mut current = ptr;
    let mut parent = get_parent(ptr);
    while !parent.is_null() {
        // SAFETY: parent is non-null and in the tree.
        if current != unsafe { (*node_deref(parent)).right.get() } {
            break;
        }
        current = parent;
        parent = get_parent(parent);
    }
    parent
}

/// Returns the in-order predecessor of `ptr`, or null if `ptr` is the minimum.
///
/// O(1) amortized over a full traversal. Symmetric to `successor`.
///
/// # Safety
///
/// - `ptr` must be non-null and point to an occupied slot in the tree.
#[inline]
unsafe fn predecessor<K, V>(ptr: NodePtr<K, V>) -> NodePtr<K, V> {
    // SAFETY: ptr is non-null.
    let node = unsafe { &*node_deref(ptr) };

    // If left child exists, predecessor is the rightmost in left subtree.
    let left = node.left.get();
    if !left.is_null() {
        return unsafe { tree_maximum(left) };
    }

    // Walk up while we're the left child.
    let mut current = ptr;
    let mut parent = get_parent(ptr);
    while !parent.is_null() {
        // SAFETY: parent is non-null and in the tree.
        if current != unsafe { (*node_deref(parent)).left.get() } {
            break;
        }
        current = parent;
        parent = get_parent(parent);
    }
    parent
}

// =============================================================================
// Prefetch
// =============================================================================

/// Prefetch a node for upcoming read access (search, iteration).
///
/// Issues a T0 (temporal, all cache levels) prefetch hint. On non-x86_64
/// platforms this is a no-op.
#[inline(always)]
fn prefetch_read_node<K, V>(ptr: NodePtr<K, V>) {
    #[cfg(target_arch = "x86_64")]
    if !ptr.is_null() {
        // SAFETY: _mm_prefetch on an invalid address is architecturally a NOP on x86.
        unsafe {
            std::arch::x86_64::_mm_prefetch(ptr as *const i8, std::arch::x86_64::_MM_HINT_T0);
        }
    }
}

// =============================================================================
// RbTree<K, V, A>
// =============================================================================

/// A self-balancing sorted map with internal slab allocation.
///
/// # Complexity
///
/// | Operation    | Time        |
/// |--------------|-------------|
/// | insert       | O(log n)    |
/// | remove       | O(log n)    |
/// | get / get_mut| O(log n)    |
/// | first / last | O(1)        |
/// | pop_first    | O(log n)    |
/// | pop_last     | O(log n)    |
/// | contains_key | O(log n)    |
///
/// # Allocation Model
///
/// The tree manages node allocation internally via a ZST allocator:
/// - Bounded: `try_insert` may fail with `Full<(K, V)>`
/// - Unbounded: `insert` always succeeds
/// - `remove`/`pop` deallocate internally and return values directly
pub struct RbTree<K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> {
    root: NodePtr<K, V>,
    leftmost: NodePtr<K, V>,
    rightmost: NodePtr<K, V>,
    len: usize,
    _marker: PhantomData<A>,
}

// =============================================================================
// impl<A: Alloc> — base block
// =============================================================================

impl<K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> RbTree<K, V, A> {
    /// Creates a new empty red-black tree.
    #[inline]
    #[allow(unused_variables, clippy::needless_pass_by_value)]
    pub fn new(alloc: A) -> Self {
        RbTree {
            root: ptr::null_mut(),
            leftmost: ptr::null_mut(),
            rightmost: ptr::null_mut(),
            len: 0,
            _marker: PhantomData,
        }
    }

    // =========================================================================
    // Queries
    // =========================================================================

    /// Returns the number of elements in the tree.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the tree is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns `true` if the tree contains the given key.
    #[inline]
    pub fn contains_key(&self, key: &K) -> bool {
        self.find(key).is_some()
    }

    /// Returns a reference to the value for the given key.
    #[inline]
    pub fn get(&self, key: &K) -> Option<&V> {
        let ptr = self.find(key)?;
        // SAFETY: find returns a valid, occupied node pointer.
        Some(unsafe { &(*node_deref(ptr)).value })
    }

    /// Returns a mutable reference to the value for the given key.
    #[inline]
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let ptr = self.find(key)?;
        // SAFETY: find returns a valid node pointer; &mut self prevents aliasing.
        Some(unsafe { &mut (*node_deref_mut(ptr)).value })
    }

    /// Returns references to the key and value for the given key.
    #[inline]
    pub fn get_key_value(&self, key: &K) -> Option<(&K, &V)> {
        let ptr = self.find(key)?;
        // SAFETY: find returns a valid, occupied node pointer.
        let node = unsafe { &*node_deref(ptr) };
        Some((&node.key, &node.value))
    }

    /// Returns the first (smallest) key-value pair.
    ///
    /// O(1) — cached leftmost pointer.
    #[inline]
    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        if self.leftmost.is_null() {
            return None;
        }
        // SAFETY: leftmost is non-null and points to an occupied slot.
        let node = unsafe { &*node_deref(self.leftmost) };
        Some((&node.key, &node.value))
    }

    /// Returns the last (largest) key-value pair.
    ///
    /// O(1) — cached rightmost pointer.
    #[inline]
    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        if self.rightmost.is_null() {
            return None;
        }
        // SAFETY: rightmost is non-null and points to an occupied slot.
        let node = unsafe { &*node_deref(self.rightmost) };
        Some((&node.key, &node.value))
    }

    // =========================================================================
    // Mutation — remove / pop / clear
    // =========================================================================

    /// Removes the node with the given key and returns the value.
    #[inline]
    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (_, v) = self.remove_entry(key)?;
        Some(v)
    }

    /// Removes the node with the given key and returns `(key, value)`.
    #[inline]
    pub fn remove_entry(&mut self, key: &K) -> Option<(K, V)> {
        let ptr = self.find(key)?;
        Some(self.remove_node(ptr))
    }

    /// Removes and returns the first (smallest) key-value pair.
    ///
    /// O(log n) — requires delete fixup.
    #[inline]
    pub fn pop_first(&mut self) -> Option<(K, V)> {
        if self.leftmost.is_null() {
            return None;
        }
        Some(self.remove_node(self.leftmost))
    }

    /// Removes and returns the last (largest) key-value pair.
    ///
    /// O(log n) — requires delete fixup.
    #[inline]
    pub fn pop_last(&mut self) -> Option<(K, V)> {
        if self.rightmost.is_null() {
            return None;
        }
        Some(self.remove_node(self.rightmost))
    }

    /// Removes all nodes, freeing them via the allocator.
    #[inline]
    pub fn clear(&mut self) {
        // Iterative post-order destruction: detach children and descend,
        // freeing leaf nodes and walking up via parent pointers.
        let mut current = self.root;
        while !current.is_null() {
            // SAFETY: current is non-null, points to an occupied slot.
            let node = unsafe { &*node_deref(current) };
            let left = node.left.get();
            let right = node.right.get();

            if !left.is_null() {
                // Detach left child and descend.
                node.left.set(ptr::null_mut());
                current = left;
            } else if !right.is_null() {
                // Detach right child and descend.
                node.right.set(ptr::null_mut());
                current = right;
            } else {
                // Leaf: read parent before freeing.
                let parent = get_parent(current);
                let slot = unsafe { Slot::from_ptr(current) };
                unsafe { A::free(slot) };
                current = parent;
            }
        }

        self.root = ptr::null_mut();
        self.leftmost = ptr::null_mut();
        self.rightmost = ptr::null_mut();
        self.len = 0;
    }

    // =========================================================================
    // Entry API
    // =========================================================================

    /// Gets the entry for the given key (taken by value).
    ///
    /// The entry captures the search result — use it to inspect, modify, insert,
    /// or remove without re-searching.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use nexus_collections::rbtree::Entry;
    ///
    /// match book.entry(price) {
    ///     Entry::Occupied(mut e) => {
    ///         e.get_mut().add_order(order);
    ///     }
    ///     Entry::Vacant(e) => {
    ///         e.try_insert(LevelData::new(order)).unwrap();
    ///     }
    /// }
    /// ```
    #[inline]
    pub fn entry(&mut self, key: K) -> Entry<'_, K, V, A> {
        let mut parent: NodePtr<K, V> = ptr::null_mut();
        let mut is_left = true;
        let mut current = self.root;

        while !current.is_null() {
            parent = current;
            // SAFETY: current is non-null and in the tree.
            let node = unsafe { &*node_deref(current) };
            if key == node.key {
                // Key dropped — existing key stays in the map (matches BTreeMap).
                drop(key);
                return Entry::Occupied(OccupiedEntry {
                    tree: self,
                    ptr: current,
                });
            }
            if key < node.key {
                is_left = true;
                current = node.left.get();
            } else {
                is_left = false;
                current = node.right.get();
            }
        }

        Entry::Vacant(VacantEntry {
            tree: self,
            key,
            parent,
            is_left,
        })
    }

    // =========================================================================
    // Iteration
    // =========================================================================

    /// Returns an iterator over `(&K, &V)` pairs in sorted order.
    #[inline]
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            front: self.leftmost,
            len: self.len,
            _marker: PhantomData,
        }
    }

    /// Returns an iterator over keys in sorted order.
    #[inline]
    pub fn keys(&self) -> Keys<'_, K, V> {
        Keys { inner: self.iter() }
    }

    /// Returns an iterator over values in key-sorted order.
    #[inline]
    pub fn values(&self) -> Values<'_, K, V> {
        Values { inner: self.iter() }
    }

    /// Returns a mutable iterator over `(&K, &mut V)` pairs in sorted order.
    ///
    /// Keys are immutable — changing them would violate sorted order.
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, K, V> {
        IterMut {
            front: self.leftmost,
            len: self.len,
            _marker: PhantomData,
        }
    }

    /// Returns a mutable iterator over values in key-sorted order.
    #[inline]
    pub fn values_mut(&mut self) -> ValuesMut<'_, K, V> {
        ValuesMut {
            inner: self.iter_mut(),
        }
    }

    /// Returns an iterator over `(&K, &V)` pairs within the given range.
    #[inline]
    pub fn range<R: std::ops::RangeBounds<K>>(&self, range: R) -> Range<'_, K, V> {
        let (front, end) = self.resolve_range_bounds(range);
        Range {
            front,
            end,
            _marker: PhantomData,
        }
    }

    /// Returns a mutable iterator over `(&K, &mut V)` pairs within the given range.
    #[inline]
    pub fn range_mut<R: std::ops::RangeBounds<K>>(&mut self, range: R) -> RangeMut<'_, K, V> {
        let (front, end) = self.resolve_range_bounds(range);
        RangeMut {
            front,
            end,
            _marker: PhantomData,
        }
    }

    // =========================================================================
    // Cursor
    // =========================================================================

    /// Returns a cursor positioned before the first element.
    ///
    /// Call `advance()` to move to the first element.
    #[inline]
    pub fn cursor_front(&mut self) -> Cursor<'_, K, V, A> {
        Cursor {
            tree: self,
            current: ptr::null_mut(),
            started: false,
        }
    }

    /// Returns a cursor positioned at the given key, or at the first
    /// element greater than the key.
    #[inline]
    pub fn cursor_at(&mut self, key: &K) -> Cursor<'_, K, V, A> {
        let current = self
            .find(key)
            .map_or_else(|| self.lower_bound(key), |ptr| ptr);
        Cursor {
            tree: self,
            current,
            started: true,
        }
    }

    // =========================================================================
    // Drain
    // =========================================================================

    /// Returns a draining iterator that removes and returns all key-value
    /// pairs in sorted order.
    ///
    /// When dropped, any remaining nodes are freed via the allocator.
    #[inline]
    pub fn drain(&mut self) -> Drain<'_, K, V, A> {
        Drain { tree: self }
    }

    // =========================================================================
    // Internal: replace_node — O(1) structural swap (Linux rb_replace_node)
    // =========================================================================

    /// Replaces `old` with `new` in the tree structure.
    ///
    /// `new` inherits `old`'s exact position: parent, children, color.
    /// `old` is unlinked but NOT freed — the caller handles deallocation.
    /// Children's parent pointers are updated to point to `new`.
    ///
    /// This is O(1) — no rotations, no fixup, no search.
    ///
    /// # Safety
    ///
    /// - `old` must be a non-null node currently in this tree.
    /// - `new` must be a non-null, freshly allocated, detached node.
    /// - The caller must ensure `new`'s key maintains BST ordering
    ///   (typically used for same-key replacement or inter-tree migration).
    #[inline]
    #[allow(dead_code)]
    unsafe fn replace_node(&mut self, old: NodePtr<K, V>, new: NodePtr<K, V>) {
        let old_node = unsafe { &*node_deref(old) };
        let parent = get_parent(old);
        let left = old_node.left.get();
        let right = old_node.right.get();
        let color = old_node.parent_color.get() & COLOR_MASK;

        // Copy structural state to new node.
        set_parent_color(new, parent, color);
        let new_node = unsafe { &*node_deref(new) };
        new_node.left.set(left);
        new_node.right.set(right);

        // Update parent's child pointer.
        if parent.is_null() {
            self.root = new;
        } else {
            let p_node = unsafe { &*node_deref(parent) };
            if old == p_node.left.get() {
                p_node.left.set(new);
            } else {
                p_node.right.set(new);
            }
        }

        // Update children's parent pointers.
        if !left.is_null() {
            set_parent(left, new);
        }
        if !right.is_null() {
            set_parent(right, new);
        }

        // Update cached extremes.
        if self.leftmost == old {
            self.leftmost = new;
        }
        if self.rightmost == old {
            self.rightmost = new;
        }
    }

    // =========================================================================
    // Internal: link_vacant — for Entry API
    // =========================================================================

    /// Internal helper: links a pre-allocated slot at a known-vacant position.
    ///
    /// Returns a raw pointer to the value — caller assigns the lifetime.
    ///
    /// # Safety
    ///
    /// - `slot` must be a valid, occupied slot
    /// - `parent`/`is_left` must be from a search that found no match
    #[allow(clippy::needless_pass_by_value)]
    unsafe fn link_vacant(
        &mut self,
        slot: Slot<RbNode<K, V>>,
        parent: NodePtr<K, V>,
        is_left: bool,
    ) -> *mut V {
        let ptr = slot.as_ptr();
        self.link_new_node(ptr, parent, is_left);
        // SAFETY: ptr was just linked into the tree.
        unsafe { std::ptr::addr_of_mut!((*node_deref_mut(ptr)).value) }
    }

    // =========================================================================
    // Internal algorithms
    // =========================================================================

    /// Read-only search — returns pointer to node with exact key, or `None`.
    ///
    /// Loads both children eagerly before branching on the comparison.
    /// Out-of-order execution pipelines the loads in parallel.
    #[inline]
    fn find(&self, key: &K) -> Option<NodePtr<K, V>> {
        let mut current = self.root;
        while !current.is_null() {
            // SAFETY: current is non-null and in the tree.
            let node = unsafe { &*node_deref(current) };
            // Direct comparisons avoid LLVM's seta/sbb Ordering materialization.
            // Generates: cmp + je (equal) + jb (less) + fall-through (greater).
            if *key == node.key {
                return Some(current);
            }
            if *key < node.key {
                current = node.left.get();
            } else {
                current = node.right.get();
            }
        }
        None
    }

    /// Find first node with key >= target.
    #[inline]
    fn lower_bound(&self, key: &K) -> NodePtr<K, V> {
        let mut result: NodePtr<K, V> = ptr::null_mut();
        let mut current = self.root;
        while !current.is_null() {
            // SAFETY: current is non-null and in the tree.
            let node = unsafe { &*node_deref(current) };
            if *key > node.key {
                current = node.right.get();
            } else {
                result = current;
                current = node.left.get();
            }
        }
        result
    }

    /// Find first node with key > target.
    #[inline]
    fn upper_bound(&self, key: &K) -> NodePtr<K, V> {
        let mut result: NodePtr<K, V> = ptr::null_mut();
        let mut current = self.root;
        while !current.is_null() {
            // SAFETY: current is non-null and in the tree.
            let node = unsafe { &*node_deref(current) };
            if *key < node.key {
                result = current;
                current = node.left.get();
            } else {
                current = node.right.get();
            }
        }
        result
    }

    /// Resolves `RangeBounds` to `(front, end)` node pointers.
    ///
    /// `front` is the first node in the range. `end` is the first node
    /// PAST the range (exclusive sentinel), or null if the range extends
    /// to the end. The iterator stops when `front == end`.
    #[inline]
    fn resolve_range_bounds<R: std::ops::RangeBounds<K>>(
        &self,
        range: R,
    ) -> (NodePtr<K, V>, NodePtr<K, V>) {
        use std::ops::Bound;

        let front = match range.start_bound() {
            Bound::Unbounded => self.leftmost,
            Bound::Included(k) => self.lower_bound(k),
            Bound::Excluded(k) => self.upper_bound(k),
        };

        let end = match range.end_bound() {
            Bound::Unbounded => ptr::null_mut(),
            Bound::Included(k) => self.upper_bound(k),
            Bound::Excluded(k) => self.lower_bound(k),
        };

        if front.is_null() || front == end {
            return (ptr::null_mut(), ptr::null_mut());
        }

        // Validate front < end in sorted order.
        if !end.is_null() {
            // SAFETY: both pointers are non-null and in the tree.
            let front_key = unsafe { &(*node_deref(front)).key };
            let end_key = unsafe { &(*node_deref(end)).key };
            if front_key >= end_key {
                return (ptr::null_mut(), ptr::null_mut());
            }
        }

        (front, end)
    }

    /// Links a new node into the tree as a child of `parent`.
    ///
    /// Sets parent pointers, updates leftmost/rightmost, increments len,
    /// then runs insert fixup for rebalancing.
    #[inline]
    fn link_new_node(&mut self, ptr: NodePtr<K, V>, parent: NodePtr<K, V>, is_left: bool) {
        // SAFETY: ptr points to a valid, occupied node (just allocated).
        // New nodes are red (COLOR_RED = 0), so parent_color = parent address.
        set_parent_color(ptr, parent, COLOR_RED);

        if parent.is_null() {
            // First node in the tree.
            self.root = ptr;
            self.leftmost = ptr;
            self.rightmost = ptr;
        } else if is_left {
            // SAFETY: parent is non-null and in the tree.
            unsafe { (*node_deref(parent)).left.set(ptr) };
            if parent == self.leftmost {
                self.leftmost = ptr;
            }
        } else {
            // SAFETY: parent is non-null and in the tree.
            unsafe { (*node_deref(parent)).right.set(ptr) };
            if parent == self.rightmost {
                self.rightmost = ptr;
            }
        }

        self.len += 1;

        // SAFETY: ptr is linked into the tree.
        unsafe { self.insert_fixup(ptr) };
    }

    /// Removes a node from the tree, deallocates it, and returns (key, value).
    ///
    /// Updates leftmost/rightmost/len. Computes successor/predecessor BEFORE
    /// unlinking to avoid an O(log n) tree walk from root after deletion.
    fn remove_node(&mut self, ptr: NodePtr<K, V>) -> (K, V) {
        // Compute new extremes BEFORE delete_node invalidates the structure.
        let new_leftmost = if ptr == self.leftmost {
            if self.len == 1 {
                ptr::null_mut()
            } else {
                // SAFETY: ptr is in the tree with len > 1, so successor exists.
                unsafe { successor(ptr) }
            }
        } else {
            self.leftmost
        };
        let new_rightmost = if ptr == self.rightmost {
            if self.len == 1 {
                ptr::null_mut()
            } else {
                // SAFETY: ptr is in the tree with len > 1, so predecessor exists.
                unsafe { predecessor(ptr) }
            }
        } else {
            self.rightmost
        };

        // SAFETY: ptr is in the tree.
        unsafe { self.delete_node(ptr) };
        self.len -= 1;
        self.leftmost = new_leftmost;
        self.rightmost = new_rightmost;

        // SAFETY: ptr was in the tree, now unlinked.
        let slot = unsafe { Slot::from_ptr(ptr) };
        let node = unsafe { A::take(slot) };
        node.into_data()
    }

    // =========================================================================
    // Rotations
    // =========================================================================

    /// Left rotation around `x`.
    ///
    /// ```text
    ///     x                y
    ///    / \              / \
    ///   a   y    =>     x    c
    ///      / \         / \
    ///     b   c       a   b
    /// ```
    ///
    /// # Safety
    ///
    /// - `x` must be non-null with a non-null right child.
    #[inline]
    unsafe fn rotate_left(&mut self, x: NodePtr<K, V>) {
        // SAFETY: x is non-null with a non-null right child.
        let x_node = unsafe { &*node_deref(x) };
        let y = x_node.right.get();
        let y_node = unsafe { &*node_deref(y) };

        // Turn y's left subtree into x's right subtree.
        let b = y_node.left.get();
        x_node.right.set(b);
        if !b.is_null() {
            set_parent(b, x);
        }

        // Link x's parent to y.
        let p = get_parent(x);
        set_parent(y, p);
        if p.is_null() {
            self.root = y;
        } else {
            // SAFETY: p is non-null and in the tree.
            let p_node = unsafe { &*node_deref(p) };
            if x == p_node.left.get() {
                p_node.left.set(y);
            } else {
                p_node.right.set(y);
            }
        }

        // Put x on y's left.
        y_node.left.set(x);
        set_parent(x, y);
    }

    /// Right rotation around `x`.
    ///
    /// ```text
    ///       x            y
    ///      / \          / \
    ///     y   c  =>   a    x
    ///    / \               / \
    ///   a   b             b   c
    /// ```
    ///
    /// # Safety
    ///
    /// - `x` must be non-null with a non-null left child.
    #[inline]
    unsafe fn rotate_right(&mut self, x: NodePtr<K, V>) {
        // SAFETY: x is non-null with a non-null left child.
        let x_node = unsafe { &*node_deref(x) };
        let y = x_node.left.get();
        let y_node = unsafe { &*node_deref(y) };

        // Turn y's right subtree into x's left subtree.
        let b = y_node.right.get();
        x_node.left.set(b);
        if !b.is_null() {
            set_parent(b, x);
        }

        // Link x's parent to y.
        let p = get_parent(x);
        set_parent(y, p);
        if p.is_null() {
            self.root = y;
        } else {
            // SAFETY: p is non-null and in the tree.
            let p_node = unsafe { &*node_deref(p) };
            if x == p_node.left.get() {
                p_node.left.set(y);
            } else {
                p_node.right.set(y);
            }
        }

        // Put x on y's right.
        y_node.right.set(x);
        set_parent(x, y);
    }

    /// Replaces the subtree rooted at `u` with the subtree rooted at `v`.
    ///
    /// Updates `u`'s parent to point to `v`, and sets `v`'s parent.
    /// Does NOT update `u`'s own parent pointer.
    ///
    /// # Safety
    ///
    /// - `u` must be non-null and in the tree.
    /// - `v` may be null (replacing with empty subtree).
    #[inline]
    unsafe fn transplant(&mut self, u: NodePtr<K, V>, v: NodePtr<K, V>) {
        // SAFETY: u is non-null.
        let u_parent = get_parent(u);
        if u_parent.is_null() {
            self.root = v;
        } else {
            // SAFETY: u_parent is non-null and in the tree.
            let p_node = unsafe { &*node_deref(u_parent) };
            if u == p_node.left.get() {
                p_node.left.set(v);
            } else {
                p_node.right.set(v);
            }
        }
        if !v.is_null() {
            set_parent(v, u_parent);
        }
    }

    // =========================================================================
    // Insert fixup (CLRS)
    // =========================================================================

    /// Restores red-black properties after insertion.
    ///
    /// At most 2 rotations.
    ///
    /// # Safety
    ///
    /// - `z` must be a valid red node just linked into the tree.
    unsafe fn insert_fixup(&mut self, mut z: NodePtr<K, V>) {
        // Loop while z's parent is red (red-red violation).
        while is_red(get_parent(z)) {
            let parent = get_parent(z);
            let grandparent = get_parent(parent);
            // SAFETY: parent is red, so it can't be root (root is black).
            // Therefore grandparent exists.

            if parent == unsafe { (*node_deref(grandparent)).left.get() } {
                // Parent is left child of grandparent.
                let uncle = unsafe { (*node_deref(grandparent)).right.get() };

                if is_red(uncle) {
                    // Case 1: Uncle is red — recolor.
                    set_color(parent, COLOR_BLACK);
                    set_color(uncle, COLOR_BLACK);
                    set_color(grandparent, COLOR_RED);
                    z = grandparent;
                } else {
                    if z == unsafe { (*node_deref(parent)).right.get() } {
                        // Case 2: z is right child — rotate left to transform to case 3.
                        z = parent;
                        unsafe { self.rotate_left(z) };
                    }
                    // Case 3: z is left child — recolor and rotate right.
                    let parent = get_parent(z);
                    let grandparent = get_parent(parent);
                    set_color(parent, COLOR_BLACK);
                    set_color(grandparent, COLOR_RED);
                    unsafe { self.rotate_right(grandparent) };
                }
            } else {
                // Symmetric: parent is right child of grandparent.
                let uncle = unsafe { (*node_deref(grandparent)).left.get() };

                if is_red(uncle) {
                    // Case 1: Uncle is red — recolor.
                    set_color(parent, COLOR_BLACK);
                    set_color(uncle, COLOR_BLACK);
                    set_color(grandparent, COLOR_RED);
                    z = grandparent;
                } else {
                    if z == unsafe { (*node_deref(parent)).left.get() } {
                        // Case 2: z is left child — rotate right to transform to case 3.
                        z = parent;
                        unsafe { self.rotate_right(z) };
                    }
                    // Case 3: z is right child — recolor and rotate left.
                    let parent = get_parent(z);
                    let grandparent = get_parent(parent);
                    set_color(parent, COLOR_BLACK);
                    set_color(grandparent, COLOR_RED);
                    unsafe { self.rotate_left(grandparent) };
                }
            }
        }

        // Root must always be black.
        set_color(self.root, COLOR_BLACK);
    }

    // =========================================================================
    // Delete (CLRS)
    // =========================================================================

    /// Unlinks node `z` from the tree and performs rebalancing.
    ///
    /// Does NOT deallocate — caller handles that via `remove_node`.
    ///
    /// # Safety
    ///
    /// - `z` must be non-null and in this tree.
    unsafe fn delete_node(&mut self, z: NodePtr<K, V>) {
        let z_node = unsafe { &*node_deref(z) };
        let z_left = z_node.left.get();
        let z_right = z_node.right.get();
        let z_color = z_node.parent_color.get() & COLOR_MASK;

        let y_original_color: usize;
        let x: NodePtr<K, V>;
        let x_parent: NodePtr<K, V>;

        if z_left.is_null() {
            // No left child — replace z with its right child (possibly null).
            y_original_color = z_color;
            x = z_right;
            x_parent = get_parent(z);
            unsafe { self.transplant(z, z_right) };
        } else if z_right.is_null() {
            // No right child — replace z with its left child.
            y_original_color = z_color;
            x = z_left;
            x_parent = get_parent(z);
            unsafe { self.transplant(z, z_left) };
        } else {
            // Two children — find in-order successor.
            let y = unsafe { tree_minimum(z_right) };
            let y_node = unsafe { &*node_deref(y) };
            y_original_color = y_node.parent_color.get() & COLOR_MASK;
            x = y_node.right.get();

            if get_parent(y) == z {
                // Successor is z's direct right child.
                x_parent = y;
            } else {
                // Successor is deeper in z's right subtree.
                x_parent = get_parent(y);
                unsafe { self.transplant(y, x) };

                // y adopts z's right subtree.
                unsafe { (*node_deref(y)).right.set(z_right) };
                set_parent(z_right, y);
            }

            // Move y into z's position.
            unsafe { self.transplant(z, y) };
            unsafe { (*node_deref(y)).left.set(z_left) };
            set_parent(z_left, y);
            // Copy z's color to y (y takes z's structural position).
            set_color(y, z_color);
        }

        if y_original_color == COLOR_BLACK {
            unsafe { self.delete_fixup(x, x_parent) };
        }
    }

    /// Restores red-black properties after deletion.
    ///
    /// `x` is the node that replaced the removed node (may be null).
    /// `x_parent` tracks x's parent since x may be null.
    ///
    /// At most 3 rotations.
    ///
    /// # Safety
    ///
    /// - `x_parent` must be valid when `x != root`.
    unsafe fn delete_fixup(&mut self, mut x: NodePtr<K, V>, mut x_parent: NodePtr<K, V>) {
        while x != self.root && !is_red(x) {
            // x_parent is non-null: x != root implies x has a parent.
            if x == unsafe { (*node_deref(x_parent)).left.get() } {
                // x is left child — sibling is right child.
                let mut w = unsafe { (*node_deref(x_parent)).right.get() };

                if is_red(w) {
                    // Case 1: Sibling is red.
                    set_color(w, COLOR_BLACK);
                    set_color(x_parent, COLOR_RED);
                    unsafe { self.rotate_left(x_parent) };
                    w = unsafe { (*node_deref(x_parent)).right.get() };
                }

                let w_left = unsafe { (*node_deref(w)).left.get() };
                let w_right = unsafe { (*node_deref(w)).right.get() };

                if !is_red(w_left) && !is_red(w_right) {
                    // Case 2: Both of sibling's children are black.
                    set_color(w, COLOR_RED);
                    x = x_parent;
                    x_parent = get_parent(x);
                } else {
                    if !is_red(w_right) {
                        // Case 3: Sibling's right child is black, left is red.
                        set_color(w_left, COLOR_BLACK);
                        set_color(w, COLOR_RED);
                        unsafe { self.rotate_right(w) };
                        w = unsafe { (*node_deref(x_parent)).right.get() };
                    }
                    // Case 4: Sibling's right child is red.
                    // Copy x_parent's color to w.
                    let parent_color =
                        unsafe { (*node_deref(x_parent)).parent_color.get() } & COLOR_MASK;
                    set_color(w, parent_color);
                    set_color(x_parent, COLOR_BLACK);
                    set_color(unsafe { (*node_deref(w)).right.get() }, COLOR_BLACK);
                    unsafe { self.rotate_left(x_parent) };
                    x = self.root;
                }
            } else {
                // Symmetric: x is right child — sibling is left child.
                let mut w = unsafe { (*node_deref(x_parent)).left.get() };

                if is_red(w) {
                    // Case 1: Sibling is red.
                    set_color(w, COLOR_BLACK);
                    set_color(x_parent, COLOR_RED);
                    unsafe { self.rotate_right(x_parent) };
                    w = unsafe { (*node_deref(x_parent)).left.get() };
                }

                let w_left = unsafe { (*node_deref(w)).left.get() };
                let w_right = unsafe { (*node_deref(w)).right.get() };

                if !is_red(w_right) && !is_red(w_left) {
                    // Case 2: Both of sibling's children are black.
                    set_color(w, COLOR_RED);
                    x = x_parent;
                    x_parent = get_parent(x);
                } else {
                    if !is_red(w_left) {
                        // Case 3: Sibling's left child is black, right is red.
                        set_color(w_right, COLOR_BLACK);
                        set_color(w, COLOR_RED);
                        unsafe { self.rotate_left(w) };
                        w = unsafe { (*node_deref(x_parent)).left.get() };
                    }
                    // Case 4: Sibling's left child is red.
                    // Copy x_parent's color to w.
                    let parent_color =
                        unsafe { (*node_deref(x_parent)).parent_color.get() } & COLOR_MASK;
                    set_color(w, parent_color);
                    set_color(x_parent, COLOR_BLACK);
                    set_color(unsafe { (*node_deref(w)).left.get() }, COLOR_BLACK);
                    unsafe { self.rotate_right(x_parent) };
                    x = self.root;
                }
            }
        }

        set_color(x, COLOR_BLACK);
    }

    // =========================================================================
    // Invariant verification (for testing)
    // =========================================================================

    /// Verifies all red-black tree invariants. Panics on violation.
    ///
    /// Checks:
    /// 1. Root is black
    /// 2. Red nodes have black children
    /// 3. All root-to-nil paths have equal black height
    /// 4. BST ordering holds
    /// 5. Parent pointers are consistent
    /// 6. Leftmost/rightmost cache matches actual extremes
    /// 7. Node count matches `len`
    #[doc(hidden)]
    pub fn verify_invariants(&self) {
        if self.root.is_null() {
            assert!(
                self.leftmost.is_null() && self.rightmost.is_null(),
                "extremes must be null when root is null"
            );
            assert_eq!(self.len, 0, "len must be 0 when root is null");
            return;
        }

        // 1. Root is black.
        assert!(!is_red(self.root), "root must be black");
        // Root's parent must be null.
        assert!(
            get_parent(self.root).is_null(),
            "root's parent must be null"
        );

        // 2-5. Recursive subtree verification.
        let mut black_height: Option<usize> = None;
        let mut count = 0usize;
        Self::verify_subtree(self.root, &mut black_height, 0, &mut count);

        // 6. Leftmost/rightmost.
        let actual_min = unsafe { tree_minimum(self.root) };
        let actual_max = unsafe { tree_maximum(self.root) };
        assert_eq!(self.leftmost, actual_min, "leftmost cache mismatch");
        assert_eq!(self.rightmost, actual_max, "rightmost cache mismatch");

        // 7. Count matches len.
        assert_eq!(
            count, self.len,
            "node count ({count}) != len ({})",
            self.len
        );
    }

    fn verify_subtree(
        ptr: NodePtr<K, V>,
        expected_bh: &mut Option<usize>,
        current_bh: usize,
        count: &mut usize,
    ) {
        if ptr.is_null() {
            // Null is black; check black-height consistency.
            let bh = current_bh + 1;
            match *expected_bh {
                None => *expected_bh = Some(bh),
                Some(expected) => {
                    assert_eq!(
                        bh, expected,
                        "black-height mismatch: got {bh}, expected {expected}"
                    );
                }
            }
            return;
        }

        *count += 1;
        let node = unsafe { &*node_deref(ptr) };

        // Red node must have black children.
        if is_red(ptr) {
            assert!(!is_red(node.left.get()), "red node has red left child");
            assert!(!is_red(node.right.get()), "red node has red right child");
        }

        // BST ordering + parent pointer consistency.
        let left = node.left.get();
        let right = node.right.get();

        if !left.is_null() {
            let left_key = unsafe { &(*node_deref(left)).key };
            assert!(
                *left_key < node.key,
                "BST violation: left key >= parent key"
            );
            assert_eq!(
                get_parent(left),
                ptr,
                "left child's parent pointer mismatch"
            );
        }
        if !right.is_null() {
            let right_key = unsafe { &(*node_deref(right)).key };
            assert!(
                *right_key > node.key,
                "BST violation: right key <= parent key"
            );
            assert_eq!(
                get_parent(right),
                ptr,
                "right child's parent pointer mismatch"
            );
        }

        let next_bh = current_bh + usize::from(!is_red(ptr));
        Self::verify_subtree(left, expected_bh, next_bh, count);
        Self::verify_subtree(right, expected_bh, next_bh, count);
    }
}

// =============================================================================
// impl<A: BoundedAlloc> — try_insert
// =============================================================================

impl<K: Ord, V: 'static, A: BoundedAlloc<Item = RbNode<K, V>>> RbTree<K, V, A> {
    /// Inserts a key-value pair, or returns the pair if the allocator is full.
    ///
    /// If a node with the same key already exists, the value is replaced
    /// and the old value is returned inside `Ok(Some(old_value))`. This
    /// path is zero-allocation.
    ///
    /// Returns `Err(Full((key, value)))` if the allocator cannot allocate.
    #[inline]
    pub fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, Full<(K, V)>> {
        // BST search for insertion point.
        let mut parent: NodePtr<K, V> = ptr::null_mut();
        let mut is_left = true;
        let mut current = self.root;

        while !current.is_null() {
            parent = current;
            // SAFETY: current is non-null and in the tree.
            let node = unsafe { &*node_deref(current) };
            if key == node.key {
                // Key exists: replace value in-place, no allocation.
                // SAFETY: current is valid; &mut self prevents aliasing.
                let existing = unsafe { &mut (*node_deref_mut(current)).value };
                return Ok(Some(std::mem::replace(existing, value)));
            }
            if key < node.key {
                is_left = true;
                current = node.left.get();
            } else {
                is_left = false;
                current = node.right.get();
            }
        }

        match A::try_alloc(RbNode::new(key, value)) {
            Ok(slot) => {
                let ptr = slot.as_ptr();
                self.link_new_node(ptr, parent, is_left);
                Ok(None)
            }
            Err(full) => Err(Full(full.into_inner().into_data())),
        }
    }
}

// =============================================================================
// impl<A: UnboundedAlloc> — insert
// =============================================================================

impl<K: Ord, V: 'static, A: UnboundedAlloc<Item = RbNode<K, V>>> RbTree<K, V, A> {
    /// Inserts a key-value pair. Always succeeds (grows as needed).
    ///
    /// If a node with the same key already exists, the value is replaced
    /// and the old value is returned. This path is zero-allocation.
    #[inline]
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        // BST search for insertion point.
        let mut parent: NodePtr<K, V> = ptr::null_mut();
        let mut is_left = true;
        let mut current = self.root;

        while !current.is_null() {
            parent = current;
            // SAFETY: current is non-null and in the tree.
            let node = unsafe { &*node_deref(current) };
            if key == node.key {
                // SAFETY: current is valid; &mut self prevents aliasing.
                let existing = unsafe { &mut (*node_deref_mut(current)).value };
                return Some(std::mem::replace(existing, value));
            }
            if key < node.key {
                is_left = true;
                current = node.left.get();
            } else {
                is_left = false;
                current = node.right.get();
            }
        }

        let slot = A::alloc(RbNode::new(key, value));
        let ptr = slot.as_ptr();
        self.link_new_node(ptr, parent, is_left);
        None
    }
}

// =============================================================================
// Drop
// =============================================================================

impl<K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> Drop for RbTree<K, V, A> {
    fn drop(&mut self) {
        self.clear();
    }
}

impl<K: Ord + fmt::Debug, V: fmt::Debug + 'static, A: Alloc<Item = RbNode<K, V>>> fmt::Debug
    for RbTree<K, V, A>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

// =============================================================================
// Entry API
// =============================================================================

/// A view into a single entry in the tree, which may be vacant or occupied.
///
/// Constructed via [`RbTree::entry`].
pub enum Entry<'a, K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> {
    /// An occupied entry — key exists in the tree.
    Occupied(OccupiedEntry<'a, K, V, A>),
    /// A vacant entry — key does not exist.
    Vacant(VacantEntry<'a, K, V, A>),
}

/// A view into an occupied entry in the tree.
pub struct OccupiedEntry<'a, K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> {
    tree: &'a mut RbTree<K, V, A>,
    ptr: NodePtr<K, V>,
}

/// A view into a vacant entry in the tree.
pub struct VacantEntry<'a, K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> {
    tree: &'a mut RbTree<K, V, A>,
    key: K,
    parent: NodePtr<K, V>,
    is_left: bool,
}

// -- Entry: base Alloc methods --

impl<K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> Entry<'_, K, V, A> {
    /// Returns a reference to this entry's key.
    #[inline]
    pub fn key(&self) -> &K {
        match self {
            Entry::Occupied(e) => e.key(),
            Entry::Vacant(e) => e.key(),
        }
    }

    /// Modifies an existing entry before potential insertion.
    ///
    /// If the entry is occupied, calls `f` with `&mut V`.
    /// If vacant, this is a no-op.
    #[inline]
    pub fn and_modify<F: FnOnce(&mut V)>(mut self, f: F) -> Self {
        if let Entry::Occupied(ref mut e) = self {
            f(e.get_mut());
        }
        self
    }
}

// -- Entry: BoundedAlloc methods --

impl<'a, K: Ord, V: 'static, A: BoundedAlloc<Item = RbNode<K, V>>> Entry<'a, K, V, A> {
    /// Ensures a value is in the entry by inserting if vacant (bounded).
    ///
    /// Returns `Err(Full((K, V)))` if the allocator is full and the entry
    /// was vacant.
    #[inline]
    pub fn or_try_insert(self, value: V) -> Result<&'a mut V, Full<(K, V)>> {
        match self {
            Entry::Occupied(e) => Ok(e.into_mut()),
            Entry::Vacant(e) => e.try_insert(value),
        }
    }

    /// Ensures a value by inserting the result of `f` if vacant (bounded).
    #[inline]
    pub fn or_try_insert_with<F: FnOnce() -> V>(self, f: F) -> Result<&'a mut V, Full<(K, V)>> {
        match self {
            Entry::Occupied(e) => Ok(e.into_mut()),
            Entry::Vacant(e) => e.try_insert(f()),
        }
    }

    /// Ensures a value by inserting `f(key)` if vacant (bounded).
    #[inline]
    pub fn or_try_insert_with_key<F: FnOnce(&K) -> V>(
        self,
        f: F,
    ) -> Result<&'a mut V, Full<(K, V)>> {
        match self {
            Entry::Occupied(e) => Ok(e.into_mut()),
            Entry::Vacant(e) => {
                let value = f(e.key());
                e.try_insert(value)
            }
        }
    }

    /// Ensures a value by inserting `V::default()` if vacant (bounded).
    #[inline]
    pub fn or_try_insert_default(self) -> Result<&'a mut V, Full<(K, V)>>
    where
        V: Default,
    {
        self.or_try_insert(V::default())
    }
}

// -- Entry: UnboundedAlloc methods --

impl<'a, K: Ord, V: 'static, A: UnboundedAlloc<Item = RbNode<K, V>>> Entry<'a, K, V, A> {
    /// Ensures a value is in the entry by inserting if vacant (unbounded).
    #[inline]
    pub fn or_insert(self, value: V) -> &'a mut V {
        match self {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => e.insert(value),
        }
    }

    /// Ensures a value by inserting the result of `f` if vacant (unbounded).
    #[inline]
    pub fn or_insert_with<F: FnOnce() -> V>(self, f: F) -> &'a mut V {
        match self {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => e.insert(f()),
        }
    }

    /// Ensures a value by inserting `f(key)` if vacant (unbounded).
    #[inline]
    pub fn or_insert_with_key<F: FnOnce(&K) -> V>(self, f: F) -> &'a mut V {
        match self {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let value = f(e.key());
                e.insert(value)
            }
        }
    }

    /// Ensures a value by inserting `V::default()` if vacant (unbounded).
    #[inline]
    pub fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert(V::default())
    }
}

// -- OccupiedEntry --

impl<'a, K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> OccupiedEntry<'a, K, V, A> {
    /// Returns a reference to the key.
    #[inline]
    pub fn key(&self) -> &K {
        // SAFETY: ptr is valid, node is in the tree.
        unsafe { &(*node_deref(self.ptr)).key }
    }

    /// Returns a reference to the value.
    #[inline]
    pub fn get(&self) -> &V {
        // SAFETY: ptr is valid, node is in the tree.
        unsafe { &(*node_deref(self.ptr)).value }
    }

    /// Returns a mutable reference to the value.
    #[inline]
    pub fn get_mut(&mut self) -> &mut V {
        // SAFETY: ptr is valid, &mut self prevents aliasing.
        unsafe { &mut (*node_deref_mut(self.ptr)).value }
    }

    /// Converts to a mutable reference to the value with the entry's lifetime.
    #[inline]
    pub fn into_mut(self) -> &'a mut V {
        // SAFETY: ptr is valid, the entry consumed &'a mut RbTree,
        // so the returned reference continues that exclusive borrow.
        unsafe { &mut (*node_deref_mut(self.ptr)).value }
    }

    /// Sets the value of the entry and returns the old value.
    #[inline]
    pub fn insert(&mut self, value: V) -> V {
        // SAFETY: ptr is valid and occupied; &mut self prevents aliasing.
        let node = unsafe { &mut *node_deref_mut(self.ptr) };
        std::mem::replace(&mut node.value, value)
    }

    /// Removes the entry and returns `(key, value)`.
    #[inline]
    pub fn remove(self) -> (K, V) {
        self.tree.remove_node(self.ptr)
    }
}

// -- VacantEntry: BoundedAlloc --

impl<'a, K: Ord, V: 'static, A: BoundedAlloc<Item = RbNode<K, V>>> VacantEntry<'a, K, V, A> {
    /// Inserts a value into the vacant entry (bounded allocator).
    ///
    /// Returns `Err(Full((K, V)))` if the allocator is full.
    #[inline]
    pub fn try_insert(self, value: V) -> Result<&'a mut V, Full<(K, V)>> {
        let VacantEntry {
            tree,
            key,
            parent,
            is_left,
        } = self;
        match A::try_alloc(RbNode::new(key, value)) {
            Ok(slot) => {
                let val_ptr = unsafe { tree.link_vacant(slot, parent, is_left) };
                // SAFETY: val_ptr points to slab memory owned by the tree.
                Ok(unsafe { &mut *val_ptr })
            }
            Err(full) => Err(Full(full.into_inner().into_data())),
        }
    }
}

// -- VacantEntry: UnboundedAlloc --

impl<'a, K: Ord, V: 'static, A: UnboundedAlloc<Item = RbNode<K, V>>> VacantEntry<'a, K, V, A> {
    /// Inserts a value into the vacant entry (unbounded allocator).
    #[inline]
    pub fn insert(self, value: V) -> &'a mut V {
        let VacantEntry {
            tree,
            key,
            parent,
            is_left,
        } = self;
        let slot = A::alloc(RbNode::new(key, value));
        let val_ptr = unsafe { tree.link_vacant(slot, parent, is_left) };
        // SAFETY: val_ptr points to slab memory owned by the tree.
        unsafe { &mut *val_ptr }
    }
}

// -- VacantEntry: base Alloc (key accessor) --

impl<K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> VacantEntry<'_, K, V, A> {
    /// Returns a reference to the key that would be used for insertion.
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }
}

// =============================================================================
// Iter
// =============================================================================

/// Iterator over `(&K, &V)` pairs in sorted order.
pub struct Iter<'a, K, V> {
    front: NodePtr<K, V>,
    len: usize,
    _marker: PhantomData<&'a ()>,
}

impl<'a, K: 'a, V: 'a> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }

        let ptr = self.front;
        // SAFETY: len > 0 implies front is non-null and points to an occupied slot.
        let node = unsafe { &*node_deref(ptr) };
        self.front = unsafe { successor(ptr) };
        self.len -= 1;
        prefetch_read_node(self.front);
        Some((&node.key, &node.value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

impl<'a, K: 'a, V: 'a> ExactSizeIterator for Iter<'a, K, V> {}

impl<'a, K: Ord + 'a, V: 'static, A: Alloc<Item = RbNode<K, V>>> IntoIterator
    for &'a RbTree<K, V, A>
{
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, K: Ord + 'a, V: 'static, A: Alloc<Item = RbNode<K, V>>> IntoIterator
    for &'a mut RbTree<K, V, A>
{
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

// =============================================================================
// Keys / Values
// =============================================================================

/// Iterator over keys in sorted order.
pub struct Keys<'a, K, V> {
    inner: Iter<'a, K, V>,
}

impl<'a, K: 'a, V: 'a> Iterator for Keys<'a, K, V> {
    type Item = &'a K;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(k, _)| k)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'a, K: 'a, V: 'a> ExactSizeIterator for Keys<'a, K, V> {}

/// Iterator over values in key-sorted order.
pub struct Values<'a, K, V> {
    inner: Iter<'a, K, V>,
}

impl<'a, K: 'a, V: 'a> Iterator for Values<'a, K, V> {
    type Item = &'a V;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, v)| v)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'a, K: 'a, V: 'a> ExactSizeIterator for Values<'a, K, V> {}

// =============================================================================
// IterMut
// =============================================================================

/// Mutable iterator over `(&K, &mut V)` pairs in sorted order.
///
/// Keys are immutable — changing them would violate sorted order.
pub struct IterMut<'a, K, V> {
    front: NodePtr<K, V>,
    len: usize,
    _marker: PhantomData<&'a mut ()>,
}

impl<'a, K: 'a, V: 'a> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }

        let ptr = self.front;
        // SAFETY: Advance to successor FIRST — successor() reads parent/child
        // pointers via shared references. Creating &mut to the same node
        // afterward avoids invalidating those reads under Stacked Borrows.
        let next = unsafe { successor(ptr) };
        let node = unsafe { &mut *node_deref_mut(ptr) };
        self.front = next;
        self.len -= 1;
        prefetch_read_node(self.front);
        Some((&node.key, &mut node.value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

impl<'a, K: 'a, V: 'a> ExactSizeIterator for IterMut<'a, K, V> {}

// =============================================================================
// ValuesMut
// =============================================================================

/// Mutable iterator over values in key-sorted order.
pub struct ValuesMut<'a, K, V> {
    inner: IterMut<'a, K, V>,
}

impl<'a, K: 'a, V: 'a> Iterator for ValuesMut<'a, K, V> {
    type Item = &'a mut V;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, v)| v)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'a, K: 'a, V: 'a> ExactSizeIterator for ValuesMut<'a, K, V> {}

// =============================================================================
// Range
// =============================================================================

/// Iterator over `(&K, &V)` pairs within a key range.
///
/// Forward-only. `end` is the exclusive sentinel — iteration stops when
/// `front == end` or `front` is null.
pub struct Range<'a, K, V> {
    front: NodePtr<K, V>,
    end: NodePtr<K, V>,
    _marker: PhantomData<&'a ()>,
}

impl<'a, K: 'a, V: 'a> Iterator for Range<'a, K, V> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.front.is_null() || self.front == self.end {
            return None;
        }

        let ptr = self.front;
        // SAFETY: front is non-null and points to an occupied slot.
        let node = unsafe { &*node_deref(ptr) };
        self.front = unsafe { successor(ptr) };
        prefetch_read_node(self.front);
        Some((&node.key, &node.value))
    }
}

// =============================================================================
// RangeMut
// =============================================================================

/// Mutable iterator over `(&K, &mut V)` pairs within a key range.
///
/// Keys are immutable — changing them would violate sorted order.
/// Forward-only. `end` is the exclusive sentinel.
pub struct RangeMut<'a, K, V> {
    front: NodePtr<K, V>,
    end: NodePtr<K, V>,
    _marker: PhantomData<&'a mut ()>,
}

impl<'a, K: 'a, V: 'a> Iterator for RangeMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.front.is_null() || self.front == self.end {
            return None;
        }

        let ptr = self.front;
        // SAFETY: See IterMut::next — successor() must precede &mut creation
        // to avoid conflicting Stacked Borrows reborrows on the same node.
        let next = unsafe { successor(ptr) };
        let node = unsafe { &mut *node_deref_mut(ptr) };
        self.front = next;
        prefetch_read_node(self.front);
        Some((&node.key, &mut node.value))
    }
}

// =============================================================================
// Cursor
// =============================================================================

/// Cursor for positional traversal with removal.
///
/// Uses parent pointers for O(1) amortized advance.
///
/// # Example
///
/// ```ignore
/// let mut cursor = tree.cursor_front();
/// while cursor.advance() {
///     if *cursor.value().unwrap() > threshold {
///         let (k, v) = cursor.remove().unwrap();
///         // cursor auto-advances to next
///     }
/// }
/// ```
pub struct Cursor<'a, K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> {
    tree: &'a mut RbTree<K, V, A>,
    current: NodePtr<K, V>,
    started: bool,
}

impl<K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> Cursor<'_, K, V, A> {
    /// Returns a reference to the current key, or `None` if not positioned.
    #[inline]
    pub fn key(&self) -> Option<&K> {
        if self.current.is_null() {
            return None;
        }
        // SAFETY: current is non-null, in the tree.
        Some(unsafe { &(*node_deref(self.current)).key })
    }

    /// Returns a reference to the current value, or `None` if not positioned.
    #[inline]
    pub fn value(&self) -> Option<&V> {
        if self.current.is_null() {
            return None;
        }
        // SAFETY: current is non-null, in the tree.
        Some(unsafe { &(*node_deref(self.current)).value })
    }

    /// Returns a mutable reference to the current value, or `None` if not positioned.
    #[inline]
    pub fn value_mut(&mut self) -> Option<&mut V> {
        if self.current.is_null() {
            return None;
        }
        // SAFETY: current is non-null, &mut self prevents aliasing.
        Some(unsafe { &mut (*node_deref_mut(self.current)).value })
    }

    /// Advances the cursor to the next element.
    ///
    /// Returns `true` if the cursor is now at a valid element.
    #[inline]
    pub fn advance(&mut self) -> bool {
        if !self.started {
            self.started = true;
            self.current = self.tree.leftmost;
            prefetch_read_node(self.current);
            return !self.current.is_null();
        }

        if self.current.is_null() {
            return false;
        }

        self.current = unsafe { successor(self.current) };
        prefetch_read_node(self.current);
        !self.current.is_null()
    }

    /// Removes the current element and advances to the next.
    ///
    /// Returns the removed `(key, value)`, or `None` if not positioned.
    #[inline]
    pub fn remove(&mut self) -> Option<(K, V)> {
        if self.current.is_null() {
            return None;
        }

        let ptr = self.current;
        // Save successor BEFORE delete — after delete, ptr is freed.
        let next = unsafe { successor(ptr) };

        let result = self.tree.remove_node(ptr);
        self.current = next;
        Some(result)
    }
}

// =============================================================================
// Drain
// =============================================================================

/// Draining iterator that removes and returns all key-value pairs
/// in sorted (ascending) order.
///
/// When dropped, any remaining nodes are freed via the allocator.
pub struct Drain<'a, K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> {
    tree: &'a mut RbTree<K, V, A>,
}

impl<K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> Iterator for Drain<'_, K, V, A> {
    type Item = (K, V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.tree.pop_first()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.tree.len(), Some(self.tree.len()))
    }
}

impl<K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> ExactSizeIterator for Drain<'_, K, V, A> {}

impl<K: Ord, V: 'static, A: Alloc<Item = RbNode<K, V>>> Drop for Drain<'_, K, V, A> {
    fn drop(&mut self) {
        self.tree.clear();
    }
}
