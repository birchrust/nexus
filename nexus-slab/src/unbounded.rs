//! Growable slab allocator with RAII Slot-based access.
//!
//! [`Slab`] is a leaked allocator that grows by adding fixed-size chunks.
//! No copying occurs during growth, providing consistent tail latency.
//!
//! # Design Philosophy
//!
//! Like [`bounded::Slab`](crate::bounded::Slab), this is an allocator, not a data structure:
//! - The allocator lives forever (leaked on creation)
//! - Handles are lightweight views (`Copy`, `!Send`)
//! - Entries own their slots (RAII - drop deallocates)
//!
//! # Example
//!
//! ```
//! use nexus_slab::unbounded::Slab;
//!
//! let slab = Slab::with_capacity(1000);
//!
//! // RAII entry - slot freed when entry drops
//! {
//!     let entry = slab.insert(42);
//!     assert_eq!(*entry.get(), 42);
//! } // entry drops, slot freed
//!
//! // Forget to keep data alive
//! let entry = slab.insert(100);
//! let key = entry.leak();
//!
//! // Access via key (unsafe - caller guarantees key validity)
//! // SAFETY: key was just returned from forget(), slot is occupied
//! assert_eq!(*unsafe { slab.get_by_key(key) }, 100);
//! ```
//!
//! # Builder API
//!
//! ```
//! use nexus_slab::{Builder, unbounded::Slab};
//!
//! let slab: Slab<u64> = Builder::default()
//!     .unbounded()
//!     .chunk_capacity(8192)
//!     .capacity(100_000)
//!     .build();
//! ```

use std::cell::{Cell, UnsafeCell};
use std::fmt;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::pin::Pin;

use crate::Key;
use crate::bounded::BoundedSlabInner;
use crate::shared::{SLOT_NONE, SlotCell};

// =============================================================================
// Constants
// =============================================================================

/// Sentinel for chunk freelist
const CHUNK_NONE: u32 = u32::MAX;

/// Default chunk capacity for growable slab
const DEFAULT_CHUNK_CAPACITY: usize = 4096;
// =============================================================================
// ChunkEntry
// =============================================================================

/// Internal wrapper for a chunk in the growable slab.
pub(crate) struct ChunkEntry<T> {
    inner: ManuallyDrop<Box<BoundedSlabInner<T>>>,
    next_with_space: Cell<u32>,
}

// =============================================================================
// SlabInner
// =============================================================================

/// Internal state for the growable slab (leaked).
pub(crate) struct SlabInner<T> {
    chunks: UnsafeCell<Vec<ChunkEntry<T>>>,
    chunk_shift: u32,
    chunk_mask: u32,
    head_with_space: Cell<u32>,
    chunk_capacity: u32,
    len: Cell<usize>,
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
            len: Cell::new(0),
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

    /// Returns the cached total length.
    #[inline]
    fn len(&self) -> usize {
        self.len.get()
    }

    #[inline]
    pub(crate) fn decode(&self, index: u32) -> (u32, u32) {
        let chunk_idx = index >> self.chunk_shift;
        let local_idx = index & self.chunk_mask;
        (chunk_idx, local_idx)
    }

    #[inline]
    fn encode(&self, chunk_idx: u32, local_idx: u32) -> u32 {
        (chunk_idx << self.chunk_shift) | local_idx
    }

