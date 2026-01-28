//! Growable slab allocator with Entry-based access.
//!
//! [`Slab`] grows by adding fixed-size chunks. No copying occurs during growth,
//! providing consistent tail latency even when capacity is exceeded.
//!
//! # Example
//!
//! ```
//! use nexus_slab::Slab;
//!
//! let slab = Slab::new();
//!
//! // Grows automatically
//! let entry = slab.insert(42);
//! assert_eq!(*entry.get(), 42);
//! ```
//!
//! # Builder API
//!
//! ```
//! use nexus_slab::Slab;
//!
//! let slab: Slab<u64> = Slab::builder()
//!     .chunk_capacity(8192)
//!     .reserve(100_000)
//!     .build();
//! ```

use std::cell::{Cell, UnsafeCell};
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};
use std::rc::Rc;

use crate::bounded::{BoundedSlabInner, Entry, Ref, RefMut};
use crate::shared::{SlotCell, SLOT_NONE};
use crate::Key;

// =============================================================================
// Constants
// =============================================================================

/// Sentinel for chunk freelist
const CHUNK_NONE: u32 = u32::MAX;

/// Default chunk capacity for growable slab
const DEFAULT_CHUNK_CAPACITY: usize = 4096;

// =============================================================================
// SlabBuilder
// =============================================================================

/// Builder for configuring a growable [`Slab`].
///
/// # Example
///
/// ```
/// use nexus_slab::Slab;
///
/// let slab: Slab<u64> = Slab::builder()
///     .chunk_capacity(8192)
///     .reserve(100_000)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct SlabBuilder<T> {
    chunk_capacity: usize,
    reserve: usize,
    _marker: PhantomData<T>,
}

impl<T> Default for SlabBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SlabBuilder<T> {
    /// Creates a new builder with default settings.
    pub fn new() -> Self {
        Self {
            chunk_capacity: DEFAULT_CHUNK_CAPACITY,
            reserve: 0,
            _marker: PhantomData,
        }
    }

    /// Sets the capacity of each internal chunk.
    ///
    /// Rounded up to the next power of two. Default: 4096.
    pub fn chunk_capacity(mut self, capacity: usize) -> Self {
        self.chunk_capacity = capacity;
        self
    }

    /// Pre-allocates space for at least this many items.
    pub fn reserve(mut self, count: usize) -> Self {
        self.reserve = count;
        self
    }

    /// Builds the slab.
    pub fn build(self) -> Slab<T> {
        let slab = Slab::with_chunk_capacity(self.chunk_capacity);
        while slab.capacity() < self.reserve {
            slab.grow();
        }
        slab
    }
}

// =============================================================================
// ChunkEntry
// =============================================================================

/// Internal wrapper for a chunk in the growable slab.
struct ChunkEntry<T> {
    inner: Rc<BoundedSlabInner<T>>,
    next_with_space: Cell<u32>,
}

// =============================================================================
// SlabInner
// =============================================================================

/// Internal state for the growable slab.
struct SlabInner<T> {
    chunks: UnsafeCell<Vec<ChunkEntry<T>>>,
    chunk_shift: u32,
    chunk_mask: u32,
    head_with_space: Cell<u32>,
    chunk_capacity: u32,
}

impl<T> SlabInner<T> {
    fn with_chunk_capacity(chunk_capacity: usize) -> Self {
        assert!(chunk_capacity > 0, "chunk_capacity must be non-zero");
        assert!(chunk_capacity <= 1 << 30, "chunk_capacity exceeds maximum");

        let chunk_capacity = chunk_capacity.next_power_of_two() as u32;
        let chunk_shift = chunk_capacity.trailing_zeros();
        let chunk_mask = chunk_capacity - 1;

        Self {
            chunks: UnsafeCell::new(Vec::new()),
            chunk_shift,
            chunk_mask,
            head_with_space: Cell::new(CHUNK_NONE),
            chunk_capacity,
        }
    }

    /// Computes len from all chunks.
    fn len(&self) -> usize {
        self.chunks()
            .iter()
            .map(|c| c.inner.len.get() as usize)
            .sum()
    }

    #[inline]
    fn decode(&self, index: u32) -> (u32, u32) {
        let chunk_idx = index >> self.chunk_shift;
        let local_idx = index & self.chunk_mask;
        (chunk_idx, local_idx)
    }

    #[inline]
    fn encode(&self, chunk_idx: u32, local_idx: u32) -> u32 {
        (chunk_idx << self.chunk_shift) | local_idx
    }

    #[inline]
    fn chunks(&self) -> &Vec<ChunkEntry<T>> {
        unsafe { &*self.chunks.get() }
    }

    #[inline]
    #[allow(clippy::mut_from_ref)]
    fn chunks_mut(&self) -> &mut Vec<ChunkEntry<T>> {
        unsafe { &mut *self.chunks.get() }
    }

    fn chunk(&self, chunk_idx: u32) -> &ChunkEntry<T> {
        debug_assert!((chunk_idx as usize) < self.chunks().len());
        unsafe { self.chunks().get_unchecked(chunk_idx as usize) }
    }

