//! Skip list sorted map with internal slab allocation.
//!
//! # Design
//!
//! A probabilistic sorted map providing O(log n) expected time for insert,
//! lookup, and removal. Predictable latency (no rebalancing) makes it
//! well-suited for order book price levels.
//!
//! # Allocation Model
//!
//! The skip list takes a ZST allocator at construction time and handles all
//! node allocation and deallocation internally. The user only sees keys and
//! values — no `BoxSlot` or `Slot` in the public API.
//!
//! - Bounded allocators: `try_insert` returns `Err(Full((K, V)))` when full
//! - Unbounded allocators: `insert` always succeeds (grows as needed)
//!
//! ```text
//! Level 3:  HEAD ─────────────────────► 50 ──────────────────► NIL
//!             │                          │
//! Level 2:  HEAD ────────► 20 ──────────► 50 ──────────────────► NIL
//!             │            │              │
//! Level 1:  HEAD ──► 10 ──► 20 ──► 30 ──► 50 ──► 60 ──► NIL
//! ```
//!
//! # Example
//!
//! ```ignore
//! mod levels {
//!     nexus_collections::skip_allocator!(u64, String, bounded);
//! }
//!
//! levels::Allocator::builder().capacity(1000).build().unwrap();
//!
//! let mut map = levels::SkipList::new(levels::Allocator);
//! map.try_insert(100, "hello".into()).unwrap();
//!
//! assert_eq!(map.get(&100), Some(&"hello".into()));
//! ```

use std::cell::Cell;
use std::marker::PhantomData;
use std::ptr;

use nexus_slab::{Alloc, BoundedAlloc, Full, Slot, SlotCell, UnboundedAlloc};

// =============================================================================
// NodePtr
// =============================================================================

/// Raw pointer to a slab-allocated skip list node.
type NodePtr<K, V, const MAX_LEVEL: usize> = *mut SlotCell<SkipNode<K, V, MAX_LEVEL>>;

// =============================================================================
// SkipNode<K, V, MAX_LEVEL>
// =============================================================================

/// A node in a skip list sorted map.
///
/// Key-first layout keeps the hot search data in the first cache line:
/// for `K=u64` at `MAX_LEVEL=8`, key(8) + forward\[0..7\](64) = 72 bytes
/// spans 2 lines; at `MAX_LEVEL=4`, key(8) + forward\[0..3\](32) = 40 bytes
/// fits in one line. Value sits beyond the search path.
///
/// Node size: `sizeof(K) + MAX_LEVEL*8 + 8 + 8 + sizeof(V)` bytes
/// (for `K=u64`, `MAX_LEVEL=8` → 96 + sizeof(V)).
#[repr(C)]
pub struct SkipNode<K, V, const MAX_LEVEL: usize> {
    key: K,
    forward: [Cell<NodePtr<K, V, MAX_LEVEL>>; MAX_LEVEL],
    back: Cell<NodePtr<K, V, MAX_LEVEL>>,
    level: Cell<u8>,
    value: V,
}

