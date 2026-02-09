//! B-tree sorted map with internal slab allocation.
//!
//! # Design
//!
//! A cache-friendly sorted map where each node stores up to B-1 key-value
//! pairs and up to B child pointers. High fanout means fewer pointer chases
//! per lookup — at B=8 a 10k-entry tree is only 3-4 levels deep.
//!
//! # Allocation Model
//!
//! Same as [`RbTree`](crate::rbtree::RbTree) — the tree takes a ZST allocator
//! at construction time and handles all node allocation/deallocation internally.
//!
//! - Bounded allocators: `try_insert` returns `Err(Full((K, V)))` when full
//! - Unbounded allocators: `insert` always succeeds (grows as needed)
//!
//! # Example
//!
//! ```ignore
//! mod levels {
//!     nexus_collections::btree_allocator!(u64, String, bounded);
//! }
//!
//! levels::Allocator::builder().capacity(1000).build().unwrap();
//!
//! let mut map = levels::BTree::new(levels::Allocator);
//! map.try_insert(100, "hello".into()).unwrap();
//!
//! assert_eq!(map.get(&100), Some(&"hello".into()));
//! ```

use std::fmt;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ptr;

use nexus_slab::{Alloc, BoundedAlloc, Full, Slot, SlotCell, UnboundedAlloc};

// =============================================================================
// Constants
// =============================================================================

/// Maximum tree depth for stack-based traversal.
/// For B=4 at 10M entries: ~23 levels. 32 is generous headroom.
const MAX_DEPTH: usize = 32;

// =============================================================================
// NodePtr
// =============================================================================

/// Raw pointer to a slab-allocated B-tree node.
type NodePtr<K, V, const B: usize> = *mut SlotCell<BTreeNode<K, V, B>>;

// =============================================================================
// BTreeNode<K, V, B>
// =============================================================================

/// A node in a B-tree sorted map.
///
/// Stores up to B-1 keys/values and up to B children. B must be even
/// and >= 4. Keys and values are in separate arrays so key scans during
/// search only touch key cache lines.
#[repr(C)]
pub struct BTreeNode<K, V, const B: usize> {
    len: u16,
    leaf: bool,
    keys: [MaybeUninit<K>; B],
    values: [MaybeUninit<V>; B],
    children: [NodePtr<K, V, B>; B],
}

impl<K, V, const B: usize> BTreeNode<K, V, B> {
    #[inline]
    fn new_leaf() -> Self {
        BTreeNode {
            len: 0,
            leaf: true,
            // SAFETY: MaybeUninit does not require initialization.
            keys: unsafe { MaybeUninit::uninit().assume_init() },
            values: unsafe { MaybeUninit::uninit().assume_init() },
            children: [ptr::null_mut(); B],
        }
    }

    #[inline]
    fn new_internal() -> Self {
        BTreeNode {
            len: 0,
            leaf: false,
            // SAFETY: MaybeUninit does not require initialization.
            keys: unsafe { MaybeUninit::uninit().assume_init() },
            values: unsafe { MaybeUninit::uninit().assume_init() },
            children: [ptr::null_mut(); B],
        }
    }
}

// =============================================================================
// node_deref
// =============================================================================

/// Dereferences a `NodePtr` to `*const BTreeNode`.
///
/// # Safety
///
/// `ptr` must be non-null and point to an occupied `SlotCell`.
#[inline]
unsafe fn node_deref<K, V, const B: usize>(ptr: NodePtr<K, V, B>) -> *const BTreeNode<K, V, B> {
    // SAFETY: Caller guarantees ptr is non-null and occupied.
    // ManuallyDrop<MaybeUninit<T>> has the same layout as T.
    unsafe { std::ptr::addr_of!((*ptr).value).cast() }
}

/// Dereferences a `NodePtr` to `*mut BTreeNode`.
///
/// # Safety
///
/// `ptr` must be non-null, occupied, and unaliased.
#[inline]
unsafe fn node_deref_mut<K, V, const B: usize>(ptr: NodePtr<K, V, B>) -> *mut BTreeNode<K, V, B> {
    // SAFETY: Caller guarantees ptr is non-null, occupied, and unaliased.
    unsafe { std::ptr::addr_of_mut!((*ptr).value).cast() }
}

// =============================================================================
// Node accessor helpers
// =============================================================================

#[inline]
unsafe fn node_len<K, V, const B: usize>(ptr: NodePtr<K, V, B>) -> usize {
    // SAFETY: Caller guarantees ptr is non-null and occupied.
    unsafe { (*node_deref(ptr)).len as usize }
}

#[inline]
unsafe fn node_is_leaf<K, V, const B: usize>(ptr: NodePtr<K, V, B>) -> bool {
    // SAFETY: Caller guarantees ptr is non-null and occupied.
    unsafe { (*node_deref(ptr)).leaf }
}

/// # Safety
///
/// `ptr` must be non-null and occupied, `i < node.len`.
#[inline]
unsafe fn key_at<'a, K, V, const B: usize>(ptr: NodePtr<K, V, B>, i: usize) -> &'a K {
    // SAFETY: i < len, so keys[i] is initialized.
    unsafe { (*node_deref(ptr)).keys[i].assume_init_ref() }
}

/// # Safety
///
/// `ptr` must be non-null and occupied, `i < node.len`.
#[inline]
unsafe fn value_at<'a, K, V, const B: usize>(ptr: NodePtr<K, V, B>, i: usize) -> &'a V {
    // SAFETY: i < len, so values[i] is initialized.
    unsafe { (*node_deref(ptr)).values[i].assume_init_ref() }
}

/// # Safety
///
/// `ptr` must be non-null, occupied, unaliased, `i < node.len`.
#[inline]
unsafe fn value_at_mut<'a, K, V, const B: usize>(ptr: NodePtr<K, V, B>, i: usize) -> &'a mut V {
    // SAFETY: i < len, unaliased.
    unsafe { (*node_deref_mut(ptr)).values[i].assume_init_mut() }
}

/// # Safety
///
/// `ptr` must be non-null, occupied, internal node, `i <= node.len`.
#[inline]
unsafe fn child_at<K, V, const B: usize>(ptr: NodePtr<K, V, B>, i: usize) -> NodePtr<K, V, B> {
    // SAFETY: i <= len for internal nodes.
    unsafe { (*node_deref(ptr)).children[i] }
}

/// Linear scan of keys. Returns `(index, found)`.
///
/// At B=8 (max 7 keys), linear scan outperforms binary search because the
/// sequential access pattern is well-predicted by the CPU. Binary search's
/// data-dependent branching hurts more than the extra comparisons cost.
///
/// # Safety
///
/// `ptr` must be non-null and occupied.
#[inline]
unsafe fn search_in_node<K: Ord, V, const B: usize>(
    ptr: NodePtr<K, V, B>,
    key: &K,
) -> (usize, bool) {
    let node = unsafe { &*node_deref(ptr) };
    let len = node.len as usize;
    let mut i = 0;
    while i < len {
        // SAFETY: i < len, so keys[i] is initialized.
        let k = unsafe { node.keys[i].assume_init_ref() };
        if *key == *k {
            return (i, true);
        }
        if *key < *k {
            return (i, false);
        }
        i += 1;
    }
    (len, false)
}

// =============================================================================
// Node mutation helpers
// =============================================================================

/// Reads key and value at index `i` out of the node (bitwise copy).
///
/// # Safety
///
/// `ptr` must be non-null, occupied, `i < node.len`. The caller takes
/// ownership; the slot must not be read again without reinitializing.
#[inline]
unsafe fn take_kv<K, V, const B: usize>(ptr: NodePtr<K, V, B>, i: usize) -> (K, V) {
    let node = unsafe { &*node_deref(ptr) };
    // SAFETY: i < len, so slots are initialized.
    let k = unsafe { node.keys[i].assume_init_read() };
    let v = unsafe { node.values[i].assume_init_read() };
    (k, v)
}

/// Shifts keys/values at `[i..len)` right by one, and children at
/// `[i+1..=len]` right by one. Makes room at key/value index `i`
/// and child index `i+1`.
///
/// # Safety
///
/// `ptr` must be non-null, occupied, unaliased. `i <= len < B-1`.
#[inline]
unsafe fn shift_right<K, V, const B: usize>(ptr: NodePtr<K, V, B>, i: usize) {
    let node = unsafe { &mut *node_deref_mut(ptr) };
    let len = node.len as usize;
    if i < len {
        // SAFETY: [i..len) is initialized, [i+1..len+1) is within B.
        // Both src and dst derived from a single as_mut_ptr() to avoid
        // conflicting Stacked Borrows reborrows.
        unsafe {
            let kp = node.keys.as_mut_ptr();
            ptr::copy(kp.add(i).cast_const(), kp.add(i + 1), len - i);
            let vp = node.values.as_mut_ptr();
            ptr::copy(vp.add(i).cast_const(), vp.add(i + 1), len - i);
        }
    }
    if !node.leaf && i < len {
        // SAFETY: children[i+1..=len] → children[i+2..=len+1].
        unsafe {
            let cp = node.children.as_mut_ptr();
            ptr::copy(cp.add(i + 1).cast_const(), cp.add(i + 2), len - i);
        }
    }
}