    #[inline]
    pub(crate) fn chunk(&self, chunk_idx: u32) -> &ChunkEntry<T> {
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
// Slot
// =============================================================================

/// RAII handle to an occupied slot in an unbounded [`Slab`].
///
/// When dropped, the slot is deallocated and returned to the freelist.
/// Use [`leak()`](Self::leak) to keep the data alive without the slot handle.
///
/// # Size
///
/// 16 bytes: slot pointer (8) + inner pointer (8).
#[must_use = "dropping Slot deallocates the slot"]
pub struct Slot<T> {
    slot: *mut SlotCell<T>,
    inner: *mut SlabInner<T>,
}

impl<T> Slot<T> {
    /// Creates a new slot handle.
    #[inline]
    pub(crate) fn new(slot: *mut SlotCell<T>, inner: *mut SlabInner<T>) -> Self {
        Self { slot, inner }
    }

    #[inline]
    fn slot(&self) -> &SlotCell<T> {
        // SAFETY: Slot holds a valid slot pointer
        unsafe { &*self.slot }
    }

    #[inline]
    fn inner(&self) -> &SlabInner<T> {
        // SAFETY: Slot holds a valid inner pointer (leaked)
        unsafe { &*self.inner }
    }
}

// Core Slot methods as inherent (no trait import needed)
impl<T> Slot<T> {
    /// Returns the key for this slot.
    #[inline]
    pub fn key(&self) -> Key {
        Key::new(self.slot().key_from_stamp())
    }

    /// Leaks the slot, keeping the data alive and returning its key.
    ///
    /// After calling `leak()`, the slot remains occupied but has no Slot owner.
    /// Access the data via key-based methods (which are unsafe) or create a new
    /// Slot via [`Slab::slot()`].
    ///
    /// This is the [`Box::leak`]-style API. The slot's memory is "leaked" in the
    /// sense that automatic cleanup is disabled.
    #[inline]
    pub fn leak(self) -> Key {
        let key = self.key();
        std::mem::forget(self);
        key
    }

    /// Alias for [`leak()`](Self::leak) for backwards compatibility.
    #[inline]
    #[doc(hidden)]
    pub fn forget(self) -> Key {
        self.leak()
    }

    /// Returns a raw pointer to the value.
    ///
    /// The pointer is valid for the lifetime of the slab (which is `'static`).
    /// This is the [`Box::as_ptr`]-style API.
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        // SlotCell is repr(C): [stamp: 8][value: T]
        unsafe { (self.slot as *const u8).add(8) as *const T }
    }

    /// Returns a mutable raw pointer to the value.
    ///
    /// The pointer is valid for the lifetime of the slab (which is `'static`).
    /// This is the [`Box::as_mut_ptr`]-style API.
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        // SlotCell is repr(C): [stamp: 8][value: T]
        unsafe { (self.slot as *mut u8).add(8) as *mut T }
    }

    /// Returns a reference to the value.
    ///
    /// Slot ownership guarantees the slot is valid.
    #[inline]
    pub fn get(&self) -> &T {
        // SAFETY: Slot owns the slot. SlotCell is repr(C): [stamp: 8][value: T]
        // Go directly to value at offset 8, bypassing abstraction chain.
        unsafe { &*((self.slot as *const u8).add(8) as *const T) }
    }

