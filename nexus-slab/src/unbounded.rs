//! Growable slab allocator with RAII Entry-based access.
//!
//! [`Slab`] is a leaked allocator that grows by adding fixed-size chunks.
//! No copying occurs during growth, providing consistent tail latency.
//!
//! # Design Philosophy
//!
//! Like [`BoundedSlab`](crate::BoundedSlab), this is an allocator, not a data structure:
//! - The allocator lives forever (leaked on creation)
//! - Handles are lightweight views (`Copy`, `!Send`)
//! - Entries own their slots (RAII - drop deallocates)
//!
//! # Example
//!
//! ```
//! use nexus_slab::Slab;
//!
//! let slab = Slab::leak(1000);
//!
//! // RAII entry - slot freed when entry drops
//! {
//!     let entry = slab.insert(42);
//!     assert_eq!(*entry.get(), 42);
//! } // entry drops, slot freed
//!
//! // Leak to keep data alive
//! let entry = slab.insert(100);
//! let key = entry.leak();
//!
//! assert_eq!(*slab.get(key).unwrap(), 100);
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
use std::mem::ManuallyDrop;
use std::ops::{Index, IndexMut};

use crate::bounded::BoundedSlabInner;
use crate::shared::{SlotCell, SLOT_NONE};
use crate::{Entry, FreeFn, FreeSlotVTable, Key, Ref, RefMut};

// =============================================================================
// Constants
// =============================================================================

/// Sentinel for chunk freelist
const CHUNK_NONE: u32 = u32::MAX;

/// Default chunk capacity for growable slab
const DEFAULT_CHUNK_CAPACITY: usize = 4096;

// =============================================================================
// Free Function
// =============================================================================

/// Returns a slot to the unbounded slab's freelist.
///
/// # Safety
///
/// - `key` must be valid for this slab
/// - `ctx` must be a valid `*mut SlabInner<T>`
/// - Slot must be occupied (caller responsible for dropping value first)
pub(crate) unsafe fn unbounded_slab_free<T>(key: Key, ctx: *mut ()) {
    let inner = ctx as *mut SlabInner<T>;

    // SAFETY: ctx is a valid SlabInner pointer, key is valid for this slab
    unsafe {
        let (chunk_idx, local_idx) = (*inner).decode(key.index());
        let chunk = (*inner).chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;
        let slot = chunk_inner.slot(local_idx);

        // Was chunk full before this free?
        let was_full = chunk_inner.is_full();

        // Return slot to freelist (LIFO)
        let free_head = chunk_inner.free_head.get();
        slot.set_vacant(free_head);
        chunk_inner.free_head.set(local_idx);
        chunk_inner.len.set(chunk_inner.len.get() - 1);

        // If chunk was full, add it back to slab's chunk freelist
        if was_full {
            chunk.next_with_space.set((*inner).head_with_space.get());
            (*inner).head_with_space.set(chunk_idx);
        }
    }
}

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

    /// Builds and leaks the slab.
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
    inner: ManuallyDrop<Box<BoundedSlabInner<T>>>,
    next_with_space: Cell<u32>,
}

// =============================================================================
// SlabInner
// =============================================================================

/// Internal state for the growable slab (leaked).
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

    #[inline]
    fn chunks(&self) -> &Vec<ChunkEntry<T>> {
        // SAFETY: Single-threaded access guaranteed by !Send
        unsafe { &*self.chunks.get() }
    }

    #[inline]
    #[allow(clippy::mut_from_ref)]
    fn chunks_mut(&self) -> &mut Vec<ChunkEntry<T>> {
        // SAFETY: Single-threaded access guaranteed by !Send
        unsafe { &mut *self.chunks.get() }
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
    fn chunk(&self, chunk_idx: u32) -> &ChunkEntry<T> {
        let chunks = self.chunks();
        debug_assert!((chunk_idx as usize) < chunks.len());
        unsafe { chunks.get_unchecked(chunk_idx as usize) }
    }

    fn grow(&self) {
        let chunks = self.chunks_mut();
        let chunk_idx = chunks.len() as u32;
        let inner = Box::new(BoundedSlabInner::with_capacity(self.chunk_capacity));

        let entry = ChunkEntry {
            inner: ManuallyDrop::new(inner),
            next_with_space: Cell::new(self.head_with_space.get()),
        };

        chunks.push(entry);
        self.head_with_space.set(chunk_idx);
    }

    fn capacity(&self) -> usize {
        self.chunks().len() * self.chunk_capacity as usize
    }
}