/// Shifts keys/values at `[i+1..len)` left by one (overwriting index `i`),
/// and children at `[i+2..=len]` left by one (overwriting `children[i+1]`).
/// Decrements `node.len`.
///
/// Caller must have already read key/value at index `i`.
///
/// # Safety
///
/// `ptr` must be non-null, occupied, unaliased. `i < len`.
#[inline]
unsafe fn shift_left<K, V, const B: usize>(ptr: NodePtr<K, V, B>, i: usize) {
    let node = unsafe { &mut *node_deref_mut(ptr) };
    let len = node.len as usize;
    if i + 1 < len {
        // SAFETY: [i+1..len) is initialized. Single as_mut_ptr() origin
        // avoids conflicting Stacked Borrows reborrows.
        unsafe {
            let kp = node.keys.as_mut_ptr();
            ptr::copy(kp.add(i + 1).cast_const(), kp.add(i), len - i - 1);
            let vp = node.values.as_mut_ptr();
            ptr::copy(vp.add(i + 1).cast_const(), vp.add(i), len - i - 1);
        }
    }
    if !node.leaf && i + 2 <= len {
        // SAFETY: children[i+2..=len] → children[i+1..=len-1].
        unsafe {
            let cp = node.children.as_mut_ptr();
            ptr::copy(cp.add(i + 2).cast_const(), cp.add(i + 1), len - i - 1);
        }
    }
    node.len -= 1;
}

/// Drops all initialized keys/values and frees the node slot.
///
/// # Safety
///
/// `ptr` must be non-null, occupied. Node must not be referenced afterward.
unsafe fn free_node<K, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize>(
    ptr: NodePtr<K, V, B>,
) {
    let node = unsafe { &mut *node_deref_mut(ptr) };
    for i in 0..node.len as usize {
        // SAFETY: [0..len) is initialized.
        unsafe {
            node.keys[i].assume_init_drop();
            node.values[i].assume_init_drop();
        }
    }
    // SAFETY: ptr is a valid occupied slot.
    let slot = unsafe { Slot::from_ptr(ptr) };
    unsafe { A::free(slot) };
}

/// Core split logic: splits a full child, pushing median to parent.
///
/// `right_ptr` is an already-allocated node for the right half.
///
/// # Safety
///
/// - `parent` must be non-null, occupied, unaliased, with `len < B-1`.
/// - `child_idx <= parent.len`.
/// - `children[child_idx]` must have `len == B-1`.
/// - `right_ptr` must be a freshly allocated node of the same leaf/internal type.
unsafe fn split_child_core<K, V, const B: usize>(
    parent: NodePtr<K, V, B>,
    child_idx: usize,
    right_ptr: NodePtr<K, V, B>,
) {
    let child = unsafe { child_at(parent, child_idx) };
    let child_node = unsafe { &mut *node_deref_mut(child) };
    let right_node = unsafe { &mut *node_deref_mut(right_ptr) };
    let child_is_leaf = child_node.leaf;

    // mid is the median index. For B=8: (8-1)/2 = 3.
    let mid = (B - 1) / 2;
    let right_len = B - 1 - mid - 1;

    // Copy keys[mid+1..B-1) and values[mid+1..B-1) to right.
    // SAFETY: mid+1..B-1 is initialized (child is full).
    unsafe {
        ptr::copy_nonoverlapping(
            child_node.keys.as_ptr().add(mid + 1),
            right_node.keys.as_mut_ptr(),
            right_len,
        );
        ptr::copy_nonoverlapping(
            child_node.values.as_ptr().add(mid + 1),
            right_node.values.as_mut_ptr(),
            right_len,
        );
    }

    if !child_is_leaf {
        // SAFETY: children[mid+1..=B-1] is valid for internal nodes.
        unsafe {
            ptr::copy_nonoverlapping(
                child_node.children.as_ptr().add(mid + 1),
                right_node.children.as_mut_ptr(),
                right_len + 1,
            );
        }
    }
    right_node.len = right_len as u16;
    right_node.leaf = child_is_leaf;

    // Extract median.
    // SAFETY: mid < B-1, so keys[mid]/values[mid] are initialized.
    let median_key = unsafe { child_node.keys[mid].assume_init_read() };
    let median_value = unsafe { child_node.values[mid].assume_init_read() };
    child_node.len = mid as u16;

    // Make room in parent at child_idx.
    // SAFETY: parent.len < B-1, so there's room.
    unsafe { shift_right(parent, child_idx) };

    let parent_node = unsafe { &mut *node_deref_mut(parent) };
    parent_node.keys[child_idx] = MaybeUninit::new(median_key);
    parent_node.values[child_idx] = MaybeUninit::new(median_value);
    parent_node.children[child_idx + 1] = right_ptr;
    parent_node.len += 1;
}

// =============================================================================
// Prefetch
// =============================================================================

#[inline(always)]
fn prefetch_read_node<K, V, const B: usize>(ptr: NodePtr<K, V, B>) {
    #[cfg(target_arch = "x86_64")]
    if !ptr.is_null() {
        // SAFETY: _mm_prefetch on an invalid address is architecturally a NOP on x86.
        unsafe {
            std::arch::x86_64::_mm_prefetch(ptr as *const i8, std::arch::x86_64::_MM_HINT_T0);
        }
    }
}

// =============================================================================
// BTree<K, V, A, B>
// =============================================================================

/// A cache-friendly sorted map with internal slab allocation.
///
/// # Complexity
///
/// | Operation    | Time            |
/// |--------------|-----------------|
/// | get / get_mut| O(B * log_B n)  |
/// | insert       | O(B * log_B n)  |
/// | remove       | O(B * log_B n)  |
/// | first / last | O(log_B n)      |
/// | pop_first    | O(B * log_B n)  |
/// | range scan   | O(B * log_B n + k) |
pub struct BTree<K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> {
    root: NodePtr<K, V, B>,
    len: usize,
    depth: usize,
    _marker: PhantomData<A>,
}

// =============================================================================
// impl<A: Alloc> — base block
// =============================================================================

impl<K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> BTree<K, V, A, B> {
    /// Creates a new empty B-tree.
    ///
    /// # Panics
    ///
    /// Panics if `B < 4`, `B` is odd, or `BTreeNode<K, V, B>` exceeds 1024 bytes.
    #[inline]
    #[allow(unused_variables, clippy::needless_pass_by_value)]
    pub fn new(alloc: A) -> Self {
        assert!(B >= 4, "B must be >= 4");
        assert!(
            B % 2 == 0,
            "B must be even (so splits produce balanced nodes)"
        );
        assert!(
            std::mem::size_of::<BTreeNode<K, V, B>>() <= 1024,
            "BTreeNode exceeds 1024 bytes — reduce B or use smaller K/V types"
        );
        BTree {
            root: ptr::null_mut(),
            len: 0,
            depth: 0,
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
        let (ptr, idx) = self.find(key)?;
        // SAFETY: find returns valid node with key at idx.
        Some(unsafe { value_at(ptr, idx) })
    }

    /// Returns a mutable reference to the value for the given key.
    #[inline]
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let (ptr, idx) = self.find(key)?;
        // SAFETY: find returns valid node; &mut self prevents aliasing.
        Some(unsafe { value_at_mut(ptr, idx) })
    }

    /// Returns references to the key and value for the given key.
    #[inline]
    pub fn get_key_value(&self, key: &K) -> Option<(&K, &V)> {
        let (ptr, idx) = self.find(key)?;
        // SAFETY: find returns valid node with key at idx.
        Some(unsafe { (key_at(ptr, idx), value_at(ptr, idx)) })
    }