    /// Returns a mutable reference to the value.
    ///
    /// Requires `&mut Slot` to ensure exclusive access.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: Slot owns the slot, &mut self ensures exclusivity.
        // SlotCell is repr(C): [stamp: 8][value: T]
        unsafe { &mut *((self.slot as *mut u8).add(8) as *mut T) }
    }

    /// Replaces the value, returning the old one.
    #[inline]
    pub fn replace(&mut self, value: T) -> T {
        // SAFETY: Slot owns the slot. SlotCell is repr(C): [stamp: 8][value: T]
        // Direct pointer access to value at offset 8.
        let value_ptr = unsafe { (self.slot as *mut u8).add(8) as *mut T };
        let old = unsafe { value_ptr.read() };
        unsafe { value_ptr.write(value) };
        old
    }

    /// Modifies the value in place. Returns self for chaining.
    #[inline]
    pub fn and_modify<F: FnOnce(&mut T)>(&mut self, f: F) -> &mut Self {
        f(self.get_mut());
        self
    }

    /// Returns `true` if the slot is still occupied.
    ///
    /// This should always return `true` for a properly-used Slot.
    /// Returns `false` only if the slot was incorrectly deallocated
    /// via unsafe key-based methods while this Slot existed.
    ///
    /// Useful for debug assertions to catch API misuse.
    #[inline]
    pub fn is_valid(&self) -> bool {
        // SAFETY: SlotCell is repr(C): [stamp: 8][value: T]
        // Read stamp directly at offset 0. Occupied = VACANT_BIT not set.
        let stamp = unsafe { *(self.slot as *const u64) };
        stamp & crate::shared::VACANT_BIT == 0
    }

    /// Extracts the value, returning it with a [`VacantSlot`] for the slot.
    ///
    /// Unlike drop, this keeps the slot reserved. The VacantSlot can be used
    /// to insert a new value into the same slot, or dropped to return the slot
    /// to the freelist.
    pub fn take(self) -> (T, VacantSlot<T>) {
        let slot = self.slot();
        let key = self.key();
        let inner = self.inner();

        // Decode once
        let (chunk_idx, local_idx) = inner.decode(key.index());
        let chunk = inner.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;

        // Check if chunk is currently full (before we free this slot)
        let chunk_became_full = chunk_inner.is_full();

        // SAFETY: Slot owns the slot, so it's valid
        let value = unsafe { (*slot.value.get()).assume_init_read() };

        let vacant = VacantSlot {
            inner: self.inner,
            key,
            chunk_idx,
            local_idx,
            chunk_became_full,
            consumed: false,
            _marker: PhantomData,
        };

        // Don't run Slot's Drop (which would deallocate)
        std::mem::forget(self);

        (value, vacant)
    }

    /// Returns a pinned reference to the value.
    ///
    /// This is safe because slab slots have stable addresses—chunks
    /// are leaked and never reallocate.
    #[inline]
    pub fn pin(&self) -> Pin<&T> {
        // SAFETY: Chunk memory is leaked and never moves
        unsafe { Pin::new_unchecked(self.get()) }
    }

    /// Returns a pinned mutable reference to the value.
    ///
    /// This is safe because slab slots have stable addresses—chunks
    /// are leaked and never reallocate.
    #[inline]
    pub fn pin_mut(&mut self) -> Pin<&mut T> {
        // SAFETY: Chunk memory is leaked and never moves
        unsafe { Pin::new_unchecked(self.get_mut()) }
    }

    /// Consumes the slot, returning the value and deallocating.
    ///
    /// The slot is returned to the freelist. This is the [`Box::into_inner`]-style API.
    #[inline]
    pub fn into_inner(self) -> T {
        // SAFETY: Slot owns the slot, so it's always valid
        unsafe { self.into_inner_unchecked() }
    }

    /// Alias for [`into_inner()`](Self::into_inner) for backwards compatibility.
    #[inline]
    #[doc(hidden)]
    pub fn remove(self) -> T {
        self.into_inner()
    }

    /// Consumes the slot without validity checks, returning the value.
    ///
    /// Currently identical to [`into_inner()`](Self::into_inner) since Slot ownership
    /// guarantees validity. Provided for API consistency with key-based methods.
    ///
    /// # Safety
    ///
    /// The slot must be valid (not previously consumed or taken).
    #[inline]
    pub unsafe fn into_inner_unchecked(self) -> T {
        let slot = self.slot();
        let inner = self.inner();

        // Read key from stamp and decode once
        let key_index = slot.key_from_stamp();
        let (chunk_idx, local_idx) = inner.decode(key_index);

        // SAFETY: Caller guarantees slot is valid
        let value = unsafe { (*slot.value.get()).assume_init_read() };

        // Return slot to freelist
        let chunk = inner.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;

        // Was chunk full before this free?
        let was_full = chunk_inner.is_full();

        let free_head = chunk_inner.free_head.get();
        slot.set_vacant(free_head);
        chunk_inner.free_head.set(local_idx);
        chunk_inner.len.set(chunk_inner.len.get() - 1);
        inner.len.set(inner.len.get() - 1);

        // If chunk was full, add it back to slab's chunk freelist
        if was_full {
            chunk.next_with_space.set(inner.head_with_space.get());
            inner.head_with_space.set(chunk_idx);
        }

        // Don't run Drop (we already handled deallocation)
        std::mem::forget(self);

        value
    }

    /// Alias for [`into_inner_unchecked()`](Self::into_inner_unchecked) for backwards compatibility.
    #[inline]
    #[doc(hidden)]
    pub unsafe fn remove_unchecked(self) -> T {
        unsafe { self.into_inner_unchecked() }
    }
}

impl<T> Drop for Slot<T> {
    fn drop(&mut self) {
        let slot = self.slot();
        let inner = self.inner();

        // Read key from stamp and decode once
        let key_index = slot.key_from_stamp();
        let (chunk_idx, local_idx) = inner.decode(key_index);

        // SAFETY: Slot is sole owner (!Clone), so if Drop runs, slot is occupied
        unsafe {
            std::ptr::drop_in_place((*slot.value.get()).as_mut_ptr());
        }

        // Return slot to freelist
        let chunk = inner.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;

        // Was chunk full before this free?
        let was_full = chunk_inner.is_full();

        let free_head = chunk_inner.free_head.get();
        slot.set_vacant(free_head);
        chunk_inner.free_head.set(local_idx);
        chunk_inner.len.set(chunk_inner.len.get() - 1);
        inner.len.set(inner.len.get() - 1);

        // If chunk was full, add it back to slab's chunk freelist
        if was_full {
            chunk.next_with_space.set(inner.head_with_space.get());
            inner.head_with_space.set(chunk_idx);
        }
    }
}