impl<K, V, const MAX_LEVEL: usize> SkipNode<K, V, MAX_LEVEL> {
    /// Creates a new detached node with the given key and value.
    ///
    /// All link pointers are null. Level is 0 (assigned at insert time).
    #[inline]
    pub fn new(key: K, value: V) -> Self {
        SkipNode {
            key,
            forward: std::array::from_fn(|_| Cell::new(ptr::null_mut())),
            back: Cell::new(ptr::null_mut()),
            level: Cell::new(0),
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
// node_deref — navigate raw pointer to SkipNode
// =============================================================================

/// Dereferences a `NodePtr<K, V, ML>` to `*const SkipNode<K, V, ML>`.
///
/// # Safety
///
/// - `ptr` must be non-null and point to an occupied `SlotCell`.
#[inline]
unsafe fn node_deref<K, V, const MAX_LEVEL: usize>(
    ptr: NodePtr<K, V, MAX_LEVEL>,
) -> *const SkipNode<K, V, MAX_LEVEL> {
    // SAFETY: Caller guarantees ptr is non-null and points to an occupied slot.
    // Use addr_of! to avoid creating an intermediate reference (matches node_deref_mut).
    // ManuallyDrop<MaybeUninit<T>> has the same layout as T.
    unsafe { std::ptr::addr_of!((*ptr).value).cast() }
}

/// Dereferences a `NodePtr<K, V, ML>` to `*mut SkipNode<K, V, ML>`.
///
/// # Safety
///
/// - `ptr` must be non-null and point to an occupied `SlotCell`.
/// - The caller must ensure no other reference to the same node exists.
#[inline]
unsafe fn node_deref_mut<K, V, const MAX_LEVEL: usize>(
    ptr: NodePtr<K, V, MAX_LEVEL>,
) -> *mut SkipNode<K, V, MAX_LEVEL> {
    // SAFETY: Caller guarantees ptr is non-null, occupied, and unaliased.
    // Use addr_of_mut! to avoid implicit DerefMut on ManuallyDrop union field.
    // ManuallyDrop<MaybeUninit<T>> has the same layout as T.
    unsafe { std::ptr::addr_of_mut!((*ptr).value).cast() }
}

// =============================================================================
// SkipList<K, V, A, MAX_LEVEL, RATIO>
// =============================================================================

/// A probabilistic sorted map with internal slab allocation.
///
/// # Complexity
///
/// | Operation    | Expected Time |
/// |--------------|---------------|
/// | insert       | O(log n)      |
/// | remove       | O(log n)      |
/// | get / get_mut| O(log n)      |
/// | first / last | O(1)          |
/// | pop_first    | O(1)          |
/// | pop_last     | O(log n)      |
/// | contains_key | O(log n)      |
///
/// # Allocation Model
///
/// The skip list manages node allocation internally via a ZST allocator:
/// - Bounded: `try_insert` may fail with `Full<(K, V)>`
/// - Unbounded: `insert` always succeeds
/// - `remove`/`pop` deallocate internally and return values directly
///
/// # Type Parameters
///
/// - `K`: Key type, must implement `Ord`
/// - `V`: Value type
/// - `A`: Allocator type (generated by [`skip_allocator!`](crate::skip_allocator))
/// - `MAX_LEVEL`: Maximum skip list height. Determines node size and max efficient
///   population (`RATIO^MAX_LEVEL`). Configured via [`skip_allocator!`](crate::skip_allocator)
///   (default 8). Use 4-6 with `RATIO=4` for matching engines, 8 with `RATIO=2`
///   for market data books.
/// - `RATIO`: Level ratio (power of two, >= 2). Controls memory vs search speed.
///   `2` = standard (p=0.5, ~2 pointers/node avg, more vertical search).
///   `4` = Redis-style (p=0.25, ~1.33 pointers/node avg, smaller nodes).
pub struct SkipList<
    K: Ord,
    V: 'static,
    A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
    const MAX_LEVEL: usize,
    const RATIO: u32,
> {
    head: [NodePtr<K, V, MAX_LEVEL>; MAX_LEVEL],
    tail: NodePtr<K, V, MAX_LEVEL>,
    rng_state: u64,
    level: usize,
    len: usize,
    _marker: PhantomData<A>,
}

// =============================================================================
// impl<A: Alloc> — base block (queries, remove, pop, clear, iter, cursor, drain)
// =============================================================================

impl<
        K: Ord,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > SkipList<K, V, A, MAX_LEVEL, RATIO>
{
    /// Creates a new empty skip list with default settings.
    ///
    /// Seeds the RNG from the current thread's stack address for
    /// per-instance entropy without syscalls.
    #[inline]
    pub fn new(alloc: A) -> Self {
        // Use stack address as cheap per-instance entropy source.
        // No syscall, no external dependency, good enough to decorrelate
        // level distributions across skip list instances.
        let entropy = &() as *const () as u64;
        Self::with_seed(entropy, alloc)
    }

    /// Creates a new empty skip list with a custom RNG seed.
    #[inline]
    #[allow(unused_variables, clippy::needless_pass_by_value)]
    pub fn with_seed(seed: u64, alloc: A) -> Self {
        const {
            assert!(MAX_LEVEL >= 1, "MAX_LEVEL must be >= 1");
            assert!(MAX_LEVEL <= 32, "MAX_LEVEL must be <= 32");
            assert!(RATIO >= 2, "RATIO must be >= 2");
            assert!(RATIO.is_power_of_two(), "RATIO must be a power of two");
        }
        let seed = if seed == 0 { 1 } else { seed };
        SkipList {
            head: [ptr::null_mut(); MAX_LEVEL],
            tail: ptr::null_mut(),
            rng_state: seed,
            level: 0,
            len: 0,
            _marker: PhantomData,
        }
    }

    // =========================================================================
    // Queries
    // =========================================================================

    /// Returns the number of elements in the skip list.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the skip list is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns `true` if the skip list contains the given key.
    #[inline]
    pub fn contains_key(&self, key: &K) -> bool {
        self.find(key).is_some()
    }

    /// Returns a reference to the value for the given key.
    #[inline]
    pub fn get(&self, key: &K) -> Option<&V> {
        let ptr = self.find(key)?;
        // SAFETY: find returns a valid, occupied node pointer.
        // Reference lifetime bounded by &self.
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
    #[inline]
    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        if self.head[0].is_null() {
            return None;
        }
        // SAFETY: head[0] is non-null and points to an occupied slot.
        let node = unsafe { &*node_deref(self.head[0]) };
        Some((&node.key, &node.value))
    }

    /// Returns the last (largest) key-value pair.
    ///
    /// O(1) due to maintained tail pointer.
    #[inline]
    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        if self.tail.is_null() {
            return None;
        }
        // SAFETY: tail is non-null and points to an occupied slot.
        let node = unsafe { &*node_deref(self.tail) };
        Some((&node.key, &node.value))
    }

    // =========================================================================
    // Mutation — remove / pop (all deallocate internally)
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
        let mut update = [ptr::null_mut(); MAX_LEVEL];
        let ptr = self.search(key, &mut update)?;

        let node_level = unsafe { &*node_deref(ptr) }.level.get();
        self.unlink_at_levels(ptr, node_level, &update);
        self.update_tail_and_level(ptr, &update);
        self.len -= 1;

        // SAFETY: ptr was in the skip list (from search), now unlinked.
        // take() moves the value out and returns the slot to the freelist.
        let slot = unsafe { Slot::from_ptr(ptr) };
        let node = unsafe { A::take(slot) };
        Some(node.into_data())
    }

    /// Removes and returns the first (smallest) key-value pair.
    ///
    /// O(1) — no search needed since we have head pointers.
    #[inline]
    pub fn pop_first(&mut self) -> Option<(K, V)> {
        if self.head[0].is_null() {
            return None;
        }

        let ptr = self.head[0];
        let node = unsafe { &*node_deref(ptr) };
        let node_level = node.level.get();

        debug_assert!((node_level as usize) < MAX_LEVEL, "node_level out of bounds");
        // SAFETY: node_level was set by random_level() which caps at MAX_LEVEL - 1.
        unsafe { std::hint::assert_unchecked((node_level as usize) < MAX_LEVEL) };

        // Update head pointers at all levels this node participates in
        for i in 0..=node_level as usize {
            self.head[i] = node.forward[i].get();
        }

        // Update back pointer of new first node
        let new_first = self.head[0];
        if new_first.is_null() {
            self.tail = ptr::null_mut();
        } else {
            // SAFETY: new_first is non-null, in the skip list
            unsafe { &*node_deref(new_first) }.back.set(ptr::null_mut());
        }

        // Reduce level if needed
        while self.level > 0 && self.head[self.level].is_null() {
            self.level -= 1;
        }

        self.len -= 1;

        // SAFETY: ptr was head of skip list, now unlinked.
        let slot = unsafe { Slot::from_ptr(ptr) };
        let node = unsafe { A::take(slot) };
        Some(node.into_data())
    }

    /// Removes and returns the last (largest) key-value pair.
    ///
    /// O(log n) — requires search for predecessors.
    #[inline]
    pub fn pop_last(&mut self) -> Option<(K, V)> {
        if self.tail.is_null() {
            return None;
        }

        let ptr = self.tail;

        // Search for predecessors of the tail node
        let mut update = [ptr::null_mut(); MAX_LEVEL];
        // SAFETY: ptr is valid (it's the tail)
        self.search(unsafe { &(*node_deref(ptr)).key }, &mut update);

        let node_level = unsafe { &*node_deref(ptr) }.level.get();
        self.unlink_at_levels(ptr, node_level, &update);

        // New tail is predecessor at level 0
        self.tail = update[0];

        // Reduce level if needed
        while self.level > 0 && self.head[self.level].is_null() {
            self.level -= 1;
        }

        self.len -= 1;

        // SAFETY: ptr was tail, now unlinked.
        let slot = unsafe { Slot::from_ptr(ptr) };
        let node = unsafe { A::take(slot) };
        Some(node.into_data())
    }

    /// Removes all nodes, freeing them via the allocator.
    #[inline]
    pub fn clear(&mut self) {
        let mut current = self.head[0];
        while !current.is_null() {
            // SAFETY: current is non-null, points to an occupied slot
            let next = unsafe { &*node_deref(current) }.forward[0].get();

            // Free via allocator: drops SkipNode and returns slot to freelist
            let slot = unsafe { Slot::from_ptr(current) };
            unsafe { A::free(slot) };

            current = next;
        }

        self.head = [ptr::null_mut(); MAX_LEVEL];
        self.tail = ptr::null_mut();
        self.level = 0;
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
    /// use nexus_collections::skiplist::Entry;
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
    pub fn entry(&mut self, key: K) -> Entry<'_, K, V, A, MAX_LEVEL, RATIO> {
        let mut update = [ptr::null_mut(); MAX_LEVEL];
        let found = self.search(&key, &mut update);

        match found {
            Some(ptr) => {
                // Key dropped — existing key stays in the map (matches BTreeMap)
                drop(key);
                Entry::Occupied(OccupiedEntry {
                    list: self,
                    ptr,
                    update,
                })
            }
            None => Entry::Vacant(VacantEntry {
                list: self,
                key,
                update,
            }),
        }
    }

    // =========================================================================
    // Iteration
    // =========================================================================

    /// Returns an iterator over `(&K, &V)` pairs in sorted order.
    ///
    /// Supports forward and reverse iteration (`DoubleEndedIterator`).
    #[inline]
    pub fn iter(&self) -> Iter<'_, K, V, MAX_LEVEL> {
        Iter {
            front: self.head[0],
            back: self.tail,
            len: self.len,
            _marker: PhantomData,
        }
    }