    /// Returns the first (smallest) key-value pair.
    ///
    /// O(log_B n) — descends to leftmost leaf.
    #[inline]
    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        if self.root.is_null() {
            return None;
        }
        let mut current = self.root;
        // SAFETY: current is non-null and in the tree.
        loop {
            if unsafe { node_is_leaf(current) } {
                return Some(unsafe { (key_at(current, 0), value_at(current, 0)) });
            }
            current = unsafe { child_at(current, 0) };
        }
    }

    /// Returns the last (largest) key-value pair.
    ///
    /// O(log_B n) — descends to rightmost leaf.
    #[inline]
    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        if self.root.is_null() {
            return None;
        }
        let mut current = self.root;
        // SAFETY: current is non-null and in the tree.
        loop {
            let len = unsafe { node_len(current) };
            if unsafe { node_is_leaf(current) } {
                return Some(unsafe { (key_at(current, len - 1), value_at(current, len - 1)) });
            }
            current = unsafe { child_at(current, len) };
        }
    }

    // =========================================================================
    // Remove / pop / clear
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
        if self.root.is_null() {
            return None;
        }

        // Build descent path while searching.
        let mut path: [(NodePtr<K, V, B>, usize); MAX_DEPTH] = [(ptr::null_mut(), 0); MAX_DEPTH];
        let mut path_len = 0usize;
        let mut current = self.root;

        loop {
            // SAFETY: current is non-null and in the tree.
            let (idx, found) = unsafe { search_in_node(current, key) };
            if found {
                let result = unsafe { self.remove_found(current, idx, &path, path_len) };
                self.len -= 1;
                return Some(result);
            }
            if unsafe { node_is_leaf(current) } {
                return None;
            }
            debug_assert!(path_len < MAX_DEPTH, "path overflow in remove_entry");
            path[path_len] = (current, idx);
            path_len += 1;
            let next = unsafe { child_at(current, idx) };
            prefetch_read_node(next);
            current = next;
        }
    }

    /// Removes and returns the first (smallest) key-value pair.
    #[inline]
    pub fn pop_first(&mut self) -> Option<(K, V)> {
        if self.root.is_null() {
            return None;
        }

        let mut path: [(NodePtr<K, V, B>, usize); MAX_DEPTH] = [(ptr::null_mut(), 0); MAX_DEPTH];
        let mut path_len = 0usize;
        let mut current = self.root;

        // SAFETY: root is non-null.
        while !unsafe { node_is_leaf(current) } {
            debug_assert!(path_len < MAX_DEPTH, "path overflow in pop_first");
            path[path_len] = (current, 0);
            path_len += 1;
            let next = unsafe { child_at(current, 0) };
            prefetch_read_node(next);
            current = next;
        }

        // current is the leftmost leaf, remove key[0].
        // SAFETY: tree is non-empty, so leaf has at least 1 key.
        let result = unsafe { take_kv(current, 0) };
        unsafe { shift_left(current, 0) };
        self.fixup_after_remove(current, &path, path_len);
        self.len -= 1;
        Some(result)
    }

    /// Removes and returns the last (largest) key-value pair.
    #[inline]
    pub fn pop_last(&mut self) -> Option<(K, V)> {
        if self.root.is_null() {
            return None;
        }

        let mut path: [(NodePtr<K, V, B>, usize); MAX_DEPTH] = [(ptr::null_mut(), 0); MAX_DEPTH];
        let mut path_len = 0usize;
        let mut current = self.root;

        // SAFETY: root is non-null.
        while !unsafe { node_is_leaf(current) } {
            let len = unsafe { node_len(current) };
            debug_assert!(path_len < MAX_DEPTH, "path overflow in pop_last");
            path[path_len] = (current, len);
            path_len += 1;
            let next = unsafe { child_at(current, len) };
            prefetch_read_node(next);
            current = next;
        }

        // current is the rightmost leaf, remove last key.
        let last = unsafe { node_len(current) } - 1;
        let result = unsafe { take_kv(current, last) };
        // No shift needed — just decrement len.
        unsafe { (*node_deref_mut(current)).len -= 1 };
        self.fixup_after_remove(current, &path, path_len);
        self.len -= 1;
        Some(result)
    }

    /// Removes all nodes, freeing them via the allocator.
    #[inline]
    pub fn clear(&mut self) {
        if !self.root.is_null() {
            // SAFETY: root is non-null and in the tree.
            unsafe { Self::clear_subtree(self.root) };
        }
        self.root = ptr::null_mut();
        self.len = 0;
        self.depth = 0;
    }

    // =========================================================================
    // Entry API
    // =========================================================================

    /// Gets the entry for the given key.
    ///
    /// # Example
    ///
    /// ```ignore
    /// map.entry(100).or_try_insert("hello".into()).unwrap();
    /// map.entry(100).and_modify(|v| *v = "world".into());
    /// assert_eq!(map.get(&100), Some(&"world".into()));
    /// ```
    #[inline]
    pub fn entry(&mut self, key: K) -> Entry<'_, K, V, A, B> {
        let mut current = self.root;
        while !current.is_null() {
            // SAFETY: current is non-null and in the tree.
            let (idx, found) = unsafe { search_in_node(current, &key) };
            if found {
                drop(key);
                return Entry::Occupied(OccupiedEntry {
                    tree: self,
                    node: current,
                    idx,
                });
            }
            if unsafe { node_is_leaf(current) } {
                break;
            }
            let next = unsafe { child_at(current, idx) };
            prefetch_read_node(next);
            current = next;
        }

        Entry::Vacant(VacantEntry { tree: self, key })
    }

    // =========================================================================
    // Iteration
    // =========================================================================

    /// Returns an iterator over `(&K, &V)` pairs in sorted order.
    #[inline]
    pub fn iter(&self) -> Iter<'_, K, V, B> {
        let mut it = Iter {
            stack: [(ptr::null_mut(), 0u16); MAX_DEPTH],
            stack_len: 0,
            remaining: self.len,
            _marker: PhantomData,
        };
        if !self.root.is_null() {
            push_leftmost_path(self.root, &mut it.stack, &mut it.stack_len);
        }
        it
    }

    /// Returns an iterator over keys in sorted order.
    #[inline]
    pub fn keys(&self) -> Keys<'_, K, V, B> {
        Keys { inner: self.iter() }
    }

    /// Returns an iterator over values in key-sorted order.
    #[inline]
    pub fn values(&self) -> Values<'_, K, V, B> {
        Values { inner: self.iter() }
    }

    /// Returns a mutable iterator over `(&K, &mut V)` pairs in sorted order.
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, K, V, B> {
        let mut it = IterMut {
            stack: [(ptr::null_mut(), 0u16); MAX_DEPTH],
            stack_len: 0,
            remaining: self.len,
            _marker: PhantomData,
        };
        if !self.root.is_null() {
            push_leftmost_path(self.root, &mut it.stack, &mut it.stack_len);
        }
        it
    }

    /// Returns a mutable iterator over values in key-sorted order.
    #[inline]
    pub fn values_mut(&mut self) -> ValuesMut<'_, K, V, B> {
        ValuesMut {
            inner: self.iter_mut(),
        }
    }

    /// Returns an iterator over `(&K, &V)` pairs within the given range.
    #[inline]
    pub fn range<R: std::ops::RangeBounds<K>>(&self, range: R) -> Range<'_, K, V, B> {
        use std::ops::Bound;

        let mut it = Range {
            stack: [(ptr::null_mut(), 0u16); MAX_DEPTH],
            stack_len: 0,
            end_node: ptr::null_mut(),
            end_idx: 0,
            _marker: PhantomData,
        };

        if self.root.is_null() {
            return it;
        }

        // Build start stack.
        match range.start_bound() {
            Bound::Unbounded => {
                push_leftmost_path(self.root, &mut it.stack, &mut it.stack_len);
            }
            Bound::Included(k) => {
                init_lower_bound_stack(self.root, k, &mut it.stack, &mut it.stack_len);
            }
            Bound::Excluded(k) => {
                init_upper_bound_stack(self.root, k, &mut it.stack, &mut it.stack_len);
            }
        }

        // Find end sentinel.
        match range.end_bound() {
            Bound::Unbounded => {} // end_node stays null
            Bound::Excluded(k) => {
                let (n, i) = self.lower_bound_pos(k);
                it.end_node = n;
                it.end_idx = i;
            }
            Bound::Included(k) => {
                let (n, i) = self.upper_bound_pos(k);
                it.end_node = n;
                it.end_idx = i;
            }
        }

        // Check if start is already at or past end.
        if it.stack_len > 0 && !it.end_node.is_null() {
            let (sn, si) = it.stack[it.stack_len - 1];
            if sn == it.end_node && si == it.end_idx {
                it.stack_len = 0;
            }
        }

        it
    }

    /// Returns a mutable iterator over `(&K, &mut V)` pairs within the given range.
    #[inline]
    pub fn range_mut<R: std::ops::RangeBounds<K>>(&mut self, range: R) -> RangeMut<'_, K, V, B> {
        use std::ops::Bound;

        let mut it = RangeMut {
            stack: [(ptr::null_mut(), 0u16); MAX_DEPTH],
            stack_len: 0,
            end_node: ptr::null_mut(),
            end_idx: 0,
            _marker: PhantomData,
        };

        if self.root.is_null() {
            return it;
        }

        match range.start_bound() {
            Bound::Unbounded => {
                push_leftmost_path(self.root, &mut it.stack, &mut it.stack_len);
            }
            Bound::Included(k) => {
                init_lower_bound_stack(self.root, k, &mut it.stack, &mut it.stack_len);
            }
            Bound::Excluded(k) => {
                init_upper_bound_stack(self.root, k, &mut it.stack, &mut it.stack_len);
            }
        }

        match range.end_bound() {
            Bound::Unbounded => {}
            Bound::Excluded(k) => {
                let (n, i) = self.lower_bound_pos(k);
                it.end_node = n;
                it.end_idx = i;
            }
            Bound::Included(k) => {
                let (n, i) = self.upper_bound_pos(k);
                it.end_node = n;
                it.end_idx = i;
            }
        }

        if it.stack_len > 0 && !it.end_node.is_null() {
            let (sn, si) = it.stack[it.stack_len - 1];
            if sn == it.end_node && si == it.end_idx {
                it.stack_len = 0;
            }
        }

        it
    }

    // =========================================================================
    // Cursor
    // =========================================================================

    /// Returns a cursor positioned before the first element.
    #[inline]
    pub fn cursor_front(&mut self) -> Cursor<'_, K, V, A, B> {
        Cursor {
            tree: self,
            stack: [(ptr::null_mut(), 0u16); MAX_DEPTH],
            stack_len: 0,
            started: false,
        }
    }

    /// Returns a cursor positioned at the given key, or at the first
    /// element greater than the key if not found.
    #[inline]
    pub fn cursor_at(&mut self, key: &K) -> Cursor<'_, K, V, A, B> {
        let mut cursor = Cursor {
            tree: self,
            stack: [(ptr::null_mut(), 0u16); MAX_DEPTH],
            stack_len: 0,
            started: true,
        };
        if !cursor.tree.root.is_null() {
            init_lower_bound_stack(
                cursor.tree.root,
                key,
                &mut cursor.stack,
                &mut cursor.stack_len,
            );
        }
        cursor
    }

    // =========================================================================
    // Drain
    // =========================================================================

    /// Returns a draining iterator that removes all key-value pairs in sorted order.
    #[inline]
    pub fn drain(&mut self) -> Drain<'_, K, V, A, B> {
        Drain { tree: self }
    }

    // =========================================================================
    // Internal: find
    // =========================================================================

    #[inline]
    fn find(&self, key: &K) -> Option<(NodePtr<K, V, B>, usize)> {
        let mut current = self.root;
        while !current.is_null() {
            // SAFETY: current is non-null and in the tree.
            let (idx, found) = unsafe { search_in_node(current, key) };
            if found {
                return Some((current, idx));
            }
            if unsafe { node_is_leaf(current) } {
                return None;
            }
            let next = unsafe { child_at(current, idx) };
            prefetch_read_node(next);
            current = next;
        }
        None
    }

    // =========================================================================
    // Internal: remove logic
    // =========================================================================

    /// Removes the key at `(node, idx)` given the descent `path`.
    ///
    /// # Safety
    ///
    /// `node` must contain a valid key at `idx`. `path` is the descent
    /// from root to `node`.
    #[inline]
    unsafe fn remove_found(
        &mut self,
        node: NodePtr<K, V, B>,
        idx: usize,
        path: &[(NodePtr<K, V, B>, usize); MAX_DEPTH],
        path_len: usize,
    ) -> (K, V) {
        if unsafe { node_is_leaf(node) } {
            // Leaf: read key/value, shift left, fixup.
            let result = unsafe { take_kv(node, idx) };
            unsafe { shift_left(node, idx) };
            self.fixup_after_remove(node, path, path_len);
            result
        } else {
            // Internal: swap with in-order predecessor, then remove from leaf.
            //
            // The predecessor is the rightmost key in child[idx].
            // Build extended path from node down to the predecessor's leaf.
            let mut ext_path = *path;
            let mut ext_len = path_len;

            debug_assert!(ext_len < MAX_DEPTH, "path overflow in remove_found");
            ext_path[ext_len] = (node, idx);
            ext_len += 1;

            let mut pred_node = unsafe { child_at(node, idx) };
            while !unsafe { node_is_leaf(pred_node) } {
                let plen = unsafe { node_len(pred_node) };
                debug_assert!(
                    ext_len < MAX_DEPTH,
                    "path overflow in remove_found predecessor"
                );
                ext_path[ext_len] = (pred_node, plen);
                ext_len += 1;
                pred_node = unsafe { child_at(pred_node, plen) };
            }

            let pred_idx = unsafe { node_len(pred_node) } - 1;

            // Read the target key/value from the internal node.
            let result = unsafe { take_kv(node, idx) };

            // Move predecessor key/value into the internal node's slot.
            let pred_k = unsafe { (*node_deref(pred_node)).keys[pred_idx].assume_init_read() };
            let pred_v = unsafe { (*node_deref(pred_node)).values[pred_idx].assume_init_read() };

            let int_node = unsafe { &mut *node_deref_mut(node) };
            int_node.keys[idx] = MaybeUninit::new(pred_k);
            int_node.values[idx] = MaybeUninit::new(pred_v);

            // Remove predecessor from its leaf (it's the last key, just decrement).
            unsafe { (*node_deref_mut(pred_node)).len -= 1 };

            self.fixup_after_remove(pred_node, &ext_path, ext_len);
            result
        }
    }

    /// Checks if `leaf_or_node` underflows after removal and rebalances.
    #[inline]
    fn fixup_after_remove(
        &mut self,
        mut node: NodePtr<K, V, B>,
        path: &[(NodePtr<K, V, B>, usize); MAX_DEPTH],
        path_len: usize,
    ) {
        let min = B / 2 - 1;
        let mut depth = path_len;

        loop {
            let len = unsafe { node_len(node) };

            // Check if root became empty.
            if node == self.root {
                if len == 0 {
                    if unsafe { node_is_leaf(node) } {
                        // Tree is now empty.
                        unsafe { free_node::<K, V, A, B>(node) };
                        self.root = ptr::null_mut();
                        self.depth = 0;
                    } else {
                        // Root has no keys but one child — shrink.
                        let new_root = unsafe { child_at(node, 0) };
                        // SAFETY: root has 0 keys, free_node drops nothing.
                        unsafe { free_node::<K, V, A, B>(node) };
                        self.root = new_root;
                        self.depth -= 1;
                    }
                }
                return;
            }

            if len >= min {
                return; // no underflow
            }

            // node underflows. Parent is path[depth - 1].
            let (parent, child_idx) = path[depth - 1];
            let parent_len = unsafe { node_len(parent) };

            // Try rotate from left sibling.
            if child_idx > 0 {
                let left = unsafe { child_at(parent, child_idx - 1) };
                if unsafe { node_len(left) } > min {
                    unsafe { Self::rotate_right(parent, child_idx) };
                    return;
                }
            }

            // Try rotate from right sibling.
            if child_idx < parent_len {
                let right = unsafe { child_at(parent, child_idx + 1) };
                if unsafe { node_len(right) } > min {
                    unsafe { Self::rotate_left(parent, child_idx) };
                    return;
                }
            }

            // Must merge.
            let merge_idx = if child_idx > 0 {
                child_idx - 1
            } else {
                child_idx
            };
            unsafe { Self::merge_children(parent, merge_idx) };

            // Continue up — parent may now underflow.
            node = parent;
            depth -= 1;
        }
    }

    /// Steals the last key from the left sibling via parent rotation.
    ///
    /// # Safety
    ///
    /// `parent` non-null, `child_idx > 0`, left sibling has > min keys.
    unsafe fn rotate_right(parent: NodePtr<K, V, B>, child_idx: usize) {
        let parent_node = unsafe { &mut *node_deref_mut(parent) };
        let child = parent_node.children[child_idx];
        let left = parent_node.children[child_idx - 1];
        let child_node = unsafe { &mut *node_deref_mut(child) };
        let left_node = unsafe { &mut *node_deref_mut(left) };
        let child_len = child_node.len as usize;
        let left_len = left_node.len as usize;
        let p_idx = child_idx - 1;

        // Shift everything in child right by 1.
        // SAFETY: keys/values [0..child_len) → [1..child_len+1).
        // Single as_mut_ptr() origin avoids Stacked Borrows conflicts.
        if child_len > 0 {
            unsafe {
                let kp = child_node.keys.as_mut_ptr();
                ptr::copy(kp.cast_const(), kp.add(1), child_len);
                let vp = child_node.values.as_mut_ptr();
                ptr::copy(vp.cast_const(), vp.add(1), child_len);
            }
        }
        if !child_node.leaf {
            // children[0..=child_len] → [1..=child_len+1]
            unsafe {
                let cp = child_node.children.as_mut_ptr();
                ptr::copy(cp.cast_const(), cp.add(1), child_len + 1);
            }
        }

        // Move parent separator down to child[0].
        child_node.keys[0] =
            MaybeUninit::new(unsafe { parent_node.keys[p_idx].assume_init_read() });
        child_node.values[0] =
            MaybeUninit::new(unsafe { parent_node.values[p_idx].assume_init_read() });

        // Move left sibling's last child to child[0] (if internal).
        if !child_node.leaf {
            child_node.children[0] = left_node.children[left_len];
        }

        // Move left sibling's last key up to parent.
        parent_node.keys[p_idx] =
            MaybeUninit::new(unsafe { left_node.keys[left_len - 1].assume_init_read() });
        parent_node.values[p_idx] =
            MaybeUninit::new(unsafe { left_node.values[left_len - 1].assume_init_read() });

        child_node.len += 1;
        left_node.len -= 1;
    }

    /// Steals the first key from the right sibling via parent rotation.
    ///
    /// # Safety
    ///
    /// `parent` non-null, `child_idx < parent.len`, right sibling has > min keys.
    unsafe fn rotate_left(parent: NodePtr<K, V, B>, child_idx: usize) {
        let parent_node = unsafe { &mut *node_deref_mut(parent) };
        let child = parent_node.children[child_idx];
        let right = parent_node.children[child_idx + 1];
        let child_node = unsafe { &mut *node_deref_mut(child) };
        let right_node = unsafe { &mut *node_deref_mut(right) };
        let child_len = child_node.len as usize;
        let right_len = right_node.len as usize;
        let p_idx = child_idx;

        // Append parent separator to end of child.
        child_node.keys[child_len] =
            MaybeUninit::new(unsafe { parent_node.keys[p_idx].assume_init_read() });
        child_node.values[child_len] =
            MaybeUninit::new(unsafe { parent_node.values[p_idx].assume_init_read() });

        // Move right's first child to child's new last child (if internal).
        if !child_node.leaf {
            child_node.children[child_len + 1] = right_node.children[0];
        }

        // Move right's first key up to parent.
        parent_node.keys[p_idx] =
            MaybeUninit::new(unsafe { right_node.keys[0].assume_init_read() });
        parent_node.values[p_idx] =
            MaybeUninit::new(unsafe { right_node.values[0].assume_init_read() });

        // Shift right's keys/values/children left by 1.
        // Single as_mut_ptr() origin avoids Stacked Borrows conflicts.
        if right_len > 1 {
            unsafe {
                let kp = right_node.keys.as_mut_ptr();
                ptr::copy(kp.add(1).cast_const(), kp, right_len - 1);
                let vp = right_node.values.as_mut_ptr();
                ptr::copy(vp.add(1).cast_const(), vp, right_len - 1);
            }
        }
        if !right_node.leaf {
            unsafe {
                let cp = right_node.children.as_mut_ptr();
                ptr::copy(cp.add(1).cast_const(), cp, right_len);
            }
        }

        child_node.len += 1;
        right_node.len -= 1;
    }

    /// Merges `children[merge_idx]` and `children[merge_idx+1]` with the
    /// separator key from parent.
    ///
    /// # Safety
    ///
    /// `parent` non-null, `merge_idx < parent.len`.
    unsafe fn merge_children(parent: NodePtr<K, V, B>, merge_idx: usize) {
        let parent_node = unsafe { &*node_deref(parent) };
        let left = parent_node.children[merge_idx];
        let right = parent_node.children[merge_idx + 1];
        let left_node = unsafe { &mut *node_deref_mut(left) };
        let right_node = unsafe { &*node_deref(right) };
        let left_len = left_node.len as usize;
        let right_len = right_node.len as usize;

        // Append separator key from parent to left.
        left_node.keys[left_len] =
            MaybeUninit::new(unsafe { (*node_deref(parent)).keys[merge_idx].assume_init_read() });
        left_node.values[left_len] =
            MaybeUninit::new(unsafe { (*node_deref(parent)).values[merge_idx].assume_init_read() });

        // Copy right's keys/values to left.
        if right_len > 0 {
            unsafe {
                ptr::copy_nonoverlapping(
                    right_node.keys.as_ptr(),
                    left_node.keys.as_mut_ptr().add(left_len + 1),
                    right_len,
                );
                ptr::copy_nonoverlapping(
                    right_node.values.as_ptr(),
                    left_node.values.as_mut_ptr().add(left_len + 1),
                    right_len,
                );
            }
        }

        // Copy right's children (if internal).
        if !left_node.leaf {
            unsafe {
                ptr::copy_nonoverlapping(
                    right_node.children.as_ptr(),
                    left_node.children.as_mut_ptr().add(left_len + 1),
                    right_len + 1,
                );
            }
        }

        left_node.len = (left_len + 1 + right_len) as u16;

        // SAFETY: All initialized keys, values, and children were moved to
        // `left` via ptr::copy_nonoverlapping above. The remaining slots are
        // MaybeUninit and won't run Drop. `right` is a valid slab pointer
        // obtained from A::try_alloc/alloc, so Slot::from_ptr is sound.
        let slot = unsafe { Slot::from_ptr(right) };
        unsafe { A::free(slot) };

        // Remove separator and right child pointer from parent.
        // SAFETY: merge_idx < parent.len.
        unsafe { shift_left(parent, merge_idx) };
    }

    /// Recursively frees all nodes in the subtree.
    ///
    /// # Safety
    ///
    /// `ptr` must be non-null and point to a valid tree node.
    unsafe fn clear_subtree(ptr: NodePtr<K, V, B>) {
        let node = unsafe { &*node_deref(ptr) };
        let len = node.len as usize;

        if !node.leaf {
            for i in 0..=len {
                let child = node.children[i];
                if !child.is_null() {
                    // SAFETY: child is a valid tree node.
                    unsafe { Self::clear_subtree(child) };
                }
            }
        }

        // SAFETY: ptr is valid and occupied.
        unsafe { free_node::<K, V, A, B>(ptr) };
    }

    // =========================================================================
    // Internal: range/iterator stack helpers
    // =========================================================================

    /// Position of first key >= `key`. Returns `(null, 0)` if none.
    fn lower_bound_pos(&self, key: &K) -> (NodePtr<K, V, B>, u16) {
        let mut result: (NodePtr<K, V, B>, u16) = (ptr::null_mut(), 0);
        let mut current = self.root;
        while !current.is_null() {
            // SAFETY: current is non-null and in the tree.
            let (idx, found) = unsafe { search_in_node(current, key) };
            if found {
                return (current, idx as u16);
            }
            if unsafe { node_is_leaf(current) } {
                if idx < unsafe { node_len(current) } {
                    return (current, idx as u16);
                }
                return result;
            }
            if idx < unsafe { node_len(current) } {
                result = (current, idx as u16);
            }
            current = unsafe { child_at(current, idx) };
        }
        result
    }

    /// Position of first key > `key`. Returns `(null, 0)` if none.
    fn upper_bound_pos(&self, key: &K) -> (NodePtr<K, V, B>, u16) {
        let mut result: (NodePtr<K, V, B>, u16) = (ptr::null_mut(), 0);
        let mut current = self.root;
        while !current.is_null() {
            // SAFETY: current is non-null and in the tree.
            let (idx, found) = unsafe { search_in_node(current, key) };
            if found {
                if unsafe { node_is_leaf(current) } {
                    if idx + 1 < unsafe { node_len(current) } {
                        return (current, (idx + 1) as u16);
                    }
                    return result;
                }
                // Internal: successor is leftmost in child[idx+1].
                let mut c = unsafe { child_at(current, idx + 1) };
                while !unsafe { node_is_leaf(c) } {
                    c = unsafe { child_at(c, 0) };
                }
                return (c, 0);
            }
            if unsafe { node_is_leaf(current) } {
                if idx < unsafe { node_len(current) } {
                    return (current, idx as u16);
                }
                return result;
            }
            if idx < unsafe { node_len(current) } {
                result = (current, idx as u16);
            }
            current = unsafe { child_at(current, idx) };
        }
        result
    }

    // =========================================================================
    // Invariant verification (for testing)
    // =========================================================================

    /// Verifies all B-tree invariants. Panics on violation.
    #[doc(hidden)]
    pub fn verify_invariants(&self) {
        if self.root.is_null() {
            assert_eq!(self.len, 0, "len must be 0 when root is null");
            assert_eq!(self.depth, 0, "depth must be 0 when root is null");
            return;
        }

        let min = B / 2 - 1;
        let mut leaf_depth: Option<usize> = None;
        let mut count = 0usize;

        Self::verify_subtree(
            self.root,
            true,
            min,
            0,
            &mut leaf_depth,
            &mut count,
            None,
            None,
        );

        assert_eq!(count, self.len, "key count ({count}) != len ({})", self.len);

        if let Some(ld) = leaf_depth {
            assert_eq!(
                ld, self.depth,
                "actual leaf depth ({ld}) != cached depth ({})",
                self.depth
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn verify_subtree(
        ptr: NodePtr<K, V, B>,
        is_root: bool,
        min: usize,
        depth: usize,
        leaf_depth: &mut Option<usize>,
        count: &mut usize,
        lower: Option<&K>,
        upper: Option<&K>,
    ) {
        let node = unsafe { &*node_deref(ptr) };
        let len = node.len as usize;

        // Key count bounds.
        assert!(len < B, "node has {len} keys, max is {}", B - 1);
        if !is_root {
            assert!(len >= min, "non-root node has {len} keys, min is {min}");
        }
        // Root of an internal tree must have at least 1 key.
        assert!(
            !(is_root && !node.leaf && len == 0),
            "internal root has 0 keys"
        );

        // Keys sorted within node.
        for i in 1..len {
            let prev = unsafe { node.keys[i - 1].assume_init_ref() };
            let curr = unsafe { node.keys[i].assume_init_ref() };
            assert!(
                prev < curr,
                "keys not sorted at indices {} and {}",
                i - 1,
                i
            );
        }

        // BST ordering with bounds from parent.
        if let Some(lo) = lower {
            let first = unsafe { node.keys[0].assume_init_ref() };
            assert!(first > lo, "key violates lower bound");
        }
        if let Some(hi) = upper {
            let last = unsafe { node.keys[len - 1].assume_init_ref() };
            assert!(last < hi, "key violates upper bound");
        }

        *count += len;

        if node.leaf {
            // Leaf depth consistency.
            match *leaf_depth {
                None => *leaf_depth = Some(depth),
                Some(expected) => {
                    assert_eq!(
                        depth, expected,
                        "leaf at depth {depth}, expected {expected}"
                    );
                }
            }
            // Leaves should have null children.
            for i in 0..B {
                assert!(node.children[i].is_null(), "leaf has non-null child at {i}");
            }
        } else {
            // Internal node: must have len+1 non-null children.
            for i in 0..=len {
                assert!(
                    !node.children[i].is_null(),
                    "internal node missing child at {i}"
                );
            }

            // Recurse into children with appropriate bounds.
            for i in 0..=len {
                let lo = if i > 0 {
                    Some(unsafe { node.keys[i - 1].assume_init_ref() })
                } else {
                    lower
                };
                let hi = if i < len {
                    Some(unsafe { node.keys[i].assume_init_ref() })
                } else {
                    upper
                };
                Self::verify_subtree(
                    node.children[i],
                    false,
                    min,
                    depth + 1,
                    leaf_depth,
                    count,
                    lo,
                    hi,
                );
            }
        }
    }
}

// =============================================================================
// impl<A: BoundedAlloc> — try_insert
// =============================================================================

impl<K: Ord, V, A: BoundedAlloc<Item = BTreeNode<K, V, B>>, const B: usize> BTree<K, V, A, B> {
    /// Inserts a key-value pair, or returns the pair if the allocator is full.
    ///
    /// If a node with the same key already exists, the value is replaced
    /// and the old value is returned inside `Ok(Some(old_value))`.
    #[inline]
    pub fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, Full<(K, V)>> {
        let (_, old) = self.try_insert_inner(key, value)?;
        Ok(old)
    }

    /// Inserts and returns `(pointer to value, Option<old_value>)`.
    /// Used by both `try_insert` and `VacantEntry::try_insert`.
    #[allow(clippy::type_complexity)]
    fn try_insert_inner(&mut self, key: K, value: V) -> Result<(*mut V, Option<V>), Full<(K, V)>> {
        // Empty tree: allocate root leaf.
        if self.root.is_null() {
            let mut leaf = BTreeNode::new_leaf();
            leaf.keys[0] = MaybeUninit::new(key);
            leaf.values[0] = MaybeUninit::new(value);
            leaf.len = 1;
            match A::try_alloc(leaf) {
                Ok(slot) => {
                    let ptr = slot.as_ptr();
                    self.root = ptr;
                    self.len += 1;
                    self.depth = 0;
                    let val_ptr = unsafe { (*node_deref_mut(ptr)).values[0].as_mut_ptr() };
                    return Ok((val_ptr, None));
                }
                Err(full) => {
                    let node = full.into_inner();
                    // SAFETY: keys[0] and values[0] were just initialized.
                    let k = unsafe { node.keys[0].assume_init_read() };
                    let v = unsafe { node.values[0].assume_init_read() };
                    return Err(Full((k, v)));
                }
            }
        }

        // If root is full, split it.
        if unsafe { node_len(self.root) } == B - 1 {
            let new_root = match A::try_alloc(BTreeNode::new_internal()) {
                Ok(slot) => slot.as_ptr(),
                Err(_) => return Err(Full((key, value))),
            };
            unsafe { (*node_deref_mut(new_root)).children[0] = self.root };
            let old_root = self.root;
            self.root = new_root;

            // Split old root as child[0] of new root.
            let right_node = if unsafe { node_is_leaf(old_root) } {
                BTreeNode::new_leaf()
            } else {
                BTreeNode::new_internal()
            };
            let right = if let Ok(slot) = A::try_alloc(right_node) {
                slot.as_ptr()
            } else {
                // Undo: restore old root, free new root.
                self.root = old_root;
                let slot = unsafe { Slot::from_ptr(new_root) };
                unsafe { A::free(slot) };
                return Err(Full((key, value)));
            };
            unsafe { split_child_core(new_root, 0, right) };
            self.depth += 1;
        }

        // Descend with preemptive splitting.
        let mut current = self.root;
        loop {
            // SAFETY: current is non-null and in the tree.
            let (idx, found) = unsafe { search_in_node(current, &key) };

            if found {
                // Duplicate key: replace value in place.
                let existing = unsafe { value_at_mut(current, idx) };
                let old = std::mem::replace(existing, value);
                let val_ptr = existing as *mut V;
                return Ok((val_ptr, Some(old)));
            }

            if unsafe { node_is_leaf(current) } {
                // Insert into leaf.
                unsafe { shift_right(current, idx) };
                let node = unsafe { &mut *node_deref_mut(current) };
                node.keys[idx] = MaybeUninit::new(key);
                node.values[idx] = MaybeUninit::new(value);
                node.len += 1;
                self.len += 1;
                let val_ptr = node.values[idx].as_mut_ptr();
                return Ok((val_ptr, None));
            }

            // Internal: check if child is full, split if needed.
            let mut child_idx = idx;
            let child = unsafe { child_at(current, child_idx) };
            if unsafe { node_len(child) } == B - 1 {
                // Allocate right sibling for split.
                let right = match A::try_alloc(if unsafe { node_is_leaf(child) } {
                    BTreeNode::new_leaf()
                } else {
                    BTreeNode::new_internal()
                }) {
                    Ok(slot) => slot.as_ptr(),
                    Err(_) => return Err(Full((key, value))),
                };
                unsafe { split_child_core(current, child_idx, right) };

                // After split, decide which child to descend into.
                let median = unsafe { key_at(current, child_idx) };
                if key == *median {
                    // Duplicate: replace median value.
                    let existing = unsafe { value_at_mut(current, child_idx) };
                    let old = std::mem::replace(existing, value);
                    let val_ptr = existing as *mut V;
                    return Ok((val_ptr, Some(old)));
                }
                if key > *median {
                    child_idx += 1;
                }
            }

            current = unsafe { child_at(current, child_idx) };
        }
    }
}

// =============================================================================
// impl<A: UnboundedAlloc> — insert
// =============================================================================

impl<K: Ord, V, A: UnboundedAlloc<Item = BTreeNode<K, V, B>>, const B: usize> BTree<K, V, A, B> {
    /// Inserts a key-value pair. Always succeeds (grows as needed).
    #[inline]
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let (_, old) = self.insert_inner(key, value);
        old
    }

    fn insert_inner(&mut self, key: K, value: V) -> (*mut V, Option<V>) {
        if self.root.is_null() {
            let mut leaf = BTreeNode::new_leaf();
            leaf.keys[0] = MaybeUninit::new(key);
            leaf.values[0] = MaybeUninit::new(value);
            leaf.len = 1;
            let slot = A::alloc(leaf);
            let ptr = slot.as_ptr();
            self.root = ptr;
            self.len += 1;
            self.depth = 0;
            let val_ptr = unsafe { (*node_deref_mut(ptr)).values[0].as_mut_ptr() };
            return (val_ptr, None);
        }

        // Split root if full.
        if unsafe { node_len(self.root) } == B - 1 {
            let new_root = A::alloc(BTreeNode::new_internal()).as_ptr();
            unsafe { (*node_deref_mut(new_root)).children[0] = self.root };
            let old_root = self.root;
            self.root = new_root;

            let right = A::alloc(if unsafe { node_is_leaf(old_root) } {
                BTreeNode::new_leaf()
            } else {
                BTreeNode::new_internal()
            })
            .as_ptr();
            unsafe { split_child_core(new_root, 0, right) };
            self.depth += 1;
        }

        let mut current = self.root;
        loop {
            let (idx, found) = unsafe { search_in_node(current, &key) };

            if found {
                let existing = unsafe { value_at_mut(current, idx) };
                let old = std::mem::replace(existing, value);
                let val_ptr = existing as *mut V;
                return (val_ptr, Some(old));
            }

            if unsafe { node_is_leaf(current) } {
                unsafe { shift_right(current, idx) };
                let node = unsafe { &mut *node_deref_mut(current) };
                node.keys[idx] = MaybeUninit::new(key);
                node.values[idx] = MaybeUninit::new(value);
                node.len += 1;
                self.len += 1;
                let val_ptr = node.values[idx].as_mut_ptr();
                return (val_ptr, None);
            }

            let mut child_idx = idx;
            let child = unsafe { child_at(current, child_idx) };
            if unsafe { node_len(child) } == B - 1 {
                let right = A::alloc(if unsafe { node_is_leaf(child) } {
                    BTreeNode::new_leaf()
                } else {
                    BTreeNode::new_internal()
                })
                .as_ptr();
                unsafe { split_child_core(current, child_idx, right) };

                let median = unsafe { key_at(current, child_idx) };
                if key == *median {
                    let existing = unsafe { value_at_mut(current, child_idx) };
                    let old = std::mem::replace(existing, value);
                    let val_ptr = existing as *mut V;
                    return (val_ptr, Some(old));
                }
                if key > *median {
                    child_idx += 1;
                }
            }

            current = unsafe { child_at(current, child_idx) };
        }
    }
}

// =============================================================================
// Drop
// =============================================================================

impl<K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> Drop for BTree<K, V, A, B> {
    fn drop(&mut self) {
        self.clear();
    }
}

impl<K: Ord + fmt::Debug, V: fmt::Debug, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize>
    fmt::Debug for BTree<K, V, A, B>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

// =============================================================================
// Entry API
// =============================================================================

/// A view into a single entry in the tree.
pub enum Entry<'a, K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> {
    /// An occupied entry — key exists.
    Occupied(OccupiedEntry<'a, K, V, A, B>),
    /// A vacant entry — key does not exist.
    Vacant(VacantEntry<'a, K, V, A, B>),
}

/// A view into an occupied entry.
pub struct OccupiedEntry<'a, K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> {
    tree: &'a mut BTree<K, V, A, B>,
    node: NodePtr<K, V, B>,
    idx: usize,
}

/// A view into a vacant entry.
///
/// **Note:** Insertion via `try_insert`/`insert` performs a second traversal
/// from the root because the preemptive-split insertion algorithm must split
/// full nodes on descent. The `entry()` lookup traversal cannot be reused.
/// For insert-heavy workloads where the key is usually absent, prefer
/// `try_insert`/`insert` directly to avoid the redundant first traversal.
pub struct VacantEntry<'a, K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> {
    tree: &'a mut BTree<K, V, A, B>,
    key: K,
}