impl<T> fmt::Debug for Slot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slot").field("key", &self.key()).finish()
    }
}

impl<T> std::ops::Deref for Slot<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T> std::ops::DerefMut for Slot<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

impl<T> AsRef<T> for Slot<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self.get()
    }
}

impl<T> AsMut<T> for Slot<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}

impl<T> std::borrow::Borrow<T> for Slot<T> {
    #[inline]
    fn borrow(&self) -> &T {
        self.get()
    }
}

impl<T> std::borrow::BorrowMut<T> for Slot<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}

impl<T: fmt::Display> fmt::Display for Slot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

impl<T> fmt::Pointer for Slot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.slot, f)
    }
}

// =============================================================================
// Slab (growable)
// =============================================================================

/// A growable slab allocator with RAII Slot-based access.
///
/// Created via [`new`](Self::new), [`with_capacity`](Self::with_capacity),
/// or the [`Builder`](crate::Builder) API. The slab is leaked and lives for `'static`.
///
/// # Thread Safety
///
/// `Slab` is `!Send` and `!Sync` - it uses raw pointers internally.
/// The slab must only be used from the thread that created it.
///
/// # Example
///
/// ```
/// use nexus_slab::unbounded::Slab;
///
/// let slab = Slab::with_capacity(1000);
/// let entry = slab.insert(42);
/// assert_eq!(*entry.get(), 42);
/// ```
#[derive(Clone, Copy)]
pub struct Slab<T> {
    ptr: *mut SlabInner<T>,
    _marker: PhantomData<*mut ()>, // Ensures !Send + !Sync
}

impl<T> Slab<T> {
    #[inline]
    fn inner(&self) -> &SlabInner<T> {
        // SAFETY: ptr is valid for 'static (leaked)
        unsafe { &*self.ptr }
    }

    /// Creates an empty slab with default settings.
    ///
    /// No memory is allocated until first insert.
    pub fn new() -> Self {
        Self::with_chunk_capacity(DEFAULT_CHUNK_CAPACITY)
    }