    fn grow(&self) {
        let chunk_idx = self.chunks().len() as u32;
        let inner = Rc::new(BoundedSlabInner::with_capacity(self.chunk_capacity));

        let entry = ChunkEntry {
            inner,
            next_with_space: Cell::new(self.head_with_space.get()),
        };

        self.chunks_mut().push(entry);
        self.head_with_space.set(chunk_idx);
    }

    fn capacity(&self) -> usize {
        self.chunks().len() * self.chunk_capacity as usize
    }
}

// =============================================================================
// Slab (growable)
// =============================================================================

/// A growable slab allocator with Entry-based access.
///
/// Grows by adding fixed-size chunks. No copying occurs during growth.
/// Entries remain valid across growth operations.
///
/// # Example
///
/// ```
/// use nexus_slab::Slab;
///
/// let slab = Slab::new();
/// let entry = slab.insert(42);
/// assert_eq!(*entry.get(), 42);
/// ```
pub struct Slab<T> {
    inner: Rc<SlabInner<T>>,
}

impl<T> Slab<T> {
    /// Creates a new empty slab with default settings.
    ///
    /// Uses a chunk capacity of 4096 slots. No memory is allocated
    /// until the first insert.
    pub fn new() -> Self {
        Self::with_chunk_capacity(DEFAULT_CHUNK_CAPACITY)
    }