    /// Returns an iterator over keys in sorted order.
    #[inline]
    pub fn keys(&self) -> Keys<'_, K, V, MAX_LEVEL> {
        Keys { inner: self.iter() }
    }

    /// Returns an iterator over values in key-sorted order.
    #[inline]
    pub fn values(&self) -> Values<'_, K, V, MAX_LEVEL> {
        Values { inner: self.iter() }
    }

    /// Returns a mutable iterator over `(&K, &mut V)` pairs in sorted order.
    ///
    /// Keys are immutable — changing them would violate sorted order.
    /// Supports forward and reverse iteration (`DoubleEndedIterator`).
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, K, V, MAX_LEVEL> {
        IterMut {
            front: self.head[0],
            back: self.tail,
            len: self.len,
            _marker: PhantomData,
        }
    }

    /// Returns a mutable iterator over values in key-sorted order.
    #[inline]
    pub fn values_mut(&mut self) -> ValuesMut<'_, K, V, MAX_LEVEL> {
        ValuesMut {
            inner: self.iter_mut(),
        }
    }

    /// Returns an iterator over `(&K, &V)` pairs within the given range.
    ///
    /// Useful for market data: "give me all price levels between X and Y."
    #[inline]
    pub fn range<R: std::ops::RangeBounds<K>>(&self, range: R) -> Range<'_, K, V, MAX_LEVEL> {
        let (front, back) = self.resolve_range_bounds(range);
        Range {
            front,
            back,
            _marker: PhantomData,
        }
    }

    /// Returns a mutable iterator over `(&K, &mut V)` pairs within the given range.
    #[inline]
    pub fn range_mut<R: std::ops::RangeBounds<K>>(
        &mut self,
        range: R,
    ) -> RangeMut<'_, K, V, MAX_LEVEL> {
        let (front, back) = self.resolve_range_bounds(range);
        RangeMut {
            front,
            back,
            _marker: PhantomData,
        }
    }

    // =========================================================================
    // Cursor
    // =========================================================================

    /// Returns a cursor positioned before the first element.
    ///
    /// Call `advance()` to move to the first element. Cursors track predecessors
    /// at each level, enabling O(1) removal at the current position.
    #[inline]
    pub fn cursor_front(&mut self) -> Cursor<'_, K, V, A, MAX_LEVEL, RATIO> {
        Cursor {
            list: self,
            current: ptr::null_mut(),
            update: [ptr::null_mut(); MAX_LEVEL],
            started: false,
            past_end: false,
        }
    }

    /// Returns a cursor positioned at the given key, or at the first
    /// element greater than the key.
    #[inline]
    pub fn cursor_at(&mut self, key: &K) -> Cursor<'_, K, V, A, MAX_LEVEL, RATIO> {
        let mut update = [ptr::null_mut(); MAX_LEVEL];
        let found = self.search(key, &mut update);
        let current = match found {
            Some(ptr) => ptr,
            None => {
                // Position at first element > key
                if update[0].is_null() {
                    self.head[0]
                } else {
                    unsafe { &*node_deref(update[0]) }.forward[0].get()
                }
            }
        };

        Cursor {
            list: self,
            current,
            update,
            started: true,
            past_end: current.is_null(),
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
    pub fn drain(&mut self) -> Drain<'_, K, V, A, MAX_LEVEL, RATIO> {
        Drain { list: self }
    }

    // =========================================================================
    // Internal: link_vacant — for Entry API
    // =========================================================================

    /// Internal helper: links a pre-allocated slot at a known-vacant position.
    ///
    /// The `update` array must come from a prior `search` that found no match.
    /// Returns a raw pointer to the value — caller assigns the lifetime.
    ///
    /// # Safety
    ///
    /// - `slot` must be a valid, occupied slot
    /// - `update` must be from a search for the slot's key that found no match
    #[allow(clippy::needless_pass_by_value)] // Intentional: consumes Slot to prevent reuse
    unsafe fn link_vacant(
        &mut self,
        slot: Slot<SkipNode<K, V, MAX_LEVEL>>,
        update: &[NodePtr<K, V, MAX_LEVEL>; MAX_LEVEL],
    ) -> *mut V {
        let ptr = slot.as_ptr();
        let new_level = self.random_level();
        // SAFETY: ptr is valid (from slot).
        unsafe { &*node_deref(ptr) }.level.set(new_level);
        self.link_node(ptr, new_level, update);
        // SAFETY: ptr was just linked into the skip list.
        unsafe { std::ptr::addr_of_mut!((*node_deref_mut(ptr)).value) }
    }

    // =========================================================================
    // Internal algorithms
    // =========================================================================

    /// Read-only search — returns pointer to node with exact key, or `None`.
    #[inline]
    fn find(&self, key: &K) -> Option<NodePtr<K, V, MAX_LEVEL>> {
        debug_assert!(self.level < MAX_LEVEL, "self.level out of bounds");
        // SAFETY: self.level is maintained as < MAX_LEVEL by link_node and update_tail_and_level.
        unsafe { std::hint::assert_unchecked(self.level < MAX_LEVEL) };

        let mut current: NodePtr<K, V, MAX_LEVEL> = ptr::null_mut();

        for i in (0..=self.level).rev() {
            let mut next = if current.is_null() {
                self.head[i]
            } else {
                // SAFETY: current is a valid occupied node
                unsafe { &*node_deref(current) }.forward[i].get()
            };

            while !next.is_null() {
                // SAFETY: next is non-null, points to occupied slot
                let next_node = unsafe { &*node_deref(next) };
                if next_node.key >= *key {
                    break;
                }
                current = next;
                next = next_node.forward[i].get();
            }
        }

        // Check exact match at level 0
        let candidate = if current.is_null() {
            self.head[0]
        } else {
            unsafe { &*node_deref(current) }.forward[0].get()
        };

        if !candidate.is_null() && unsafe { &*node_deref(candidate) }.key == *key {
            Some(candidate)
        } else {
            None
        }
    }

    /// Mutation search — fills `update` with predecessor at each level.
    /// Returns pointer to node with exact key, or `None`.
    #[inline]
    fn search(
        &self,
        key: &K,
        update: &mut [NodePtr<K, V, MAX_LEVEL>; MAX_LEVEL],
    ) -> Option<NodePtr<K, V, MAX_LEVEL>> {
        debug_assert!(self.level < MAX_LEVEL, "self.level out of bounds");
        // SAFETY: self.level is maintained as < MAX_LEVEL by link_node and update_tail_and_level.
        unsafe { std::hint::assert_unchecked(self.level < MAX_LEVEL) };

        let mut current: NodePtr<K, V, MAX_LEVEL> = ptr::null_mut();

        for i in (0..=self.level).rev() {
            let mut next = if current.is_null() {
                self.head[i]
            } else {
                // SAFETY: current is a valid occupied node
                unsafe { &*node_deref(current) }.forward[i].get()
            };

            while !next.is_null() {
                // SAFETY: next is non-null, points to occupied slot
                let next_node = unsafe { &*node_deref(next) };
                if next_node.key >= *key {
                    break;
                }
                current = next;
                next = next_node.forward[i].get();
            }

            update[i] = current;
        }

        // Check exact match at level 0
        let candidate = if current.is_null() {
            self.head[0]
        } else {
            unsafe { &*node_deref(current) }.forward[0].get()
        };

        if !candidate.is_null() && unsafe { &*node_deref(candidate) }.key == *key {
            Some(candidate)
        } else {
            None
        }
    }

    /// Generates a random level using geometric distribution.
    ///
    /// Uses xorshift64 and counts trailing ones, divided by
    /// `log2(RATIO)` which the compiler constant-folds at monomorphization.
    #[inline]
    fn random_level(&mut self) -> u8 {
        let r = self.xorshift64();
        // RATIO.trailing_zeros() is const-folded: RATIO=2 → 1, RATIO=4 → 2, etc.
        // For RATIO=2 this becomes `trailing_ones() / 1` → identity (no division).
        let level = r.trailing_ones() / RATIO.trailing_zeros();
        level.min(MAX_LEVEL as u32 - 1) as u8
    }

    /// xorshift64 PRNG.
    #[inline]
    fn xorshift64(&mut self) -> u64 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        x
    }

    /// Returns the last node with key < `target`, or null if no such node.
    ///
    /// Same traversal as `find()` but returns the predecessor, not the match.
    #[inline]
    fn find_predecessor(&self, target: &K) -> NodePtr<K, V, MAX_LEVEL> {
        debug_assert!(self.level < MAX_LEVEL, "self.level out of bounds");
        // SAFETY: self.level is maintained as < MAX_LEVEL by link_node and update_tail_and_level.
        unsafe { std::hint::assert_unchecked(self.level < MAX_LEVEL) };

        let mut current: NodePtr<K, V, MAX_LEVEL> = ptr::null_mut();

        for i in (0..=self.level).rev() {
            let mut next = if current.is_null() {
                self.head[i]
            } else {
                // SAFETY: current is a valid occupied node
                unsafe { &*node_deref(current) }.forward[i].get()
            };

            while !next.is_null() {
                // SAFETY: next is non-null, points to occupied slot
                let next_node = unsafe { &*node_deref(next) };
                if next_node.key >= *target {
                    break;
                }
                current = next;
                next = next_node.forward[i].get();
            }
        }

        current
    }

    /// Resolves `RangeBounds` to `(front, back)` node pointers.
    ///
    /// Both `range()` and `range_mut()` use this to convert bounds to pointers.
    #[inline]
    fn resolve_range_bounds<R: std::ops::RangeBounds<K>>(
        &self,
        range: R,
    ) -> (NodePtr<K, V, MAX_LEVEL>, NodePtr<K, V, MAX_LEVEL>) {
        use std::ops::Bound;

        // Resolve front pointer
        let front = match range.start_bound() {
            Bound::Unbounded => self.head[0],
            Bound::Included(k) => {
                // First node >= k
                let pred = self.find_predecessor(k);
                if pred.is_null() {
                    self.head[0]
                } else {
                    // SAFETY: pred is non-null, in the skip list
                    unsafe { &*node_deref(pred) }.forward[0].get()
                }
            }
            Bound::Excluded(k) => {
                // Find exact match or first node > k
                let pred = self.find_predecessor(k);
                let candidate = if pred.is_null() {
                    self.head[0]
                } else {
                    // SAFETY: pred is non-null, in the skip list
                    unsafe { &*node_deref(pred) }.forward[0].get()
                };
                // candidate is first node >= k; skip if exact match
                if !candidate.is_null() && unsafe { &*node_deref(candidate) }.key == *k {
                    unsafe { &*node_deref(candidate) }.forward[0].get()
                } else {
                    candidate
                }
            }
        };

        // Resolve back pointer
        let back = match range.end_bound() {
            Bound::Unbounded => self.tail,
            Bound::Included(k) => {
                // Find the node with key == k, or the last node < k
                let pred = self.find_predecessor(k);
                let candidate = if pred.is_null() {
                    self.head[0]
                } else {
                    // SAFETY: pred is non-null, in the skip list
                    unsafe { &*node_deref(pred) }.forward[0].get()
                };
                // candidate is first node >= k; if exact match, use it
                if !candidate.is_null() && unsafe { &*node_deref(candidate) }.key == *k {
                    candidate
                } else {
                    // No exact match — back is predecessor
                    pred
                }
            }
            Bound::Excluded(k) => {
                // Last node < k is the predecessor
                self.find_predecessor(k)
            }
        };

        // Validate: front must be <= back (in sorted order).
        // If either is null, or front's key > back's key, range is empty.
        if front.is_null() || back.is_null() {
            return (ptr::null_mut(), ptr::null_mut());
        }

        // SAFETY: both pointers are non-null and in the skip list
        let front_key = unsafe { &(*node_deref(front)).key };
        let back_key = unsafe { &(*node_deref(back)).key };
        if front_key > back_key {
            return (ptr::null_mut(), ptr::null_mut());
        }

        (front, back)
    }

    /// Links a node into the skip list at the position described by `update`.
    #[inline]
    fn link_node(
        &mut self,
        ptr: NodePtr<K, V, MAX_LEVEL>,
        new_level: u8,
        update: &[NodePtr<K, V, MAX_LEVEL>; MAX_LEVEL],
    ) {
        debug_assert!((new_level as usize) < MAX_LEVEL, "new_level out of bounds");
        // SAFETY: new_level is from random_level() which caps at MAX_LEVEL - 1.
        unsafe { std::hint::assert_unchecked((new_level as usize) < MAX_LEVEL) };

        // SAFETY: ptr points to a valid, occupied node
        let node = unsafe { &*node_deref(ptr) };
        let mut is_tail = true;

        for i in 0..=new_level as usize {
            let next = if update[i].is_null() {
                self.head[i]
            } else {
                // SAFETY: update[i] is a valid occupied node (from search)
                unsafe { &*node_deref(update[i]) }.forward[i].get()
            };

            node.forward[i].set(next);

            if i == 0 && !next.is_null() {
                is_tail = false;
            }

            if update[i].is_null() {
                self.head[i] = ptr;
            } else {
                unsafe { &*node_deref(update[i]) }.forward[i].set(ptr);
            }
        }

        // Maintain back pointer at level 0
        node.back.set(update[0]);
        let next_at_0 = node.forward[0].get();
        if !next_at_0.is_null() {
            // SAFETY: next_at_0 is non-null, points to occupied slot
            unsafe { &*node_deref(next_at_0) }.back.set(ptr);
        }

        if is_tail {
            self.tail = ptr;
        }

        if (new_level as usize) > self.level {
            self.level = new_level as usize;
        }

        self.len += 1;
    }

    /// Unlinks a node from forward and back pointers at each level.
    /// Does NOT update tail, level, or len — caller handles those.
    #[inline]
    fn unlink_at_levels(
        &mut self,
        ptr: NodePtr<K, V, MAX_LEVEL>,
        node_level: u8,
        update: &[NodePtr<K, V, MAX_LEVEL>; MAX_LEVEL],
    ) {
        debug_assert!((node_level as usize) < MAX_LEVEL, "node_level out of bounds");
        // SAFETY: node_level was set by random_level() which caps at MAX_LEVEL - 1.
        unsafe { std::hint::assert_unchecked((node_level as usize) < MAX_LEVEL) };

        let node = unsafe { &*node_deref(ptr) };

        for i in 0..=node_level as usize {
            let next = node.forward[i].get();

            if update[i].is_null() {
                self.head[i] = next;
            } else {
                unsafe { &*node_deref(update[i]) }.forward[i].set(next);
            }
        }

        // Update back pointer of successor at level 0
        let next_at_0 = node.forward[0].get();
        if !next_at_0.is_null() {
            unsafe { &*node_deref(next_at_0) }.back.set(node.back.get());
        }
    }

    /// Updates tail pointer and reduces level after an unlink.
    #[inline]
    fn update_tail_and_level(
        &mut self,
        ptr: NodePtr<K, V, MAX_LEVEL>,
        update: &[NodePtr<K, V, MAX_LEVEL>; MAX_LEVEL],
    ) {
        // If removed node was tail, new tail is its predecessor at level 0
        if unsafe { &*node_deref(ptr) }.forward[0].get().is_null() {
            self.tail = update[0];
        }

        // Reduce level if needed
        while self.level > 0 && self.head[self.level].is_null() {
            self.level -= 1;
        }
    }
}