// -- Entry: base Alloc methods --

impl<K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> Entry<'_, K, V, A, B> {
    /// Returns a reference to this entry's key.
    #[inline]
    pub fn key(&self) -> &K {
        match self {
            Entry::Occupied(e) => e.key(),
            Entry::Vacant(e) => e.key(),
        }
    }

    /// Modifies an existing entry before potential insertion.
    #[inline]
    pub fn and_modify<F: FnOnce(&mut V)>(mut self, f: F) -> Self {
        if let Entry::Occupied(ref mut e) = self {
            f(e.get_mut());
        }
        self
    }
}

// -- Entry: BoundedAlloc methods --

impl<'a, K: Ord, V, A: BoundedAlloc<Item = BTreeNode<K, V, B>>, const B: usize>
    Entry<'a, K, V, A, B>
{
    /// Ensures a value is in the entry by inserting if vacant (bounded).
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

impl<'a, K: Ord, V, A: UnboundedAlloc<Item = BTreeNode<K, V, B>>, const B: usize>
    Entry<'a, K, V, A, B>
{
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

impl<'a, K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize>
    OccupiedEntry<'a, K, V, A, B>
{
    /// Returns a reference to the key.
    #[inline]
    pub fn key(&self) -> &K {
        // SAFETY: node/idx are valid.
        unsafe { key_at(self.node, self.idx) }
    }

    /// Returns a reference to the value.
    #[inline]
    pub fn get(&self) -> &V {
        // SAFETY: node/idx are valid.
        unsafe { value_at(self.node, self.idx) }
    }

    /// Returns a mutable reference to the value.
    #[inline]
    pub fn get_mut(&mut self) -> &mut V {
        // SAFETY: node/idx are valid; &mut self prevents aliasing.
        unsafe { value_at_mut(self.node, self.idx) }
    }

    /// Converts to a mutable reference with the entry's lifetime.
    #[inline]
    pub fn into_mut(self) -> &'a mut V {
        // SAFETY: node/idx are valid.
        unsafe { value_at_mut(self.node, self.idx) }
    }

    /// Sets the value of the entry and returns the old value.
    #[inline]
    pub fn insert(&mut self, value: V) -> V {
        // SAFETY: node/idx are valid; &mut self prevents aliasing.
        let slot = unsafe { &mut (*node_deref_mut(self.node)).values[self.idx] };
        let old = unsafe { slot.assume_init_read() };
        *slot = MaybeUninit::new(value);
        old
    }

    /// Removes the entry and returns `(key, value)`.
    #[inline]
    pub fn remove(self) -> (K, V) {
        // SAFETY: node/idx valid. Bitwise-copy the key to the stack so we
        // search without holding a reference into the node (which gets mutated
        // during removal). ManuallyDrop prevents double-drop — remove_entry
        // returns the authoritative owned copy.
        let key_copy = std::mem::ManuallyDrop::new(unsafe {
            (*node_deref(self.node)).keys[self.idx].assume_init_read()
        });
        self.tree
            .remove_entry(&key_copy)
            .expect("occupied entry must exist")
    }
}

// -- VacantEntry: BoundedAlloc --

impl<'a, K: Ord, V, A: BoundedAlloc<Item = BTreeNode<K, V, B>>, const B: usize>
    VacantEntry<'a, K, V, A, B>
{
    /// Inserts a value into the vacant entry (bounded).
    #[inline]
    pub fn try_insert(self, value: V) -> Result<&'a mut V, Full<(K, V)>> {
        let VacantEntry { tree, key } = self;
        let (val_ptr, _) = tree.try_insert_inner(key, value)?;
        // SAFETY: val_ptr points to slab memory owned by the tree.
        Ok(unsafe { &mut *val_ptr })
    }
}