    /// Creates a new slab with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::builder().reserve(capacity).build()
    }

    /// Returns a builder for configuring a slab.
    pub fn builder() -> SlabBuilder<T> {
        SlabBuilder::new()
    }

    /// Creates a new slab with the specified chunk capacity.
    ///
    /// Chunk capacity is rounded up to the next power of two.
    pub fn with_chunk_capacity(chunk_capacity: usize) -> Self {
        Self {
            inner: Rc::new(SlabInner::with_chunk_capacity(chunk_capacity)),
        }
    }

    /// Returns the number of occupied slots.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if no slots are occupied.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }

    /// Returns the total capacity across all allocated chunks.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Returns the number of allocated chunks.
    #[inline]
    pub fn num_chunks(&self) -> usize {
        self.inner.chunks().len()
    }

    /// Allocates a new chunk.
    fn grow(&self) {
        self.inner.grow();
    }

    /// Inserts a value, returning an Entry handle.
    ///
    /// Grows automatically if needed.
    pub fn insert(&self, value: T) -> Entry<T> {
        // Ensure we have space
        if self.inner.head_with_space.get() == CHUNK_NONE {
            self.inner.grow();
        }

        let chunk_idx = self.inner.head_with_space.get();
        let chunk = self.inner.chunk(chunk_idx);
        let chunk_inner = &chunk.inner;

        let free_head = chunk_inner.free_head.get();
        debug_assert!(free_head != SLOT_NONE);

        let slot = chunk_inner.slot(free_head);
        let next_free = slot.next_free();

        chunk_inner.free_head.set(next_free);
        chunk_inner.len.set(chunk_inner.len.get() + 1);

        // Update chunk freelist if this chunk is now full
        if chunk_inner.is_full() {
            self.inner.head_with_space.set(chunk.next_with_space.get());
        }

        // Write value and mark occupied
        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_occupied();

        let global_idx = self.inner.encode(chunk_idx, free_head);

        Entry {
            slab: Rc::downgrade(chunk_inner),
            slot_ptr: slot as *const SlotCell<T>,
            index: global_idx,
        }
    }

    /// Inserts with access to the Entry before the value exists.
    pub fn insert_with<F>(&self, f: F) -> Entry<T>
    where
        F: FnOnce(Entry<T>) -> T,
    {
        if self.inner.head_with_space.get() == CHUNK_NONE {
            self.inner.grow();
        }

        let chunk_idx = self.inner.head_with_space.get();
        let chunk = self.inner.chunk(chunk_idx);
        let chunk_inner = &chunk.inner;

        let free_head = chunk_inner.free_head.get();
        let slot = chunk_inner.slot(free_head);
        let next_free = slot.next_free();

        chunk_inner.free_head.set(next_free);
        chunk_inner.len.set(chunk_inner.len.get() + 1);

        if chunk_inner.is_full() {
            self.inner.head_with_space.set(chunk.next_with_space.get());
        }

        let global_idx = self.inner.encode(chunk_idx, free_head);

        let entry = Entry {
            slab: Rc::downgrade(chunk_inner),
            slot_ptr: slot as *const SlotCell<T>,
            index: global_idx,
        };

        let value = f(entry.clone());

        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_occupied();

        entry
    }

    /// Creates an Entry from a key.
    ///
    /// Returns `None` if the key is out of bounds or the slot is vacant.
    pub fn entry(&self, key: Key) -> Option<Entry<T>> {
        let index = key.index();
        let (chunk_idx, local_idx) = self.inner.decode(index);

        if (chunk_idx as usize) >= self.inner.chunks().len() {
            return None;
        }

        let chunk = self.inner.chunk(chunk_idx);
        let chunk_inner = &chunk.inner;

        if local_idx >= chunk_inner.capacity {
            return None;
        }

        let slot = chunk_inner.slot(local_idx);
        if slot.is_vacant() {
            return None;
        }

        Some(Entry {
            slab: Rc::downgrade(chunk_inner),
            slot_ptr: slot as *const SlotCell<T>,
            index,
        })
    }

    /// Reserves a slot without filling it, returning a [`SlabVacantEntry`].
    ///
    /// The `SlabVacantEntry` can be used to get the key before constructing
    /// the value, then fill the slot via [`SlabVacantEntry::insert`].
    ///
    /// If the `SlabVacantEntry` is dropped without calling `insert`, the slot
    /// is automatically returned to the freelist.
    ///
    /// Unlike [`BoundedSlab::vacant_entry`](crate::BoundedSlab::vacant_entry),
    /// this never fails - the slab grows if needed.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::Slab;
    ///
    /// let slab: Slab<String> = Slab::new();
    ///
    /// let vacant = slab.vacant_entry();
    /// let key = vacant.key();
    ///
    /// let entry = vacant.insert(format!("item-{}", key.index()));
    /// assert!(entry.get().starts_with("item-"));
    /// ```
    pub fn vacant_entry(&self) -> SlabVacantEntry<T> {
        // Ensure we have space
        if self.inner.head_with_space.get() == CHUNK_NONE {
            self.inner.grow();
        }

        let chunk_idx = self.inner.head_with_space.get();
        let chunk = self.inner.chunk(chunk_idx);
        let chunk_inner = &chunk.inner;

        let free_head = chunk_inner.free_head.get();
        let slot = chunk_inner.slot(free_head);
        let next_free = slot.next_free();

        chunk_inner.free_head.set(next_free);
        chunk_inner.len.set(chunk_inner.len.get() + 1);

        // Check if chunk became full after our reservation
        let chunk_became_full = chunk_inner.is_full();
        if chunk_became_full {
            self.inner.head_with_space.set(chunk.next_with_space.get());
        }

        let global_idx = self.inner.encode(chunk_idx, free_head);

        SlabVacantEntry {
            chunk_inner: Rc::downgrade(chunk_inner),
            slab_inner: Rc::downgrade(&self.inner),
            slot_ptr: slot as *const SlotCell<T>,
            local_index: free_head,
            global_index: global_idx,
            chunk_idx,
            chunk_became_full,
            _marker: PhantomData,
        }
    }

    /// Removes a value via its Entry handle.
    ///
    /// This is faster than [`Entry::remove`] because it skips the
    /// `Weak::upgrade()` liveness check.
    ///
    /// # Panics
    ///
    /// Panics if the slot is vacant or borrowed.
    #[inline]
    #[allow(clippy::needless_pass_by_value)]
    pub fn remove(&self, entry: Entry<T>) -> T {
        self.try_remove(entry).expect("slot is vacant or borrowed")
    }

    /// Removes a value via its Entry handle, returning `None` if invalid.
    ///
    /// This is the non-panicking version of [`remove`](Self::remove).
    /// Returns `None` if the slot is vacant or currently borrowed.
    ///
    /// This is faster than [`Entry::try_remove`] because it skips the
    /// `Weak::upgrade()` liveness check - the slab already has the `Rc`.
    #[inline]
    #[allow(clippy::needless_pass_by_value)]
    pub fn try_remove(&self, entry: Entry<T>) -> Option<T> {
        let slot = unsafe { &*entry.slot_ptr };

        if !slot.is_available() {
            return None;
        }

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        // Decode chunk and local index
        let (chunk_idx, local_idx) = self.inner.decode(entry.index);
        let chunk = self.inner.chunk(chunk_idx);
        let chunk_inner = &chunk.inner;

        // Was chunk full before this remove?
        let was_full = chunk_inner.is_full();

        // Update chunk freelist
        let free_head = chunk_inner.free_head.get();
        slot.set_vacant(free_head);
        chunk_inner.free_head.set(local_idx);
        chunk_inner.len.set(chunk_inner.len.get() - 1);

        // If chunk was full, add it back to the slab's chunk freelist
        if was_full {
            chunk.next_with_space.set(self.inner.head_with_space.get());
            self.inner.head_with_space.set(chunk_idx);
        }

        Some(value)
    }

    /// Removes all values from the slab.
    ///
    /// Chunks are not deallocated.
    pub fn clear(&self) {
        if self.inner.len() == 0 {
            return;
        }

        let chunks = self.inner.chunks();
        for (i, chunk) in chunks.iter().enumerate() {
            let chunk_inner = &chunk.inner;

            for j in 0..chunk_inner.capacity {
                let slot = chunk_inner.slot(j);
                if slot.is_occupied() {
                    unsafe {
                        std::ptr::drop_in_place((*slot.value.get()).as_mut_ptr());
                    }
                }
                let next = if j + 1 < chunk_inner.capacity {
                    j + 1
                } else {
                    SLOT_NONE
                };
                slot.set_vacant(next);
            }

            chunk_inner.len.set(0);
            chunk_inner.free_head.set(0);

            // Rebuild chunk freelist
            let next_chunk = if i + 1 < chunks.len() {
                (i + 1) as u32
            } else {
                CHUNK_NONE
            };
            chunk.next_with_space.set(next_chunk);
        }

        if !chunks.is_empty() {
            self.inner.head_with_space.set(0);
        }
    }

    // =========================================================================
    // Key-based access (for collections compatibility)
    // =========================================================================

    /// Returns `true` if the key refers to an occupied slot.
    #[inline]
    pub fn contains_key(&self, key: Key) -> bool {
        let index = key.index();
        let (chunk_idx, local_idx) = self.inner.decode(index);

        if (chunk_idx as usize) >= self.inner.chunks().len() {
            return false;
        }

        let chunk = self.inner.chunk(chunk_idx);
        if local_idx >= chunk.inner.capacity {
            return false;
        }

        chunk.inner.slot(local_idx).is_occupied()
    }

    /// Alias for [`contains_key`](Self::contains_key) for API compatibility.
    #[inline]
    pub fn contains(&self, key: Key) -> bool {
        self.contains_key(key)
    }

    /// Returns a tracked reference to the value at `key`.
    ///
    /// The returned [`Ref`] guard participates in runtime borrow tracking,
    /// preventing conflicting access while the guard is held.
    #[inline]
    pub fn get(&self, key: Key) -> Option<Ref<T>> {
        let index = key.index();
        let (chunk_idx, local_idx) = self.inner.decode(index);

        if (chunk_idx as usize) >= self.inner.chunks().len() {
            return None;
        }

        let chunk = self.inner.chunk(chunk_idx);
        if local_idx >= chunk.inner.capacity {
            return None;
        }

        let slot = chunk.inner.slot(local_idx);
        if !slot.is_available() {
            return None;
        }

        slot.set_borrowed();
        Some(Ref {
            _slab: Rc::clone(&chunk.inner),
            slot_ptr: slot as *const SlotCell<T>,
        })
    }

    /// Returns a tracked mutable reference to the value at `key`.
    ///
    /// The returned [`RefMut`] guard participates in runtime borrow tracking,
    /// preventing conflicting access while the guard is held.
    #[inline]
    pub fn get_mut(&self, key: Key) -> Option<RefMut<T>> {
        let index = key.index();
        let (chunk_idx, local_idx) = self.inner.decode(index);

        if (chunk_idx as usize) >= self.inner.chunks().len() {
            return None;
        }

        let chunk = self.inner.chunk(chunk_idx);
        if local_idx >= chunk.inner.capacity {
            return None;
        }

        let slot = chunk.inner.slot(local_idx);
        if !slot.is_available() {
            return None;
        }

        slot.set_borrowed();
        Some(RefMut {
            _slab: Rc::clone(&chunk.inner),
            slot_ptr: slot as *const SlotCell<T>,
        })
    }

    // =========================================================================
    // Untracked access (unsafe - bypasses borrow tracking)
    // =========================================================================

    /// Returns an untracked reference to the value at `key`.
    ///
    /// This bypasses runtime borrow tracking for performance. The validity
    /// of the key is still checked.
    ///
    /// # Safety
    ///
    /// Caller must ensure no conflicting Entry operations (remove, replace,
    /// get_mut) occur on this slot while the reference is live.
    #[inline]
    pub unsafe fn get_untracked(&self, key: Key) -> Option<&T> {
        let index = key.index();
        let (chunk_idx, local_idx) = self.inner.decode(index);

        if (chunk_idx as usize) >= self.inner.chunks().len() {
            return None;
        }

        let chunk = self.inner.chunk(chunk_idx);
        if local_idx >= chunk.inner.capacity {
            return None;
        }

        let slot = chunk.inner.slot(local_idx);
        if slot.is_vacant() {
            return None;
        }

        Some(unsafe { slot.value_ref() })
    }

    /// Returns an untracked mutable reference to the value at `key`.
    ///
    /// This bypasses runtime borrow tracking for performance. The validity
    /// of the key is still checked.
    ///
    /// # Safety
    ///
    /// Caller must ensure exclusive access and no conflicting Entry operations
    /// occur on this slot while the reference is live.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_untracked_mut(&self, key: Key) -> Option<&mut T> {
        let index = key.index();
        let (chunk_idx, local_idx) = self.inner.decode(index);

        if (chunk_idx as usize) >= self.inner.chunks().len() {
            return None;
        }

        let chunk = self.inner.chunk(chunk_idx);
        if local_idx >= chunk.inner.capacity {
            return None;
        }

        let slot = chunk.inner.slot(local_idx);
        if slot.is_vacant() {
            return None;
        }

        Some(unsafe { slot.value_mut() })
    }

    /// Returns an untracked reference without any checks.
    ///
    /// # Safety
    ///
    /// - The key must be valid and the slot must be occupied.
    /// - No conflicting Entry operations while reference is live.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key) -> &T {
        let index = key.index();
        let (chunk_idx, local_idx) = self.inner.decode(index);
        let chunk = self.inner.chunk(chunk_idx);
        unsafe { chunk.inner.slot(local_idx).value_ref() }
    }

    /// Returns an untracked mutable reference without any checks.
    ///
    /// # Safety
    ///
    /// - The key must be valid and the slot must be occupied.
    /// - Caller must ensure exclusive access.
    /// - No conflicting Entry operations while reference is live.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_unchecked_mut(&self, key: Key) -> &mut T {
        let index = key.index();
        let (chunk_idx, local_idx) = self.inner.decode(index);
        let chunk = self.inner.chunk(chunk_idx);
        unsafe { chunk.inner.slot(local_idx).value_mut() }
    }

    /// Returns an [`SlabUntrackedAccessor`] for Index/IndexMut syntax.
    ///
    /// # Safety
    ///
    /// While the accessor or any reference from it is live, caller must not
    /// perform Entry operations that could invalidate references.
    #[inline]
    pub unsafe fn untracked(&self) -> SlabUntrackedAccessor<'_, T> {
        SlabUntrackedAccessor(self)
    }

    /// Removes and returns the value at `key`.
    ///
    /// # Panics
    ///
    /// Panics if the key is invalid, the slot is vacant, or borrowed.
    #[inline]
    pub fn remove_by_key(&self, key: Key) -> T {
        self.try_remove_by_key(key)
            .expect("key invalid, slot vacant, or borrowed")
    }

    /// Removes a value by key, returning `None` if invalid.
    ///
    /// This is the non-panicking version of [`remove_by_key`](Self::remove_by_key).
    /// Returns `None` if the key is out of bounds, the slot is vacant, or borrowed.
    #[inline]
    pub fn try_remove_by_key(&self, key: Key) -> Option<T> {
        let index = key.index();
        let (chunk_idx, local_idx) = self.inner.decode(index);

        let chunks = self.inner.chunks();
        if (chunk_idx as usize) >= chunks.len() {
            return None;
        }

        let chunk = self.inner.chunk(chunk_idx);
        let chunk_inner = &chunk.inner;

        if local_idx >= chunk_inner.capacity {
            return None;
        }

        let slot = chunk_inner.slot(local_idx);
        if !slot.is_available() {
            return None;
        }

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        // Was chunk full before this remove?
        let was_full = chunk_inner.is_full();

        // Update chunk freelist
        let free_head = chunk_inner.free_head.get();
        slot.set_vacant(free_head);
        chunk_inner.free_head.set(local_idx);
        chunk_inner.len.set(chunk_inner.len.get() - 1);

        // If chunk was full, add it back to the slab's chunk freelist
        if was_full {
            chunk.next_with_space.set(self.inner.head_with_space.get());
            self.inner.head_with_space.set(chunk_idx);
        }

        Some(value)
    }

    /// Removes a value by key without bounds or occupancy checks.
    ///
    /// # Safety
    ///
    /// The key must be valid and the slot must be occupied.
    #[inline]
    pub unsafe fn remove_unchecked_by_key(&self, key: Key) -> T {
        let index = key.index();
        let (chunk_idx, local_idx) = self.inner.decode(index);

        let chunk = self.inner.chunk(chunk_idx);
        let chunk_inner = &chunk.inner;
        let slot = chunk_inner.slot(local_idx);

        debug_assert!(!slot.is_vacant(), "remove_unchecked_by_key on vacant slot");

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let was_full = chunk_inner.is_full();

        let free_head = chunk_inner.free_head.get();
        slot.set_vacant(free_head);
        chunk_inner.free_head.set(local_idx);
        chunk_inner.len.set(chunk_inner.len.get() - 1);

        if was_full {
            chunk.next_with_space.set(self.inner.head_with_space.get());
            self.inner.head_with_space.set(chunk_idx);
        }

        value
    }
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for Slab<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<T> fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            .field("len", &self.len())
            .field("capacity", &self.capacity())
            .field("num_chunks", &self.num_chunks())
            .finish()
    }
}