// Note: No Drop impl - this is leaked and never dropped

// =============================================================================
// Slab (growable)
// =============================================================================

/// A growable slab allocator with RAII Entry-based access.
///
/// Created via [`leak`](Self::leak) or [`builder().build()`](SlabBuilder::build),
/// which allocates and leaks the slab. The slab lives for `'static`.
///
/// # Thread Safety
///
/// `Slab` is `!Send` and `!Sync` - it uses raw pointers internally.
/// The slab must only be used from the thread that created it.
///
/// # Example
///
/// ```
/// use nexus_slab::Slab;
///
/// let slab = Slab::leak(1000);
/// let entry = slab.insert(42);
/// assert_eq!(*entry.get(), 42);
/// ```
#[derive(Clone, Copy)]
pub struct Slab<T> {
    ptr: *mut SlabInner<T>,
    vtable: *const FreeSlotVTable,
    _marker: PhantomData<*mut ()>, // Ensures !Send + !Sync
}

impl<T> Slab<T> {
    #[inline]
    fn inner(&self) -> &SlabInner<T> {
        // SAFETY: ptr is valid for 'static (leaked)
        unsafe { &*self.ptr }
    }

    /// Creates and leaks a slab, returning a `Copy` handle.
    ///
    /// The slab lives for `'static`. No memory is allocated until first insert.
    pub fn leak(initial_capacity: usize) -> Self {
        Self::builder().reserve(initial_capacity).build()
    }

    /// Creates an empty slab with default settings.
    ///
    /// No memory is allocated until first insert.
    pub fn new() -> Self {
        Self::with_chunk_capacity(DEFAULT_CHUNK_CAPACITY)
    }