// -- VacantEntry: UnboundedAlloc --

impl<'a, K: Ord, V, A: UnboundedAlloc<Item = BTreeNode<K, V, B>>, const B: usize>
    VacantEntry<'a, K, V, A, B>
{
    /// Inserts a value into the vacant entry (unbounded).
    #[inline]
    pub fn insert(self, value: V) -> &'a mut V {
        let VacantEntry { tree, key } = self;
        let (val_ptr, _) = tree.insert_inner(key, value);
        // SAFETY: val_ptr points to slab memory owned by the tree.
        unsafe { &mut *val_ptr }
    }
}

// -- VacantEntry: base Alloc --

impl<K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> VacantEntry<'_, K, V, A, B> {
    /// Returns a reference to the key that would be used for insertion.
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }
}

// =============================================================================
// Iterator stack helpers (free functions)
// =============================================================================

/// Pushes the leftmost path from `node` onto the stack.
fn push_leftmost_path<K, V, const B: usize>(
    mut node: NodePtr<K, V, B>,
    stack: &mut [(NodePtr<K, V, B>, u16); MAX_DEPTH],
    stack_len: &mut usize,
) {
    loop {
        debug_assert!(
            *stack_len < MAX_DEPTH,
            "stack overflow in push_leftmost_path"
        );
        stack[*stack_len] = (node, 0);
        *stack_len += 1;
        // SAFETY: node is non-null and in the tree.
        if unsafe { node_is_leaf(node) } {
            return;
        }
        node = unsafe { child_at(node, 0) };
    }
}