    /// Creates a slab with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let slab = Self::with_chunk_capacity(DEFAULT_CHUNK_CAPACITY);
        while slab.capacity() < capacity {
            slab.grow();
        }
        slab
    }

    /// Creates a slab with the specified chunk capacity.
    ///
    /// Chunk capacity is rounded up to the next power of two.
    pub(crate) fn with_chunk_capacity(chunk_capacity: usize) -> Self {
        let inner = Box::new(SlabInner::with_chunk_capacity(chunk_capacity));
        let inner_ptr = Box::into_raw(inner);

        Self {
            ptr: inner_ptr,
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
    pub(crate) fn grow(&self) {
        self.inner().grow();
    }

    // =========================================================================
    // Insert
    // =========================================================================

    /// Inserts a value, returning an RAII [`Slot`] handle.
    ///
    /// Grows automatically if needed. The returned slot owns the storage.
    /// When dropped, the slot is deallocated.
    pub fn insert(&self, value: T) -> Slot<T> {
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
        let next_free = slot.claim_next_free(); // 1 stamp read

        chunk_inner.free_head.set(next_free);
        chunk_inner.len.set(chunk_inner.len.get() + 1);
        inner.len.set(inner.len.get() + 1);

        // Update chunk freelist if this chunk is now full
        if chunk_inner.is_full() {
            inner.head_with_space.set(chunk.next_with_space.get());
        }

        let global_idx = inner.encode(chunk_idx, free_head);

        // Write value and mark occupied
        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_key_occupied(global_idx); // 1 stamp write

        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();

        Slot::new(slot_ptr, self.ptr)
    }

    /// Inserts with access to the slot before the value exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    pub fn insert_with<F>(&self, f: F) -> Slot<T>
    where
        F: FnOnce(&Slot<T>) -> T,
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
        let next_free = slot.claim_next_free(); // 1 stamp read

        chunk_inner.free_head.set(next_free);
        chunk_inner.len.set(chunk_inner.len.get() + 1);
        inner.len.set(inner.len.get() + 1);

        if chunk_inner.is_full() {
            inner.head_with_space.set(chunk.next_with_space.get());
        }

        let global_idx = inner.encode(chunk_idx, free_head);

        // Store key in stamp BEFORE creating Slot (so Slot::key() works)
        // This requires read-modify-write since we preserve VACANT_BIT temporarily
        slot.set_key(global_idx);

        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();

        // Create entry (slot not yet occupied, but key is readable from stamp)
        let entry = Slot::new(slot_ptr, self.ptr);

        let value = f(&entry);

        unsafe {
            (*slot.value.get()).write(value);
        }
        slot.set_key_occupied(global_idx); // 1 stamp write

        entry
    }

    /// Creates an RAII Slot from a key.
    ///
    /// Returns `None` if the key is out of bounds or the slot is vacant.
    ///
    /// **Warning**: The returned slot owns the storage. When dropped, the slot
    /// is deallocated. Only call this when you want to take ownership.
    pub fn slot(&self, key: Key) -> Option<Slot<T>> {
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

        // Key is already in slot's stamp from when it was inserted
        let slot_ptr = (slot as *const SlotCell<T>).cast_mut();
        Some(Slot::new(slot_ptr, self.ptr))
    }

    /// Alias for [`slot()`](Self::slot) for backwards compatibility.
    #[inline]
    #[doc(hidden)]
    #[deprecated(since = "0.9.0", note = "renamed to slot")]
    pub fn entry(&self, key: Key) -> Option<Slot<T>> {
        self.slot(key)
    }

    /// Reserves a slot without filling it, returning a [`VacantSlot`].
    ///
    /// If dropped without calling `insert`, the slot is automatically
    /// returned to the freelist.
    pub fn vacant_slot(&self) -> VacantSlot<T> {
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
        let next_free = slot.claim_next_free(); // 1 stamp read

        chunk_inner.free_head.set(next_free);
        chunk_inner.len.set(chunk_inner.len.get() + 1);
        inner.len.set(inner.len.get() + 1);

        // Check if chunk became full after our reservation
        let chunk_became_full = chunk_inner.is_full();
        if chunk_became_full {
            inner.head_with_space.set(chunk.next_with_space.get());
        }

        let global_idx = inner.encode(chunk_idx, free_head);

        // Store key in stamp - VacantSlot::insert will later call set_key_occupied
        slot.set_key(global_idx);

        VacantSlot {
            inner: self.ptr,
            key: Key::new(global_idx),
            chunk_idx,
            local_idx: free_head,
            chunk_became_full,
            consumed: false,
            _marker: PhantomData,
        }
    }

    /// Alias for [`vacant_slot()`](Self::vacant_slot) for backwards compatibility.
    #[inline]
    #[doc(hidden)]
    #[deprecated(since = "0.9.0", note = "renamed to vacant_slot")]
    pub fn vacant_entry(&self) -> VacantSlot<T> {
        self.vacant_slot()
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

    /// Removes a value by key.
    ///
    /// Use this when you have a forgotten key and want to deallocate.
    ///
    /// # Safety
    ///
    /// - Key must refer to an occupied slot
    /// - No Slot may exist for this slot (would become dangling)
    #[inline]
    pub unsafe fn remove_by_key(&self, key: Key) -> T {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);

        let chunk = inner.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;
        let slot = chunk_inner.slot(local_idx);

        // SAFETY: Caller guarantees slot is occupied
        let value = unsafe { (*slot.value.get()).assume_init_read() };

        // Was chunk full before this remove?
        let was_full = chunk_inner.is_full();

        // Update chunk freelist
        let free_head = chunk_inner.free_head.get();
        slot.set_vacant(free_head);
        chunk_inner.free_head.set(local_idx);
        chunk_inner.len.set(chunk_inner.len.get() - 1);
        inner.len.set(inner.len.get() - 1);

        // If chunk was full, add it back to the slab's chunk freelist
        if was_full {
            chunk.next_with_space.set(inner.head_with_space.get());
            inner.head_with_space.set(chunk_idx);
        }

        value
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

        // SAFETY: Caller guarantees slot is occupied
        let value = unsafe { (*slot.value.get()).assume_init_read() };

        // Was chunk full before this remove?
        let was_full = chunk_inner.is_full();

        // Update chunk freelist
        let free_head = chunk_inner.free_head.get();
        slot.set_vacant(free_head);
        chunk_inner.free_head.set(local_idx);
        chunk_inner.len.set(chunk_inner.len.get() - 1);
        inner.len.set(inner.len.get() - 1);

        // If chunk was full, add it back to the slab's chunk freelist
        if was_full {
            chunk.next_with_space.set(inner.head_with_space.get());
            inner.head_with_space.set(chunk_idx);
        }

        value
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

        inner.len.set(0);
    }

    // =========================================================================
    // Unsafe key-based access
    // =========================================================================

    /// Returns a reference to the value at `key`.
    ///
    /// # Safety
    ///
    /// - Key must refer to an occupied slot
    /// - No Slot may have exclusive (`&mut`) access to this slot
    /// - Caller must ensure no aliasing violations
    #[inline]
    pub unsafe fn get_by_key(&self, key: Key) -> &T {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);
        let chunk = inner.chunk(chunk_idx);
        unsafe { chunk.inner.slot(local_idx).value_ref() }
    }

    /// Returns a mutable reference to the value at `key`.
    ///
    /// # Safety
    ///
    /// - Key must refer to an occupied slot
    /// - No other references (Slot-based or key-based) may exist to this slot
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_by_key_mut(&self, key: Key) -> &mut T {
        let inner = self.inner();
        let index = key.index();
        let (chunk_idx, local_idx) = inner.decode(index);
        let chunk = inner.chunk(chunk_idx);
        unsafe { chunk.inner.slot(local_idx).value_mut() }
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
// VacantSlot
// =============================================================================

/// A reserved but unfilled slot in a [`Slab`].
///
/// Created by [`Slab::vacant_slot`]. Fill with [`insert`](Self::insert)
/// or drop to return the slot to the freelist.
#[must_use = "dropping VacantSlot releases the reserved slot"]
pub struct VacantSlot<T> {
    inner: *mut SlabInner<T>,
    key: Key,
    chunk_idx: u32,
    local_idx: u32,
    chunk_became_full: bool,
    consumed: bool,
    _marker: PhantomData<T>,
}

impl<T> VacantSlot<T> {
    #[inline]
    fn inner(&self) -> &SlabInner<T> {
        // SAFETY: inner ptr is valid for 'static
        unsafe { &*self.inner }
    }

    /// Returns the key this slot will have once filled.
    #[inline]
    pub fn key(&self) -> Key {
        self.key
    }

    /// Fills the slot with a value, returning an RAII [`Slot`].
    #[inline]
    pub fn insert(mut self, value: T) -> Slot<T> {
        let key_index = self.key.index();

        let slot_ptr = {
            let inner = self.inner();
            let chunk = inner.chunk(self.chunk_idx);
            let slot = chunk.inner.slot(self.local_idx);

            unsafe {
                (*slot.value.get()).write(value);
            }
            slot.set_key_occupied(key_index); // 1 write (no read needed)

            (slot as *const SlotCell<T>).cast_mut()
        };

        self.consumed = true;

        Slot::new(slot_ptr, self.inner)
    }

    /// Fills the slot using a closure that receives the key.
    ///
    /// Useful for self-referential patterns.
    #[inline]
    pub fn insert_with<F: FnOnce(Key) -> T>(self, f: F) -> Slot<T> {
        let key = self.key;
        self.insert(f(key))
    }
}

impl<T> Drop for VacantSlot<T> {
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
            inner.len.set(inner.len.get() - 1);

            // If chunk was full after our reservation, re-add to slab freelist
            if self.chunk_became_full {
                chunk.next_with_space.set(inner.head_with_space.get());
                inner.head_with_space.set(self.chunk_idx);
            }
        }
    }
}