    /// Creates a slab with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::builder().reserve(capacity).build()
    }

    /// Returns a builder for configuring a slab.
    pub fn builder() -> SlabBuilder<T> {
        SlabBuilder::new()
    }

    /// Creates a slab with the specified chunk capacity.
    ///
    /// Chunk capacity is rounded up to the next power of two.
    fn with_chunk_capacity(chunk_capacity: usize) -> Self {
        // 1. Allocate uninit box and leak immediately to get stable address
        let inner_uninit = Box::<SlabInner<T>>::new_uninit();
        let inner_ptr = Box::into_raw(inner_uninit) as *mut SlabInner<T>;

        // 2. Create and leak vtable (needs inner_ptr which is now valid forever)
        let vtable = Box::leak(Box::new(FreeSlotVTable {
            inner: inner_ptr as *mut (),
            free_fn: unbounded_slab_free::<T> as FreeFn,
        }));

        // 3. Initialize inner in place through the leaked pointer
        let inner_data = SlabInner::with_chunk_capacity(chunk_capacity);
        unsafe {
            inner_ptr.write(inner_data);
        }

        Self {
            ptr: inner_ptr,
            vtable,
            _marker: PhantomData,
        }
    }

    /// Returns the number of occupied slots.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner().len()
    }

    /// Returns `true` if no slots are occupied.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner().len() == 0
    }

    /// Returns the total capacity across all allocated chunks.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner().capacity()
    }

    /// Returns the number of allocated chunks.
    #[inline]
    pub fn num_chunks(&self) -> usize {
        self.inner().chunks().len()
    }

    /// Allocates a new chunk.
    fn grow(&self) {
        self.inner().grow();
    }

    // =========================================================================
    // Insert
    // =========================================================================

    /// Inserts a value, returning an RAII [`Entry`] handle.
    ///
    /// Grows automatically if needed. The returned entry owns the slot.
    /// When dropped, the slot is deallocated.
    pub fn insert(&self, value: T) -> Entry<T> {
        let inner = self.inner();

        // Ensure we have space
        if inner.head_with_space.get() == CHUNK_NONE {
            inner.grow();
        }

        let chunk_idx = inner.head_with_space.get();
        let chunk = inner.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;

        let free_head = chunk_inner.free_head.get();
        debug_assert!(free_head != SLOT_NONE);

        let slot = chunk_inner.slot(free_head);
        let next_free = slot.next_free();

        chunk_inner.free_head.set(next_free);
        chunk_inner.len.set(chunk_inner.len.get() + 1);

        // Update chunk freelist if this chunk is now full
        if chunk_inner.is_full() {
            inner.head_with_space.set(chunk.next_with_space.get());
        }

        // Write value and mark occupied
        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_occupied();

        let global_idx = inner.encode(chunk_idx, free_head);
        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();

        Entry::new(slot_ptr, self.vtable, Key::new(global_idx))
    }

    /// Inserts with access to the entry before the value exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    pub fn insert_with<F>(&self, f: F) -> Entry<T>
    where
        F: FnOnce(&Entry<T>) -> T,
    {
        let inner = self.inner();

        if inner.head_with_space.get() == CHUNK_NONE {
            inner.grow();
        }

        let chunk_idx = inner.head_with_space.get();
        let chunk = inner.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;

        let free_head = chunk_inner.free_head.get();
        let slot = chunk_inner.slot(free_head);
        let next_free = slot.next_free();

        chunk_inner.free_head.set(next_free);
        chunk_inner.len.set(chunk_inner.len.get() + 1);

        if chunk_inner.is_full() {
            inner.head_with_space.set(chunk.next_with_space.get());
        }

        let global_idx = inner.encode(chunk_idx, free_head);
        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();

        let entry = Entry::new(slot_ptr, self.vtable, Key::new(global_idx));

        let value = f(&entry);

        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_occupied();

        entry
    }

    /// Creates an RAII Entry from a key.
    ///
    /// Returns `None` if the key is out of bounds or the slot is vacant.
    ///
    /// **Warning**: The returned entry owns the slot. When dropped, the slot
    /// is deallocated. Only call this when you want to take ownership.
    pub fn entry(&self, key: Key) -> Option<Entry<T>> {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);

        if (chunk_idx as usize) >= inner.chunks().len() {
            return None;
        }

        let chunk = inner.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;

        if local_idx >= chunk_inner.capacity {
            return None;
        }

        let slot = chunk_inner.slot(local_idx);
        if slot.is_vacant() {
            return None;
        }

        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();
        Some(Entry::new(slot_ptr, self.vtable, key))
    }

    /// Reserves a slot without filling it, returning a [`SlabVacantEntry`].
    ///
    /// If dropped without calling `insert`, the slot is automatically
    /// returned to the freelist.
    pub fn vacant_entry(&self) -> SlabVacantEntry<T> {
        let inner = self.inner();

        // Ensure we have space
        if inner.head_with_space.get() == CHUNK_NONE {
            inner.grow();
        }

        let chunk_idx = inner.head_with_space.get();
        let chunk = inner.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;

        let free_head = chunk_inner.free_head.get();
        let slot = chunk_inner.slot(free_head);
        let next_free = slot.next_free();

        chunk_inner.free_head.set(next_free);
        chunk_inner.len.set(chunk_inner.len.get() + 1);

        // Check if chunk became full after our reservation
        let chunk_became_full = chunk_inner.is_full();
        if chunk_became_full {
            inner.head_with_space.set(chunk.next_with_space.get());
        }

        let global_idx = inner.encode(chunk_idx, free_head);

        SlabVacantEntry {
            ptr: self.ptr,
            vtable: self.vtable,
            key: Key::new(global_idx),
            chunk_idx,
            local_idx: free_head,
            chunk_became_full,
            consumed: false,
            _marker: PhantomData,
        }
    }

    // =========================================================================
    // Key-based access
    // =========================================================================

    /// Returns `true` if the key refers to an occupied slot.
    #[inline]
    pub fn contains_key(&self, key: Key) -> bool {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);

        if (chunk_idx as usize) >= inner.chunks().len() {
            return false;
        }

        let chunk = inner.chunk(chunk_idx);
        if local_idx >= chunk.inner.capacity {
            return false;
        }

        chunk.inner.slot(local_idx).is_occupied()
    }

    /// Alias for [`contains_key`](Self::contains_key).
    #[inline]
    pub fn contains(&self, key: Key) -> bool {
        self.contains_key(key)
    }

    /// Returns a tracked reference to the value at `key`.
    #[inline]
    pub fn get(&self, key: Key) -> Option<Ref<T>> {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);

        if (chunk_idx as usize) >= inner.chunks().len() {
            return None;
        }

        let chunk = inner.chunk(chunk_idx);
        if local_idx >= chunk.inner.capacity {
            return None;
        }

        let slot = chunk.inner.slot(local_idx);
        if !slot.is_available() {
            return None;
        }

        slot.set_borrowed();
        Some(Ref::new((slot as *const SlotCell<T>).cast_mut()))
    }

    /// Returns a tracked mutable reference to the value at `key`.
    #[inline]
    pub fn get_mut(&self, key: Key) -> Option<RefMut<T>> {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);

        if (chunk_idx as usize) >= inner.chunks().len() {
            return None;
        }

        let chunk = inner.chunk(chunk_idx);
        if local_idx >= chunk.inner.capacity {
            return None;
        }

        let slot = chunk.inner.slot(local_idx);
        if !slot.is_available() {
            return None;
        }

        slot.set_borrowed();
        Some(RefMut::new((slot as *const SlotCell<T>).cast_mut()))
    }

    /// Removes a value by key, bypassing RAII.
    ///
    /// Use this when you have a leaked key and want to deallocate.
    #[inline]
    pub fn remove_by_key(&self, key: Key) -> Option<T> {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);

        if (chunk_idx as usize) >= inner.chunks().len() {
            return None;
        }

        let chunk = inner.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;

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
            chunk.next_with_space.set(inner.head_with_space.get());
            inner.head_with_space.set(chunk_idx);
        }

        Some(value)
    }

    /// Alias for [`remove_by_key`](Self::remove_by_key).
    #[inline]
    pub fn try_remove_by_key(&self, key: Key) -> Option<T> {
        self.remove_by_key(key)
    }

    /// Removes all values from the slab.
    ///
    /// Chunks are not deallocated.
    pub fn clear(&self) {
        let inner = self.inner();

        if inner.len() == 0 {
            return;
        }

        for (i, chunk) in inner.chunks().iter().enumerate() {
            let chunk_inner = &*chunk.inner;

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
            let next_chunk = if i + 1 < inner.chunks().len() {
                (i + 1) as u32
            } else {
                CHUNK_NONE
            };
            chunk.next_with_space.set(next_chunk);
        }

        if !inner.chunks().is_empty() {
            inner.head_with_space.set(0);
        }
    }

    // =========================================================================
    // Unsafe access
    // =========================================================================

    /// Returns an untracked reference to the value at `key`.
    ///
    /// # Safety
    ///
    /// No concurrent mutable access to this slot may exist.
    #[inline]
    pub unsafe fn get_untracked(&self, key: Key) -> Option<&T> {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);

        if (chunk_idx as usize) >= inner.chunks().len() {
            return None;
        }

        let chunk = inner.chunk(chunk_idx);
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
    /// # Safety
    ///
    /// No concurrent access to this slot may exist.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_untracked_mut(&self, key: Key) -> Option<&mut T> {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);

        if (chunk_idx as usize) >= inner.chunks().len() {
            return None;
        }

        let chunk = inner.chunk(chunk_idx);
        if local_idx >= chunk.inner.capacity {
            return None;
        }

        let slot = chunk.inner.slot(local_idx);
        if slot.is_vacant() {
            return None;
        }

        Some(unsafe { slot.value_mut() })
    }

    /// Returns a reference without any checks.
    ///
    /// # Safety
    ///
    /// Key must be valid and slot must be occupied. No concurrent mutable access.
    #[inline]
    pub unsafe fn get_unchecked(&self, key: Key) -> &T {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);
        let chunk = inner.chunk(chunk_idx);
        unsafe { chunk.inner.slot(local_idx).value_ref() }
    }

    /// Returns a mutable reference without any checks.
    ///
    /// # Safety
    ///
    /// Key must be valid and slot must be occupied. No concurrent access.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_unchecked_mut(&self, key: Key) -> &mut T {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);
        let chunk = inner.chunk(chunk_idx);
        unsafe { chunk.inner.slot(local_idx).value_mut() }
    }

    /// Gets an accessor for Index/IndexMut syntax.
    ///
    /// # Safety
    ///
    /// While this accessor is live, no Entry operations may occur.
    #[inline]
    pub unsafe fn untracked(&self) -> SlabUntrackedAccessor<'_, T> {
        SlabUntrackedAccessor(self)
    }

    /// Removes a value by key without bounds or occupancy checks.
    ///
    /// # Safety
    ///
    /// The key must be valid and the slot must be occupied.
    #[inline]
    pub unsafe fn remove_unchecked_by_key(&self, key: Key) -> T {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);

        let chunk = inner.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;
        let slot = chunk_inner.slot(local_idx);

        debug_assert!(!slot.is_vacant(), "remove_unchecked_by_key on vacant slot");

        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let was_full = chunk_inner.is_full();

        let free_head = chunk_inner.free_head.get();
        slot.set_vacant(free_head);
        chunk_inner.free_head.set(local_idx);
        chunk_inner.len.set(chunk_inner.len.get() - 1);

        if was_full {
            chunk.next_with_space.set(inner.head_with_space.get());
            inner.head_with_space.set(chunk_idx);
        }

        value
    }
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self::new()
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
/// Created by [`Slab::vacant_entry`]. Fill with [`insert`](Self::insert)
/// or drop to return the slot to the freelist.
pub struct SlabVacantEntry<T> {
    ptr: *mut SlabInner<T>,
    vtable: *const FreeSlotVTable,
    key: Key,
    chunk_idx: u32,
    local_idx: u32,
    chunk_became_full: bool,
    consumed: bool,
    _marker: PhantomData<T>,
}