// =============================================================================
// impl<A: BoundedAlloc> — try_insert
// =============================================================================

impl<
        K: Ord,
        V: 'static,
        A: BoundedAlloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > SkipList<K, V, A, MAX_LEVEL, RATIO>
{
    /// Inserts a key-value pair, or returns the pair if the allocator is full.
    ///
    /// If a node with the same key already exists, the value is replaced
    /// and the old value is returned inside `Ok(Some(old_value))`. This
    /// path is zero-allocation — the value is replaced in-place.
    ///
    /// Returns `Err(Full((key, value)))` if the allocator cannot allocate.
    #[inline]
    pub fn try_insert(&mut self, key: K, value: V) -> Result<Option<V>, Full<(K, V)>> {
        let mut update = [ptr::null_mut(); MAX_LEVEL];
        let found = self.search(&key, &mut update);

        if let Some(existing_ptr) = found {
            // Key exists: replace value in-place, no allocation needed.
            // key dropped here — existing key stays in the map (matches BTreeMap).
            // SAFETY: existing_ptr is valid and in the skip list; &mut self
            // prevents aliasing.
            let existing = unsafe { &mut (*node_deref_mut(existing_ptr)).value };
            return Ok(Some(std::mem::replace(existing, value)));
        }

        match A::try_alloc(SkipNode::new(key, value)) {
            Ok(slot) => {
                let ptr = slot.as_ptr();
                let new_level = self.random_level();
                // SAFETY: ptr is valid (from slot we own).
                unsafe { &*node_deref(ptr) }.level.set(new_level);
                self.link_node(ptr, new_level, &update);
                Ok(None)
            }
            Err(full) => Err(Full(full.into_inner().into_data())),
        }
    }
}