// =============================================================================
// SlabVacantEntry
// =============================================================================

/// A reserved but unfilled slot in a [`Slab`].
///
/// Created by [`Slab::vacant_entry`], this represents a slot that has been
/// claimed from the freelist but not yet filled with a value.
///
/// Use [`insert`](Self::insert) to fill the slot and get an [`Entry`] handle.
/// If dropped without calling `insert`, the slot is automatically returned
/// to the freelist.
///
/// # Example
///
/// ```
/// use nexus_slab::Slab;
///
/// let slab: Slab<String> = Slab::new();
///
/// let vacant = slab.vacant_entry();
/// let key = vacant.key();
///
/// let entry = vacant.insert(format!("slot-{}", key.index()));
/// assert_eq!(*entry.get(), format!("slot-{}", key.index()));
/// ```
pub struct SlabVacantEntry<T> {
    chunk_inner: std::rc::Weak<BoundedSlabInner<T>>,
    slab_inner: std::rc::Weak<SlabInner<T>>,
    slot_ptr: *const SlotCell<T>,
    local_index: u32,
    global_index: u32,
    chunk_idx: u32,
    chunk_became_full: bool,
    _marker: PhantomData<T>,
}

impl<T> SlabVacantEntry<T> {
    /// Returns the key this slot will have once filled.
    #[inline]
    pub fn key(&self) -> Key {
        Key::new(self.global_index)
    }