impl<T> SlabVacantEntry<T> {
    #[inline]
    fn inner(&self) -> &SlabInner<T> {
        // SAFETY: ptr is valid for 'static
        unsafe { &*self.ptr }
    }

    /// Returns the key this slot will have once filled.
    #[inline]
    pub fn key(&self) -> Key {
        self.key
    }

    /// Fills the slot with a value, returning an RAII [`Entry`].
    #[inline]
    pub fn insert(mut self, value: T) -> Entry<T> {
        // Scope the borrow of self to avoid conflict with consumed assignment
        let slot_ptr = {
            let inner = self.inner();
            let chunk = inner.chunk(self.chunk_idx);
            let slot = chunk.inner.slot(self.local_idx);

            unsafe {
                (*slot.value.get()).write(value);
            }
            slot.set_occupied();

            (slot as *const SlotCell<T>).cast_mut()
        };

        self.consumed = true;

        Entry::new(slot_ptr, self.vtable, self.key)
    }
}

impl<T> Drop for SlabVacantEntry<T> {
    fn drop(&mut self) {
        if !self.consumed {
            let inner = self.inner();
            let chunk = inner.chunk(self.chunk_idx);
            let chunk_inner = &*chunk.inner;
            let slot = chunk_inner.slot(self.local_idx);

            // Return slot to freelist
            let free_head = chunk_inner.free_head.get();
            slot.set_vacant(free_head);
            chunk_inner.free_head.set(self.local_idx);
            chunk_inner.len.set(chunk_inner.len.get() - 1);

            // If chunk was full after our reservation, re-add to slab freelist
            if self.chunk_became_full {
                chunk.next_with_space.set(inner.head_with_space.get());
                inner.head_with_space.set(self.chunk_idx);
            }
        }
    }
}