// =============================================================================
// impl<A: UnboundedAlloc> — insert
// =============================================================================

impl<
        K: Ord,
        V: 'static,
        A: UnboundedAlloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > SkipList<K, V, A, MAX_LEVEL, RATIO>
{
    /// Inserts a key-value pair. Always succeeds (grows as needed).
    ///
    /// If a node with the same key already exists, the value is replaced
    /// and the old value is returned. This path is zero-allocation — the
    /// value is replaced in-place.
    #[inline]
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        let mut update = [ptr::null_mut(); MAX_LEVEL];
        let found = self.search(&key, &mut update);

        if let Some(existing_ptr) = found {
            // Key exists: replace value in-place, no allocation needed.
            // key dropped here — existing key stays in the map (matches BTreeMap).
            // SAFETY: existing_ptr is valid and in the skip list; &mut self
            // prevents aliasing.
            let existing = unsafe { &mut (*node_deref_mut(existing_ptr)).value };
            return Some(std::mem::replace(existing, value));
        }

        let slot = A::alloc(SkipNode::new(key, value));
        let ptr = slot.as_ptr();
        let new_level = self.random_level();
        // SAFETY: ptr is valid (from slot we own).
        unsafe { &*node_deref(ptr) }.level.set(new_level);
        self.link_node(ptr, new_level, &update);
        None
    }
}