impl<T> fmt::Debug for VacantSlot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VacantSlot")
            .field("key", &self.key)
            .finish()
    }
}

// =============================================================================
// Additional helper for Slab (try_insert for API parity with bounded::Slab)
// =============================================================================

impl<T> Slab<T> {
    /// Inserts a value, returning an RAII [`Slot`] handle.
    ///
    /// This always succeeds (grows if needed), but the method name
    /// provides API parity with [`bounded::Slab::try_insert`](crate::bounded::Slab::try_insert).
    #[inline]
    pub fn try_insert(&self, value: T) -> Slot<T> {
        self.insert(value)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Builder;

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
        let slab: Slab<u64> = Builder::default()
            .unbounded()
            .chunk_capacity(128)
            .capacity(1000)
            .build();
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

        // Slot dropped, slot freed
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn forget_keeps_data() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(100);
        let key = entry.leak();

        // Data still exists
        assert_eq!(slab.len(), 1);
        // SAFETY: key is valid (just obtained from forget)
        assert_eq!(unsafe { *slab.get_by_key(key) }, 100);

        // Clean up via remove
        // SAFETY: key is valid
        let value = unsafe { slab.remove_by_key(key) };
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
            let entry = slab.slot(key).unwrap();
            assert_eq!(*entry.get(), 42);
        }