impl<T> fmt::Debug for SlabVacantEntry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlabVacantEntry")
            .field("key", &self.key)
            .finish()
    }
}

// =============================================================================
// SlabUntrackedAccessor
// =============================================================================

/// Wrapper enabling Index/IndexMut syntax with untracked access for [`Slab`].
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
// Additional helper for Slab (try_insert for API parity with BoundedSlab)
// =============================================================================

impl<T> Slab<T> {
    /// Inserts a value, returning an RAII [`Entry`] handle.
    ///
    /// This always succeeds (grows if needed), but the method name
    /// provides API parity with [`BoundedSlab::try_insert`](crate::BoundedSlab::try_insert).
    #[inline]
    pub fn try_insert(&self, value: T) -> Entry<T> {
        self.insert(value)
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
    fn insert_and_drop() {
        let slab: Slab<u64> = Slab::new();

        {
            let entry = slab.insert(42);
            assert_eq!(slab.len(), 1);
            assert_eq!(*entry.get(), 42);
        }

        // Entry dropped, slot freed
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn leak_keeps_data() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(100);
        let key = entry.leak();

        // Data still exists
        assert_eq!(slab.len(), 1);
        assert_eq!(*slab.get(key).unwrap(), 100);

        // Clean up via remove
        let value = slab.remove_by_key(key).unwrap();
        assert_eq!(value, 100);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn entry_from_key() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        let key = entry.leak();

        // Re-acquire RAII entry
        {
            let entry = slab.entry(key).unwrap();
            assert_eq!(*entry.get(), 42);
        }

        // Entry dropped, slot freed
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn multiple_chunks() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

        // Insert and leak to keep entries alive
        let mut keys = Vec::new();
        for i in 0..20u64 {
            let entry = slab.insert(i);
            keys.push(entry.leak());
        }

        assert!(slab.num_chunks() > 1);
        assert_eq!(slab.len(), 20);

        for (i, key) in keys.iter().enumerate() {
            assert_eq!(*slab.get(*key).unwrap(), i as u64);
        }
    }

    #[test]
    fn insert_with_self_referential() {
        let slab: Slab<(Key, u64)> = Slab::new();

        let entry = slab.insert_with(|e| (e.key(), 42));

        let (stored_key, value) = *entry.get();
        assert_eq!(stored_key, entry.key());
        assert_eq!(value, 42);
    }

    #[test]
    fn clear_preserves_chunks() {
        let slab: Slab<u64> = Slab::builder().chunk_capacity(4).build();

        for i in 0..20u64 {
            slab.insert(i).leak(); // Leak to keep data
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
    fn key_based_access() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        let key = entry.leak();

        assert!(slab.contains_key(key));

        // Safe tracked access returns Ref guard
        {
            let guard = slab.get(key).unwrap();
            assert_eq!(*guard, 42);
        }

        // Unsafe untracked access
        unsafe {
            let accessor = slab.untracked();
            assert_eq!(accessor[key], 42);
        }

        let removed = slab.remove_by_key(key).unwrap();
        assert_eq!(removed, 42);
        assert!(!slab.contains_key(key));
    }

    #[test]
    fn borrow_tracking() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);

        {
            let _ref1 = entry.get();
            // Second borrow should fail while first is held
            assert!(entry.try_get().is_none());
        }

        // After drop, borrow succeeds
        let _ref2 = entry.get();
    }