// =============================================================================
// Drop (on base Alloc)
// =============================================================================

impl<
        K: Ord,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > Drop for SkipList<K, V, A, MAX_LEVEL, RATIO>
{
    fn drop(&mut self) {
        self.clear();
    }
}

// =============================================================================
// Entry API
// =============================================================================

/// A view into a single entry in the skip list, which may be vacant or occupied.
///
/// Constructed via [`SkipList::entry`].
pub enum Entry<
    'a,
    K: Ord,
    V: 'static,
    A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
    const MAX_LEVEL: usize,
    const RATIO: u32,
> {
    /// An occupied entry — key exists in the skip list.
    Occupied(OccupiedEntry<'a, K, V, A, MAX_LEVEL, RATIO>),
    /// A vacant entry — key does not exist.
    Vacant(VacantEntry<'a, K, V, A, MAX_LEVEL, RATIO>),
}

/// A view into an occupied entry in the skip list.
pub struct OccupiedEntry<
    'a,
    K: Ord,
    V: 'static,
    A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
    const MAX_LEVEL: usize,
    const RATIO: u32,
> {
    list: &'a mut SkipList<K, V, A, MAX_LEVEL, RATIO>,
    ptr: NodePtr<K, V, MAX_LEVEL>,
    update: [NodePtr<K, V, MAX_LEVEL>; MAX_LEVEL],
}

/// A view into a vacant entry in the skip list.
pub struct VacantEntry<
    'a,
    K: Ord,
    V: 'static,
    A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
    const MAX_LEVEL: usize,
    const RATIO: u32,
> {
    list: &'a mut SkipList<K, V, A, MAX_LEVEL, RATIO>,
    key: K,
    update: [NodePtr<K, V, MAX_LEVEL>; MAX_LEVEL],
}

// -- Entry: base Alloc methods --

impl<
        K: Ord,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > Entry<'_, K, V, A, MAX_LEVEL, RATIO>
{
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

impl<
        'a,
        K: Ord,
        V: 'static,
        A: BoundedAlloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > Entry<'a, K, V, A, MAX_LEVEL, RATIO>
{
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
}

// -- Entry: UnboundedAlloc methods --

impl<
        'a,
        K: Ord,
        V: 'static,
        A: UnboundedAlloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > Entry<'a, K, V, A, MAX_LEVEL, RATIO>
{
    /// Ensures a value is in the entry by inserting if vacant (unbounded).
    #[inline]
    pub fn or_insert(self, value: V) -> &'a mut V {
        match self {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => e.insert(value),
        }
    }
}

// -- OccupiedEntry --

impl<
        'a,
        K: Ord,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > OccupiedEntry<'a, K, V, A, MAX_LEVEL, RATIO>
{
    /// Returns a reference to the key.
    #[inline]
    pub fn key(&self) -> &K {
        // SAFETY: ptr is valid, node is in the skip list
        unsafe { &(*node_deref(self.ptr)).key }
    }

    /// Returns a reference to the value.
    #[inline]
    pub fn get(&self) -> &V {
        // SAFETY: ptr is valid, node is in the skip list
        unsafe { &(*node_deref(self.ptr)).value }
    }

    /// Returns a mutable reference to the value.
    #[inline]
    pub fn get_mut(&mut self) -> &mut V {
        // SAFETY: ptr is valid, &mut self prevents aliasing
        unsafe { &mut (*node_deref_mut(self.ptr)).value }
    }

    /// Converts to a mutable reference to the value with the entry's lifetime.
    #[inline]
    pub fn into_mut(self) -> &'a mut V {
        // SAFETY: ptr is valid, the entry consumed &'a mut SkipList,
        // so the returned reference continues that exclusive borrow.
        unsafe { &mut (*node_deref_mut(self.ptr)).value }
    }

    /// Removes the entry and returns `(key, value)`.
    #[inline]
    pub fn remove(self) -> (K, V) {
        let node_level = unsafe { &*node_deref(self.ptr) }.level.get();
        self.list
            .unlink_at_levels(self.ptr, node_level, &self.update);
        self.list.update_tail_and_level(self.ptr, &self.update);
        self.list.len -= 1;

        // SAFETY: ptr was in the skip list, now unlinked
        let slot = unsafe { Slot::from_ptr(self.ptr) };
        let node = unsafe { A::take(slot) };
        node.into_data()
    }
}

// -- VacantEntry: BoundedAlloc --

impl<
        'a,
        K: Ord,
        V: 'static,
        A: BoundedAlloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > VacantEntry<'a, K, V, A, MAX_LEVEL, RATIO>
{
    /// Inserts a value into the vacant entry (bounded allocator).
    ///
    /// Returns `Err(Full((K, V)))` if the allocator is full.
    #[inline]
    pub fn try_insert(self, value: V) -> Result<&'a mut V, Full<(K, V)>> {
        let VacantEntry { list, key, update } = self;
        match A::try_alloc(SkipNode::new(key, value)) {
            Ok(slot) => {
                let val_ptr = unsafe { list.link_vacant(slot, &update) };
                // SAFETY: val_ptr points to slab memory owned by the skip list.
                // 'a is bounded by the &'a mut SkipList that VacantEntry consumed,
                // so the node won't be freed during 'a.
                Ok(unsafe { &mut *val_ptr })
            }
            Err(full) => Err(Full(full.into_inner().into_data())),
        }
    }
}

// -- VacantEntry: UnboundedAlloc --

impl<
        'a,
        K: Ord,
        V: 'static,
        A: UnboundedAlloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > VacantEntry<'a, K, V, A, MAX_LEVEL, RATIO>
{
    /// Inserts a value into the vacant entry (unbounded allocator).
    #[inline]
    pub fn insert(self, value: V) -> &'a mut V {
        let VacantEntry { list, key, update } = self;
        let slot = A::alloc(SkipNode::new(key, value));
        let val_ptr = unsafe { list.link_vacant(slot, &update) };
        // SAFETY: val_ptr points to slab memory owned by the skip list.
        // 'a is bounded by the &'a mut SkipList that VacantEntry consumed,
        // so the node won't be freed during 'a.
        unsafe { &mut *val_ptr }
    }
}

// -- VacantEntry: base Alloc (key accessor) --

impl<
        K: Ord,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > VacantEntry<'_, K, V, A, MAX_LEVEL, RATIO>
{
    /// Returns a reference to the key that would be used for insertion.
    #[inline]
    pub fn key(&self) -> &K {
        &self.key
    }
}