    /// Fills the slot with a value, returning an [`Entry`] handle.
    ///
    /// Consumes the `SlabVacantEntry`, preventing the slot from being
    /// returned to the freelist.
    ///
    /// # Panics
    ///
    /// Panics if the slab has been dropped while holding this `SlabVacantEntry`.
    #[inline]
    pub fn insert(self, value: T) -> Entry<T> {
        // Verify chunk is still alive
        let _chunk_inner = self
            .chunk_inner
            .upgrade()
            .expect("slab dropped while holding SlabVacantEntry");

        let slot = unsafe { &*self.slot_ptr };

        // Write the value and mark occupied
        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_occupied();

        // Create Entry before forgetting self
        let entry = Entry {
            slab: self.chunk_inner.clone(),
            slot_ptr: self.slot_ptr,
            index: self.global_index,
        };

        // Prevent Drop from returning slot to freelist
        std::mem::forget(self);

        entry
    }
}

impl<T> Drop for SlabVacantEntry<T> {
    fn drop(&mut self) {
        // Return slot to chunk freelist if chunk still exists
        if let Some(chunk_inner) = self.chunk_inner.upgrade() {
            let slot = unsafe { &*self.slot_ptr };

            let free_head = chunk_inner.free_head.get();
            slot.set_vacant(free_head);
            chunk_inner.free_head.set(self.local_index);
            chunk_inner.len.set(chunk_inner.len.get() - 1);

            // If chunk was full after our reservation, re-add to slab freelist
            if self.chunk_became_full {
                if let Some(slab_inner) = self.slab_inner.upgrade() {
                    let chunk = slab_inner.chunk(self.chunk_idx);
                    chunk.next_with_space.set(slab_inner.head_with_space.get());
                    slab_inner.head_with_space.set(self.chunk_idx);
                }
            }
        }
    }
}