/// Initializes the stack to the first key >= `key` (lower bound).
fn init_lower_bound_stack<K: Ord, V, const B: usize>(
    root: NodePtr<K, V, B>,
    key: &K,
    stack: &mut [(NodePtr<K, V, B>, u16); MAX_DEPTH],
    stack_len: &mut usize,
) {
    let mut current = root;
    while !current.is_null() {
        // SAFETY: current is non-null and in the tree.
        let (idx, found) = unsafe { search_in_node(current, key) };
        if found {
            // Exact match: start yielding from this position.
            // The first key >= target IS key[idx] itself (child[idx]
            // contains only keys < key[idx], all below the bound).
            stack[*stack_len] = (current, idx as u16);
            *stack_len += 1;
            return;
        }
        if unsafe { node_is_leaf(current) } {
            if idx < unsafe { node_len(current) } {
                stack[*stack_len] = (current, idx as u16);
                *stack_len += 1;
            }
            // else: all keys < target, answer is in an ancestor already on stack.
            return;
        }
        // Internal: key belongs in child[idx]. key[idx] (if exists) is > key.
        if idx < unsafe { node_len(current) } {
            stack[*stack_len] = (current, idx as u16);
            *stack_len += 1;
        }
        current = unsafe { child_at(current, idx) };
    }
}