// =============================================================================
// Iter — borrowing, double-ended
// =============================================================================

/// Iterator over `(&K, &V)` pairs in sorted order.
///
/// Supports both forward and reverse iteration.
pub struct Iter<'a, K, V, const MAX_LEVEL: usize> {
    front: NodePtr<K, V, MAX_LEVEL>,
    back: NodePtr<K, V, MAX_LEVEL>,
    len: usize,
    _marker: PhantomData<&'a ()>,
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> Iterator for Iter<'a, K, V, MAX_LEVEL> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }

        let ptr = self.front;
        // SAFETY: len > 0 implies front is non-null and points to an occupied
        // slot. The iterator borrows &'a SkipList, so the reference is valid.
        let node = unsafe { &*node_deref(ptr) };
        self.front = node.forward[0].get();
        self.len -= 1;
        Some((&node.key, &node.value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> DoubleEndedIterator
    for Iter<'a, K, V, MAX_LEVEL>
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }

        let ptr = self.back;
        // SAFETY: len > 0 implies back is non-null and points to an occupied slot.
        let node = unsafe { &*node_deref(ptr) };
        self.back = node.back.get();
        self.len -= 1;
        Some((&node.key, &node.value))
    }
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> ExactSizeIterator
    for Iter<'a, K, V, MAX_LEVEL>
{
}

impl<
        'a,
        K: Ord + 'a,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > IntoIterator for &'a SkipList<K, V, A, MAX_LEVEL, RATIO>
{
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V, MAX_LEVEL>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<
        'a,
        K: Ord + 'a,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > IntoIterator for &'a mut SkipList<K, V, A, MAX_LEVEL, RATIO>
{
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V, MAX_LEVEL>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

// =============================================================================
// Keys / Values
// =============================================================================

/// Iterator over keys in sorted order.
pub struct Keys<'a, K, V, const MAX_LEVEL: usize> {
    inner: Iter<'a, K, V, MAX_LEVEL>,
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> Iterator for Keys<'a, K, V, MAX_LEVEL> {
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

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> DoubleEndedIterator
    for Keys<'a, K, V, MAX_LEVEL>
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(k, _)| k)
    }
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> ExactSizeIterator
    for Keys<'a, K, V, MAX_LEVEL>
{
}

/// Iterator over values in key-sorted order.
pub struct Values<'a, K, V, const MAX_LEVEL: usize> {
    inner: Iter<'a, K, V, MAX_LEVEL>,
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> Iterator for Values<'a, K, V, MAX_LEVEL> {
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

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> DoubleEndedIterator
    for Values<'a, K, V, MAX_LEVEL>
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, v)| v)
    }
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> ExactSizeIterator
    for Values<'a, K, V, MAX_LEVEL>
{
}

// =============================================================================
// IterMut — mutable borrowing, double-ended
// =============================================================================

/// Mutable iterator over `(&K, &mut V)` pairs in sorted order.
///
/// Keys are immutable — changing them would violate sorted order.
pub struct IterMut<'a, K, V, const MAX_LEVEL: usize> {
    front: NodePtr<K, V, MAX_LEVEL>,
    back: NodePtr<K, V, MAX_LEVEL>,
    len: usize,
    _marker: PhantomData<&'a mut ()>,
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> Iterator for IterMut<'a, K, V, MAX_LEVEL> {
    type Item = (&'a K, &'a mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }

        let ptr = self.front;
        // SAFETY: len > 0 implies front is non-null and points to an occupied
        // slot. The iterator holds &'a mut SkipList, so no aliasing.
        let node = unsafe { &mut *node_deref_mut(ptr) };
        self.front = node.forward[0].get();
        self.len -= 1;
        Some((&node.key, &mut node.value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> DoubleEndedIterator
    for IterMut<'a, K, V, MAX_LEVEL>
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.len == 0 {
            return None;
        }

        let ptr = self.back;
        // SAFETY: len > 0 implies back is non-null and points to an occupied slot.
        let node = unsafe { &mut *node_deref_mut(ptr) };
        self.back = node.back.get();
        self.len -= 1;
        Some((&node.key, &mut node.value))
    }
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> ExactSizeIterator
    for IterMut<'a, K, V, MAX_LEVEL>
{
}

// =============================================================================
// ValuesMut
// =============================================================================

/// Mutable iterator over values in key-sorted order.
pub struct ValuesMut<'a, K, V, const MAX_LEVEL: usize> {
    inner: IterMut<'a, K, V, MAX_LEVEL>,
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> Iterator for ValuesMut<'a, K, V, MAX_LEVEL> {
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

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> DoubleEndedIterator
    for ValuesMut<'a, K, V, MAX_LEVEL>
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, v)| v)
    }
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> ExactSizeIterator
    for ValuesMut<'a, K, V, MAX_LEVEL>
{
}

// =============================================================================
// Range — borrowing range iterator
// =============================================================================

/// Iterator over `(&K, &V)` pairs within a key range.
///
/// Does not track length — uses pointer equality for meeting detection.
pub struct Range<'a, K, V, const MAX_LEVEL: usize> {
    front: NodePtr<K, V, MAX_LEVEL>,
    back: NodePtr<K, V, MAX_LEVEL>,
    _marker: PhantomData<&'a ()>,
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> Iterator for Range<'a, K, V, MAX_LEVEL> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.front.is_null() {
            return None;
        }

        let ptr = self.front;
        // SAFETY: front is non-null and points to an occupied slot.
        let node = unsafe { &*node_deref(ptr) };

        if self.front == self.back {
            // Last element — mark both as exhausted
            self.front = ptr::null_mut();
            self.back = ptr::null_mut();
        } else {
            self.front = node.forward[0].get();
        }

        Some((&node.key, &node.value))
    }
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> DoubleEndedIterator
    for Range<'a, K, V, MAX_LEVEL>
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.back.is_null() {
            return None;
        }

        let ptr = self.back;
        // SAFETY: back is non-null and points to an occupied slot.
        let node = unsafe { &*node_deref(ptr) };

        if self.front == self.back {
            // Last element — mark both as exhausted
            self.front = ptr::null_mut();
            self.back = ptr::null_mut();
        } else {
            self.back = node.back.get();
        }

        Some((&node.key, &node.value))
    }
}

// =============================================================================
// RangeMut — mutable range iterator
// =============================================================================