impl<T> fmt::Debug for SlabVacantEntry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlabVacantEntry")
            .field("global_index", &self.global_index)
            .field("chunk_idx", &self.chunk_idx)
            .field("local_index", &self.local_index)
            .field("alive", &(self.chunk_inner.strong_count() > 0))
            .finish()
    }
}

// =============================================================================
// SlabUntrackedAccessor
// =============================================================================

/// Wrapper enabling Index/IndexMut syntax with untracked access for [`Slab`].
///
/// See [`UntrackedAccessor`](crate::UntrackedAccessor) for safety requirements.
pub struct SlabUntrackedAccessor<'a, T>(&'a Slab<T>);

impl<T> Index<Key> for SlabUntrackedAccessor<'_, T> {
    type Output = T;

    #[inline]
    fn index(&self, key: Key) -> &T {
        // SAFETY: Caller of untracked() guarantees no conflicting Entry ops
        unsafe { self.0.get_unchecked(key) }
    }
}

impl<T> IndexMut<Key> for SlabUntrackedAccessor<'_, T> {
    #[inline]
    fn index_mut(&mut self, key: Key) -> &mut T {
        // SAFETY: Caller of untracked() guarantees no conflicting Entry ops
        unsafe { self.0.get_unchecked_mut(key) }
    }
}