    #[test]
    fn handle_is_copy() {
        let slab = Slab::new();
        let slab2 = slab; // Copy
        let slab3 = slab; // Copy again

        let _e1 = slab.try_insert(1u64).leak();
        let _e2 = slab2.try_insert(2u64).leak();
        let _e3 = slab3.try_insert(3u64).leak();

        assert_eq!(slab.len(), 3);
    }

    #[test]
    fn remove_unchecked_by_key_basic() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        let key = entry.leak();

        let value = unsafe { slab.remove_unchecked_by_key(key) };
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn entry_is_valid() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        assert!(entry.is_valid());

        let key = entry.leak();

        // Remove via key makes the entry invalid
        slab.remove_by_key(key);

        // Can re-acquire entry but it's invalid
        assert!(slab.entry(key).is_none());
    }

    #[test]
    fn vacant_entry_insert() {
        let slab: Slab<u64> = Slab::new();

        let vacant = slab.vacant_entry();
        let entry = vacant.insert(42);

        assert_eq!(*entry.get(), 42);
        assert_eq!(slab.len(), 1);
    }

    #[test]
    fn vacant_entry_key_matches() {
        let slab: Slab<u64> = Slab::new();

        let vacant = slab.vacant_entry();
        let key_before = vacant.key();
        let entry = vacant.insert(100);
        let key_after = entry.key();

        assert_eq!(key_before, key_after);
    }

    #[test]
    fn vacant_entry_drop_returns_slot() {
        let slab: Slab<u64> = Slab::new();

        {
            let _vacant = slab.vacant_entry();
            assert_eq!(slab.len(), 1);
        }

        // After drop, slot is returned to freelist
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn replace() {
        let slab: Slab<u64> = Slab::new();
        let entry = slab.insert(42);

        let old = entry.replace(100);
        assert_eq!(old, 42);
        assert_eq!(*entry.get(), 100);
    }

    #[test]
    fn and_modify() {
        let slab: Slab<u64> = Slab::new();
        let entry = slab.insert(0);

        entry.and_modify(|v| *v += 1).and_modify(|v| *v *= 2);

        assert_eq!(*entry.get(), 2);
    }

    #[test]
    fn explicit_remove() {
        let slab: Slab<u64> = Slab::new();
        let entry = slab.insert(42);

        let value = entry.remove();
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }
}