/// Initializes the stack to the first key > `key` (upper bound / excluded).
fn init_upper_bound_stack<K: Ord, V, const B: usize>(
    root: NodePtr<K, V, B>,
    key: &K,
    stack: &mut [(NodePtr<K, V, B>, u16); MAX_DEPTH],
    stack_len: &mut usize,
) {
    let mut current = root;
    while !current.is_null() {
        // SAFETY: current is non-null and in the tree.
        let (idx, found) = unsafe { search_in_node(current, key) };
        if found {
            // Want first key > key. Skip key[idx].
            if unsafe { node_is_leaf(current) } {
                if idx + 1 < unsafe { node_len(current) } {
                    stack[*stack_len] = (current, (idx + 1) as u16);
                    *stack_len += 1;
                }
                // else: answer is in ancestor already on stack.
            } else {
                // Internal: successor is in child[idx+1]'s leftmost.
                stack[*stack_len] = (current, (idx + 1) as u16);
                *stack_len += 1;
                let child = unsafe { child_at(current, idx + 1) };
                push_leftmost_path(child, stack, stack_len);
            }
            return;
        }
        // Not found — same as lower_bound (first key >= target is first key > key).
        if unsafe { node_is_leaf(current) } {
            if idx < unsafe { node_len(current) } {
                stack[*stack_len] = (current, idx as u16);
                *stack_len += 1;
            }
            return;
        }
        if idx < unsafe { node_len(current) } {
            stack[*stack_len] = (current, idx as u16);
            *stack_len += 1;
        }
        current = unsafe { child_at(current, idx) };
    }
}

/// Advances the iterator stack to the next position. Returns the current
/// `(NodePtr, key_index)` before advancing, or `None` if exhausted.
fn advance_stack<K, V, const B: usize>(
    stack: &mut [(NodePtr<K, V, B>, u16); MAX_DEPTH],
    stack_len: &mut usize,
) -> Option<(NodePtr<K, V, B>, usize)> {
    // Skip exhausted nodes.
    while *stack_len > 0 {
        let (node, idx) = stack[*stack_len - 1];
        if (idx as usize) < unsafe { node_len(node) } {
            break;
        }
        *stack_len -= 1;
    }

    if *stack_len == 0 {
        return None;
    }

    let (node, idx) = stack[*stack_len - 1];
    let i = idx as usize;

    stack[*stack_len - 1].1 = (i + 1) as u16;
    if !unsafe { node_is_leaf(node) } {
        // Internal: push leftmost of child[i+1].
        let child = unsafe { child_at(node, i + 1) };
        push_leftmost_path(child, stack, stack_len);
    }

    Some((node, i))
}