impl<T> fmt::Debug for SlabUntrackedAccessor<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlabUntrackedAccessor")
            .field("len", &self.0.len())
            .field("capacity", &self.0.capacity())
            .finish()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let slab: Slab<u64> = Slab::new();
        assert_eq!(slab.len(), 0);
        assert!(slab.is_empty());
        assert_eq!(slab.capacity(), 0);
    }

    #[test]
    fn insert_grows() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        assert_eq!(slab.len(), 1);
        assert!(slab.capacity() > 0);
        assert_eq!(*entry.get(), 42);
    }

    #[test]
    fn with_capacity_preallocates() {
        let slab: Slab<u64> = Slab::with_capacity(10_000);
        assert!(slab.capacity() >= 10_000);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn builder_api() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(128).reserve(1000).build();

        assert!(slab.capacity() >= 1000);
    }

    #[test]
    fn insert_remove() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        assert_eq!(*entry.get(), 42);

        let value = entry.remove();
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn entry_key_roundtrip() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        let key = entry.key();

        let entry2 = slab.entry(key).unwrap();
        assert_eq!(*entry2.get(), 42);
    }

    #[test]
    fn multiple_chunks() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

        let mut entries = Vec::new();
        for i in 0..20u64 {
            entries.push(slab.insert(i));
        }

        assert!(slab.num_chunks() > 1);
        assert_eq!(slab.len(), 20);

        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(*entry.get(), i as u64);
        }
    }

    #[test]
    fn insert_with_self_referential() {
        struct Node {
            self_ref: Entry<Node>,
            data: u64,
        }

        let slab: Slab<Node> = Slab::new();

        let entry = slab.insert_with(|e| Node {
            self_ref: e.clone(),
            data: 42,
        });

        let node = entry.get();
        assert_eq!(node.data, 42);
    }

    #[test]
    fn clear_preserves_chunks() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

        for i in 0..20u64 {
            slab.insert(i);
        }

        let chunks_before = slab.num_chunks();
        slab.clear();

        assert_eq!(slab.len(), 0);
        assert_eq!(slab.num_chunks(), chunks_before);
    }

    #[test]
    fn stress_insert_remove() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(64).build();

        for i in 0..10_000u64 {
            let entry = slab.insert(i);
            assert_eq!(*entry.get(), i);
            entry.remove();
        }
    }

    #[test]
    fn slab_remove_fast_path() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        assert_eq!(slab.len(), 1);

        let value = slab.remove(entry);
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn slab_remove_fast_path_across_chunks() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

        // Fill multiple chunks
        let entries: Vec<_> = (0..20u64).map(|i| slab.insert(i)).collect();
        assert!(slab.num_chunks() > 1);

        // Remove all via fast path
        for (i, entry) in entries.into_iter().enumerate() {
            let value = slab.remove(entry);
            assert_eq!(value, i as u64);
        }

        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn slab_remove_fast_path_reuses_slot() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

        for i in 0..1000u64 {
            let entry = slab.insert(i);
            let value = slab.remove(entry);
            assert_eq!(value, i);
        }

        // Should only have used first chunk since we kept removing
        assert_eq!(slab.num_chunks(), 1);
    }

    #[test]
    fn key_based_access() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        let key = entry.key();

        assert!(slab.contains_key(key));

        // Safe tracked access returns Ref guard
        {
            let guard = slab.get(key).unwrap();
            assert_eq!(*guard, 42);
        }

        // Unsafe untracked access via SlabUntrackedAccessor for indexing
        unsafe {
            let accessor = slab.untracked();
            assert_eq!(accessor[key], 42);
        }

        let removed = slab.remove_by_key(key);
        assert_eq!(removed, 42);
        assert!(!slab.contains_key(key));
    }

    #[test]
    fn get_untracked_basic() {
        let slab: Slab<u64> = Slab::new();
        let entry = slab.insert(42);
        let key = entry.key();

        // Slab-level untracked
        unsafe {
            assert_eq!(slab.get_untracked(key), Some(&42));
            assert_eq!(slab.get_untracked_mut(key), Some(&mut 42));
        }

        // Entry-level untracked
        unsafe {
            assert_eq!(entry.get_untracked(), Some(&42));
            assert_eq!(entry.get_untracked_mut(), Some(&mut 42));
        }
    }

    #[test]
    fn untracked_accessor_basic() {
        let slab: Slab<u64> = Slab::new();
        let entry = slab.insert(42);
        let key = entry.key();

        unsafe {
            let accessor = slab.untracked();
            assert_eq!(accessor[key], 42);
        }

        unsafe {
            let mut accessor = slab.untracked();
            accessor[key] = 100;
        }

        assert_eq!(*entry.get(), 100);
    }

    #[test]
    fn tracked_get_blocks_double_borrow() {
        let slab: Slab<u64> = Slab::new();
        let entry = slab.insert(42);
        let key = entry.key();

        // Hold a tracked borrow via slab.get()
        let _guard = slab.get(key).unwrap();

        // Entry access should fail (slot is borrowed)
        assert!(entry.try_get().is_none());

        // Another slab.get() should also fail
        assert!(slab.get(key).is_none());
    }

    #[test]
    fn remove_unchecked_by_key_basic() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        let key = entry.key();

        let value = unsafe { slab.remove_unchecked_by_key(key) };
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn remove_unchecked_by_key_across_chunks() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

        // Fill multiple chunks
        let keys: Vec<Key> = (0..20u64).map(|i| slab.insert(i).key()).collect();
        assert!(slab.num_chunks() > 1);

        // Remove all via unchecked
        for (i, key) in keys.into_iter().enumerate() {
            let value = unsafe { slab.remove_unchecked_by_key(key) };
            assert_eq!(value, i as u64);
        }

        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn entry_is_valid() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        assert!(entry.is_valid());

        // Still valid even when borrowed
        let _r = entry.get();
        let entry2 = entry.clone();
        assert!(entry2.is_valid());

        drop(_r);

        // Invalid after remove
        entry.remove();
        assert!(!entry2.is_valid());
    }

    #[test]
    fn entry_equality() {
        let slab: Slab<u64> = Slab::new();

        let e1 = slab.insert(1);
        let e2 = slab.insert(2);
        let e1_clone = e1.clone();

        assert_eq!(e1, e1_clone);
        assert_ne!(e1, e2);
    }

    #[test]
    fn try_remove_success() {
        let slab: Slab<u64> = Slab::new();
        let entry = slab.insert(42);

        let value = slab.try_remove(entry);
        assert_eq!(value, Some(42));
        assert!(slab.is_empty());
    }

    #[test]
    fn try_remove_vacant_returns_none() {
        let slab: Slab<u64> = Slab::new();
        let entry = slab.insert(42);
        let entry_clone = entry.clone();

        // Remove via one handle
        slab.remove(entry);

        // Try to remove via the other - should return None
        let result = slab.try_remove(entry_clone);
        assert!(result.is_none());
    }

    #[test]
    fn try_remove_borrowed_returns_none() {
        let slab: Slab<u64> = Slab::new();
        let entry = slab.insert(42);
        let entry_clone = entry.clone();

        // Hold a borrow
        let _guard = entry.get();

        // Try to remove - should return None because borrowed
        let result = slab.try_remove(entry_clone);
        assert!(result.is_none());
    }

    #[test]
    fn try_remove_by_key_success() {
        let slab: Slab<u64> = Slab::new();
        let key = slab.insert(42).key();

        let value = slab.try_remove_by_key(key);
        assert_eq!(value, Some(42));
        assert!(slab.is_empty());
    }

    #[test]
    fn try_remove_by_key_invalid_returns_none() {
        let slab: Slab<u64> = Slab::new();

        // Invalid key (out of bounds - no chunks yet)
        let result = slab.try_remove_by_key(Key::from_raw(100));
        assert!(result.is_none());

        // Valid index but vacant
        let key = slab.insert(42).key();
        slab.remove_by_key(key);
        let result = slab.try_remove_by_key(key);
        assert!(result.is_none());
    }

    // =========================================================================
    // SlabVacantEntry tests
    // =========================================================================

    #[test]
    fn slab_vacant_entry_insert() {
        let slab: Slab<u64> = Slab::new();

        let vacant = slab.vacant_entry();
        let entry = vacant.insert(42);

        assert_eq!(*entry.get(), 42);
        assert_eq!(slab.len(), 1);
    }

    #[test]
    fn slab_vacant_entry_key_matches_final_entry() {
        let slab: Slab<u64> = Slab::new();

        let vacant = slab.vacant_entry();
        let key_before = vacant.key();
        let entry = vacant.insert(100);
        let key_after = entry.key();

        assert_eq!(key_before, key_after);
    }

    #[test]
    fn slab_vacant_entry_drop_returns_slot() {
        let slab: Slab<u64> = Slab::new();

        // Create and drop a vacant entry
        {
            let _vacant = slab.vacant_entry();
            assert_eq!(slab.len(), 1); // Slot is reserved
        }

        // After drop, slot is returned to freelist
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn slab_vacant_entry_reuses_freed_slot() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

        // Reserve and drop a slot
        let key1 = {
            let vacant = slab.vacant_entry();
            vacant.key()
        };

        // Next insert should reuse the same slot (LIFO)
        let entry = slab.insert(42);
        assert_eq!(entry.key(), key1);
    }

    #[test]
    fn slab_vacant_entry_grows_if_needed() {
        let slab: Slab<u64> = Slab::new();

        assert_eq!(slab.capacity(), 0);

        // vacant_entry should trigger growth
        let vacant = slab.vacant_entry();
        assert!(slab.capacity() > 0);

        // Clean up
        vacant.insert(42);
    }

    #[test]
    fn slab_vacant_entry_across_chunks() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

        // Fill first chunk
        let _entries: Vec<_> = (0..4u64).map(|i| slab.insert(i)).collect();
        assert_eq!(slab.num_chunks(), 1);

        // Next vacant_entry should create a new chunk
        let vacant = slab.vacant_entry();
        assert_eq!(slab.num_chunks(), 2);

        // Insert should work
        let entry = vacant.insert(100);
        assert_eq!(*entry.get(), 100);
    }

    #[test]
    fn slab_vacant_entry_drop_when_chunk_was_full() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

        // Fill first chunk completely (4 slots)
        let _entries: Vec<_> = (0..4u64).map(|i| slab.insert(i)).collect();
        assert_eq!(slab.num_chunks(), 1);

        // Get a vacant entry from second chunk
        let vacant = slab.vacant_entry();
        assert_eq!(slab.num_chunks(), 2);
        assert_eq!(slab.len(), 5); // 4 in chunk 0, 1 reserved in chunk 1

        // Drop the vacant entry - should return slot AND re-add chunk to freelist
        drop(vacant);
        assert_eq!(slab.len(), 4);

        // The next insert should use the freed slot in chunk 1
        // (not create a third chunk)
        let entry = slab.insert(999);
        assert_eq!(slab.num_chunks(), 2); // Still only 2 chunks
        assert_eq!(*entry.get(), 999);
    }

    #[test]
    fn slab_vacant_entry_debug() {
        let slab: Slab<u64> = Slab::new();
        let vacant = slab.vacant_entry();

        let debug = format!("{:?}", vacant);
        assert!(debug.contains("SlabVacantEntry"));
        assert!(debug.contains("global_index"));
        assert!(debug.contains("alive"));
    }
}