        // Slot dropped, slot freed
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn multiple_chunks() {
        let slab: Slab<u64> = Builder::default().unbounded().chunk_capacity(4).build();

        // Insert and forget to keep entries alive
        let mut keys = Vec::new();
        for i in 0..20u64 {
            let entry = slab.insert(i);
            keys.push(entry.leak());
        }

        assert!(slab.num_chunks() > 1);
        assert_eq!(slab.len(), 20);

        for (i, key) in keys.iter().enumerate() {
            // SAFETY: key is valid
            assert_eq!(unsafe { *slab.get_by_key(*key) }, i as u64);
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
        let slab: Slab<u64> = Builder::default().unbounded().chunk_capacity(4).build();

        for i in 0..20u64 {
            slab.insert(i).leak(); // Forget to keep data
        }

        let chunks_before = slab.num_chunks();
        slab.clear();

        assert_eq!(slab.len(), 0);
        assert_eq!(slab.num_chunks(), chunks_before);
    }

    #[test]
    fn stress_insert_remove() {
        let slab: Slab<u64> = Builder::default().unbounded().chunk_capacity(64).build();

        for i in 0..10_000u64 {
            let entry = slab.insert(i);
            assert_eq!(*entry.get(), i);
            entry.into_inner();
        }
    }

    #[test]
    fn key_based_access() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        let key = entry.leak();

        assert!(slab.contains_key(key));

        // Unsafe key-based access
        // SAFETY: key is valid
        assert_eq!(unsafe { *slab.get_by_key(key) }, 42);

        // SAFETY: key is valid
        let removed = unsafe { slab.remove_by_key(key) };
        assert_eq!(removed, 42);
        assert!(!slab.contains_key(key));
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
    fn remove_by_key_basic() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        let key = entry.leak();

        // SAFETY: key is valid
        let value = unsafe { slab.remove_by_key(key) };
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn entry_after_key_removal() {
        let slab: Slab<u64> = Slab::new();

        let entry = slab.insert(42);
        let key = entry.leak();

        // Remove via key
        // SAFETY: key is valid
        unsafe { slab.remove_by_key(key) };

        // Can't re-acquire entry - slot is vacant
        assert!(slab.slot(key).is_none());
    }

    #[test]
    fn vacant_entry_insert() {
        let slab: Slab<u64> = Slab::new();

        let vacant = slab.vacant_slot();
        let entry = vacant.insert(42);

        assert_eq!(*entry.get(), 42);
        assert_eq!(slab.len(), 1);
    }

    #[test]
    fn vacant_entry_key_matches() {
        let slab: Slab<u64> = Slab::new();

        let vacant = slab.vacant_slot();
        let key_before = vacant.key();
        let entry = vacant.insert(100);
        let key_after = entry.key();

        assert_eq!(key_before, key_after);
    }

    #[test]
    fn vacant_entry_drop_returns_slot() {
        let slab: Slab<u64> = Slab::new();

        {
            let _vacant = slab.vacant_slot();
            assert_eq!(slab.len(), 1);
        }

        // After drop, slot is returned to freelist
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn replace() {
        let slab: Slab<u64> = Slab::new();
        let mut entry = slab.insert(42);

        let old = entry.replace(100);
        assert_eq!(old, 42);
        assert_eq!(*entry.get(), 100);
    }

    #[test]
    fn and_modify() {
        let slab: Slab<u64> = Slab::new();
        let mut entry = slab.insert(0);

        entry.and_modify(|v| *v += 1).and_modify(|v| *v *= 2);

        assert_eq!(*entry.get(), 2);
    }

    #[test]
    fn explicit_remove() {
        let slab: Slab<u64> = Slab::new();
        let entry = slab.insert(42);

        let value = entry.into_inner();
        assert_eq!(value, 42);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn entry_size() {
        // Slot is 16 bytes: slot ptr (8) + inner ptr (8)
        // Key is stored in slot's stamp, not in Slot
        assert_eq!(std::mem::size_of::<Slot<u64>>(), 16);
    }
}