/// Like `advance_stack` but also checks the end sentinel.
fn advance_stack_range<K, V, const B: usize>(
    stack: &mut [(NodePtr<K, V, B>, u16); MAX_DEPTH],
    stack_len: &mut usize,
    end_node: NodePtr<K, V, B>,
    end_idx: u16,
) -> Option<(NodePtr<K, V, B>, usize)> {
    // Skip exhausted nodes.
    while *stack_len > 0 {
        let (node, idx) = stack[*stack_len - 1];
        if (idx as usize) < unsafe { node_len(node) } {
            break;
        }
        *stack_len -= 1;
    }

    if *stack_len == 0 {
        return None;
    }

    let (node, idx) = stack[*stack_len - 1];

    // Check end sentinel.
    if !end_node.is_null() && node == end_node && idx == end_idx {
        *stack_len = 0;
        return None;
    }

    let i = idx as usize;

    stack[*stack_len - 1].1 = (i + 1) as u16;
    if !unsafe { node_is_leaf(node) } {
        let child = unsafe { child_at(node, i + 1) };
        push_leftmost_path(child, stack, stack_len);
    }

    Some((node, i))
}

// =============================================================================
// Iter
// =============================================================================

/// Iterator over `(&K, &V)` pairs in sorted order.
pub struct Iter<'a, K, V, const B: usize> {
    stack: [(NodePtr<K, V, B>, u16); MAX_DEPTH],
    stack_len: usize,
    remaining: usize,
    _marker: PhantomData<&'a ()>,
}

impl<'a, K: 'a, V: 'a, const B: usize> Iterator for Iter<'a, K, V, B> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (node, idx) = advance_stack(&mut self.stack, &mut self.stack_len)?;
        self.remaining -= 1;
        // SAFETY: node is valid and idx < len.
        Some(unsafe { (key_at(node, idx), value_at(node, idx)) })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<'a, K: 'a, V: 'a, const B: usize> ExactSizeIterator for Iter<'a, K, V, B> {}

impl<'a, K: Ord + 'a, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> IntoIterator
    for &'a BTree<K, V, A, B>
{
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V, B>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, K: Ord + 'a, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> IntoIterator
    for &'a mut BTree<K, V, A, B>
{
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V, B>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

// =============================================================================
// Keys / Values
// =============================================================================

/// Iterator over keys in sorted order.
pub struct Keys<'a, K, V, const B: usize> {
    inner: Iter<'a, K, V, B>,
}

impl<'a, K: 'a, V: 'a, const B: usize> Iterator for Keys<'a, K, V, B> {
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

impl<'a, K: 'a, V: 'a, const B: usize> ExactSizeIterator for Keys<'a, K, V, B> {}

/// Iterator over values in key-sorted order.
pub struct Values<'a, K, V, const B: usize> {
    inner: Iter<'a, K, V, B>,
}

impl<'a, K: 'a, V: 'a, const B: usize> Iterator for Values<'a, K, V, B> {
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

impl<'a, K: 'a, V: 'a, const B: usize> ExactSizeIterator for Values<'a, K, V, B> {}

// =============================================================================
// IterMut
// =============================================================================

/// Mutable iterator over `(&K, &mut V)` pairs in sorted order.
pub struct IterMut<'a, K, V, const B: usize> {
    stack: [(NodePtr<K, V, B>, u16); MAX_DEPTH],
    stack_len: usize,
    remaining: usize,
    _marker: PhantomData<&'a mut ()>,
}

impl<'a, K: 'a, V: 'a, const B: usize> Iterator for IterMut<'a, K, V, B> {
    type Item = (&'a K, &'a mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (node, idx) = advance_stack(&mut self.stack, &mut self.stack_len)?;
        self.remaining -= 1;
        // SAFETY: node is valid, idx < len, &mut prevents aliasing.
        Some(unsafe { (key_at(node, idx), value_at_mut(node, idx)) })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<'a, K: 'a, V: 'a, const B: usize> ExactSizeIterator for IterMut<'a, K, V, B> {}

// =============================================================================
// ValuesMut
// =============================================================================

/// Mutable iterator over values in key-sorted order.
pub struct ValuesMut<'a, K, V, const B: usize> {
    inner: IterMut<'a, K, V, B>,
}

impl<'a, K: 'a, V: 'a, const B: usize> Iterator for ValuesMut<'a, K, V, B> {
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

impl<'a, K: 'a, V: 'a, const B: usize> ExactSizeIterator for ValuesMut<'a, K, V, B> {}

// =============================================================================
// Range
// =============================================================================

/// Iterator over `(&K, &V)` pairs within a key range.
pub struct Range<'a, K, V, const B: usize> {
    stack: [(NodePtr<K, V, B>, u16); MAX_DEPTH],
    stack_len: usize,
    end_node: NodePtr<K, V, B>,
    end_idx: u16,
    _marker: PhantomData<&'a ()>,
}

impl<'a, K: 'a, V: 'a, const B: usize> Iterator for Range<'a, K, V, B> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (node, idx) = advance_stack_range(
            &mut self.stack,
            &mut self.stack_len,
            self.end_node,
            self.end_idx,
        )?;
        // SAFETY: node is valid, idx < len.
        Some(unsafe { (key_at(node, idx), value_at(node, idx)) })
    }
}

// =============================================================================
// RangeMut
// =============================================================================

/// Mutable iterator over `(&K, &mut V)` pairs within a key range.
pub struct RangeMut<'a, K, V, const B: usize> {
    stack: [(NodePtr<K, V, B>, u16); MAX_DEPTH],
    stack_len: usize,
    end_node: NodePtr<K, V, B>,
    end_idx: u16,
    _marker: PhantomData<&'a mut ()>,
}

impl<'a, K: 'a, V: 'a, const B: usize> Iterator for RangeMut<'a, K, V, B> {
    type Item = (&'a K, &'a mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let (node, idx) = advance_stack_range(
            &mut self.stack,
            &mut self.stack_len,
            self.end_node,
            self.end_idx,
        )?;
        // SAFETY: node is valid, idx < len, &mut prevents aliasing.
        Some(unsafe { (key_at(node, idx), value_at_mut(node, idx)) })
    }
}

// =============================================================================
// Cursor
// =============================================================================

/// Cursor for positional traversal with removal.
///
/// After `remove()`, the cursor repositions to the successor.
pub struct Cursor<'a, K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> {
    tree: &'a mut BTree<K, V, A, B>,
    stack: [(NodePtr<K, V, B>, u16); MAX_DEPTH],
    stack_len: usize,
    started: bool,
}

impl<K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> Cursor<'_, K, V, A, B> {
    /// Returns a reference to the current key.
    #[inline]
    pub fn key(&self) -> Option<&K> {
        if self.stack_len == 0 || !self.started {
            return None;
        }
        let (node, idx) = self.stack[self.stack_len - 1];
        if (idx as usize) >= unsafe { node_len(node) } {
            return None;
        }
        Some(unsafe { key_at(node, idx as usize) })
    }

    /// Returns a reference to the current value.
    #[inline]
    pub fn value(&self) -> Option<&V> {
        if self.stack_len == 0 || !self.started {
            return None;
        }
        let (node, idx) = self.stack[self.stack_len - 1];
        if (idx as usize) >= unsafe { node_len(node) } {
            return None;
        }
        Some(unsafe { value_at(node, idx as usize) })
    }

    /// Returns a mutable reference to the current value.
    #[inline]
    pub fn value_mut(&mut self) -> Option<&mut V> {
        if self.stack_len == 0 || !self.started {
            return None;
        }
        let (node, idx) = self.stack[self.stack_len - 1];
        if (idx as usize) >= unsafe { node_len(node) } {
            return None;
        }
        Some(unsafe { value_at_mut(node, idx as usize) })
    }

    /// Advances the cursor to the next element. Returns `true` if positioned.
    #[inline]
    pub fn advance(&mut self) -> bool {
        if self.started {
            advance_stack(&mut self.stack, &mut self.stack_len);
        } else {
            self.started = true;
            if !self.tree.root.is_null() {
                push_leftmost_path(self.tree.root, &mut self.stack, &mut self.stack_len);
            }
        }
        self.key().is_some()
    }

    /// Removes the current element and advances to the successor.
    #[inline]
    pub fn remove(&mut self) -> Option<(K, V)> {
        if self.stack_len == 0 || !self.started {
            return None;
        }
        let (node, idx) = self.stack[self.stack_len - 1];
        let i = idx as usize;
        if i >= unsafe { node_len(node) } {
            return None;
        }

        // SAFETY: node/idx valid. Bitwise-copy the key to the stack so we
        // search without holding a reference into the node (which gets mutated
        // during removal). ManuallyDrop prevents double-drop.
        let key_copy =
            std::mem::ManuallyDrop::new(unsafe { (*node_deref(node)).keys[i].assume_init_read() });
        let result = self.tree.remove_entry(&key_copy);

        // Reposition to successor via upper_bound on the removed key.
        self.stack_len = 0;
        if let Some((ref removed_key, _)) = result {
            if !self.tree.is_empty() {
                init_upper_bound_stack(
                    self.tree.root,
                    removed_key,
                    &mut self.stack,
                    &mut self.stack_len,
                );
            }
        }

        result
    }
}

// =============================================================================
// Drain
// =============================================================================

/// Draining iterator that removes and returns all key-value pairs in sorted order.
pub struct Drain<'a, K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> {
    tree: &'a mut BTree<K, V, A, B>,
}

impl<K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> Iterator
    for Drain<'_, K, V, A, B>
{
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

impl<K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> ExactSizeIterator
    for Drain<'_, K, V, A, B>
{
}

impl<K: Ord, V, A: Alloc<Item = BTreeNode<K, V, B>>, const B: usize> Drop
    for Drain<'_, K, V, A, B>
{
    fn drop(&mut self) {
        self.tree.clear();
    }
}