/// Mutable iterator over `(&K, &mut V)` pairs within a key range.
///
/// Keys are immutable — changing them would violate sorted order.
pub struct RangeMut<'a, K, V, const MAX_LEVEL: usize> {
    front: NodePtr<K, V, MAX_LEVEL>,
    back: NodePtr<K, V, MAX_LEVEL>,
    _marker: PhantomData<&'a mut ()>,
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> Iterator for RangeMut<'a, K, V, MAX_LEVEL> {
    type Item = (&'a K, &'a mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.front.is_null() {
            return None;
        }

        let ptr = self.front;
        // SAFETY: front is non-null and occupied. &mut prevents aliasing.
        let node = unsafe { &mut *node_deref_mut(ptr) };

        if self.front == self.back {
            self.front = ptr::null_mut();
            self.back = ptr::null_mut();
        } else {
            self.front = node.forward[0].get();
        }

        Some((&node.key, &mut node.value))
    }
}

impl<'a, K: 'a, V: 'a, const MAX_LEVEL: usize> DoubleEndedIterator
    for RangeMut<'a, K, V, MAX_LEVEL>
{
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.back.is_null() {
            return None;
        }

        let ptr = self.back;
        // SAFETY: back is non-null and occupied. &mut prevents aliasing.
        let node = unsafe { &mut *node_deref_mut(ptr) };

        if self.front == self.back {
            self.front = ptr::null_mut();
            self.back = ptr::null_mut();
        } else {
            self.back = node.back.get();
        }

        Some((&node.key, &mut node.value))
    }
}

// =============================================================================
// Cursor
// =============================================================================

/// Cursor for positional traversal with O(1) removal.
///
/// Tracks predecessor nodes at each level so removal at the current position
/// doesn't require re-searching.
///
/// # Example
///
/// ```ignore
/// let mut cursor = skip_list.cursor_front();
/// while cursor.advance() {
///     if *cursor.value().unwrap() > threshold {
///         let (k, v) = cursor.remove().unwrap();
///         // cursor auto-advances to next
///     }
/// }
/// ```
pub struct Cursor<
    'a,
    K: Ord,
    V: 'static,
    A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
    const MAX_LEVEL: usize,
    const RATIO: u32,
> {
    list: &'a mut SkipList<K, V, A, MAX_LEVEL, RATIO>,
    current: NodePtr<K, V, MAX_LEVEL>,
    update: [NodePtr<K, V, MAX_LEVEL>; MAX_LEVEL],
    started: bool,
    past_end: bool,
}

impl<
        K: Ord,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > Cursor<'_, K, V, A, MAX_LEVEL, RATIO>
{
    /// Returns a reference to the current key, or `None` if not positioned.
    #[inline]
    pub fn key(&self) -> Option<&K> {
        if self.current.is_null() {
            return None;
        }
        // SAFETY: current is non-null, in the skip list
        Some(unsafe { &(*node_deref(self.current)).key })
    }

    /// Returns a reference to the current value, or `None` if not positioned.
    #[inline]
    pub fn value(&self) -> Option<&V> {
        if self.current.is_null() {
            return None;
        }
        // SAFETY: current is non-null, in the skip list
        Some(unsafe { &(*node_deref(self.current)).value })
    }

    /// Returns a mutable reference to the current value, or `None` if not positioned.
    #[inline]
    pub fn value_mut(&mut self) -> Option<&mut V> {
        if self.current.is_null() {
            return None;
        }
        // SAFETY: current is non-null, &mut self prevents aliasing
        Some(unsafe { &mut (*node_deref_mut(self.current)).value })
    }

    /// Advances the cursor to the next element.
    ///
    /// Returns `true` if the cursor is now at a valid element.
    #[inline]
    pub fn advance(&mut self) -> bool {
        if !self.started {
            self.started = true;
            self.past_end = false;
            self.current = self.list.head[0];
            if self.current.is_null() {
                self.past_end = true;
            }
            return !self.current.is_null();
        }

        if self.current.is_null() {
            return false;
        }

        // Update predecessor tracking at levels this node participates in
        let node = unsafe { &*node_deref(self.current) };
        let node_level = node.level.get() as usize;
        for i in 0..=node_level {
            self.update[i] = self.current;
        }

        self.current = node.forward[0].get();
        if self.current.is_null() {
            self.past_end = true;
        }
        !self.current.is_null()
    }

    /// Moves the cursor to the previous element.
    ///
    /// Returns `true` if the cursor is now at a valid element.
    ///
    /// **Note:** Moving backward does NOT update the predecessor array.
    /// `remove()` after `advance_back()` will re-search internally.
    #[inline]
    pub fn advance_back(&mut self) -> bool {
        if self.current.is_null() {
            if self.past_end {
                self.past_end = false;
                self.current = self.list.tail;
                return !self.current.is_null();
            }
            return false;
        }

        let back = unsafe { &*node_deref(self.current) }.back.get();
        self.current = back;
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
        let node = unsafe { &*node_deref(ptr) };
        let node_level = node.level.get();
        let next = node.forward[0].get();

        // Verify update array is current. After advance_back(), it may be stale.
        let mut update = self.update;
        let expected_prev = node.back.get();
        if update[0] != expected_prev {
            self.list
                .search(unsafe { &(*node_deref(ptr)).key }, &mut update);
        }

        self.list.unlink_at_levels(ptr, node_level, &update);
        self.list.update_tail_and_level(ptr, &update);
        self.list.len -= 1;

        // Advance cursor to next
        self.current = next;

        // SAFETY: ptr was in the skip list, now unlinked.
        let slot = unsafe { Slot::from_ptr(ptr) };
        let node = unsafe { A::take(slot) };
        Some(node.into_data())
    }
}

// =============================================================================
// Drain
// =============================================================================

/// Draining iterator that removes and returns all key-value pairs
/// in sorted (ascending) order.
///
/// When dropped, any remaining nodes are freed via the allocator.
pub struct Drain<
    'a,
    K: Ord,
    V: 'static,
    A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
    const MAX_LEVEL: usize,
    const RATIO: u32,
> {
    list: &'a mut SkipList<K, V, A, MAX_LEVEL, RATIO>,
}

impl<
        K: Ord,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > Iterator for Drain<'_, K, V, A, MAX_LEVEL, RATIO>
{
    type Item = (K, V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.list.pop_first()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.list.len(), Some(self.list.len()))
    }
}

impl<
        K: Ord,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > ExactSizeIterator for Drain<'_, K, V, A, MAX_LEVEL, RATIO>
{
}

impl<
        K: Ord,
        V: 'static,
        A: Alloc<Item = SkipNode<K, V, MAX_LEVEL>>,
        const MAX_LEVEL: usize,
        const RATIO: u32,
    > Drop for Drain<'_, K, V, A, MAX_LEVEL, RATIO>
{
    fn drop(&mut self) {
        self.list.clear();
    }
}
