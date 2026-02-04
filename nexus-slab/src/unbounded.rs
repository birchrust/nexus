//! Growable slab allocator.
//!
//! This module provides an unbounded (growable) slab with leaked storage.
//! Growth happens by adding independent chunks — no copying.
//!
//! # Example
//!
//! ```
//! use nexus_slab::unbounded::Slab;
//!
//! let slab = Slab::new(4096);
//! let slot = slab.new_slot(42u64);
//! assert_eq!(*slot, 42);
//! // slot drops, storage freed back to slab
//! ```

use std::borrow::{Borrow, BorrowMut};
use std::cell::Cell;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::ptr;

use crate::Key;
use crate::bounded::SlabInner as BoundedSlabInner;
use crate::shared::SlotCell;

// =============================================================================
// Constants
// =============================================================================

/// Sentinel for chunk freelist
const CHUNK_NONE: u32 = u32::MAX;

// =============================================================================
// ChunkEntry
// =============================================================================

/// Internal wrapper for a chunk in the growable slab.
#[doc(hidden)]
pub struct ChunkEntry<T> {
    pub(crate) inner: Box<BoundedSlabInner<T>>,
    pub(crate) next_with_space: Cell<u32>,
}

impl<T> ChunkEntry<T> {
    /// Returns a reference to the inner BoundedSlabInner.
    #[doc(hidden)]
    #[inline]
    pub fn inner_ref(&self) -> &BoundedSlabInner<T> {
        &self.inner
    }
}

// =============================================================================
// SlabInner
// =============================================================================

/// Internal state for a growable slab.
///
/// This is the storage backend for unbounded allocators.
///
/// # Const Construction
///
/// This type supports const construction via [`new()`](Self::new) followed by
/// runtime initialization via [`init()`](Self::init). This enables use with
/// `thread_local!` using the `const { }` block syntax for zero-overhead TLS access.
///
/// ```ignore
/// thread_local! {
///     static SLAB: SlabInner<MyType> = const { SlabInner::new() };
/// }
///
/// // Later, at runtime:
/// SLAB.with(|s| s.init(4096));
/// ```
#[doc(hidden)]
pub struct SlabInner<T> {
    chunks: std::cell::UnsafeCell<Vec<ChunkEntry<T>>>,
    chunk_shift: Cell<u32>,
    chunk_mask: Cell<u32>,
    head_with_space: Cell<u32>,
    chunk_capacity: Cell<u32>,
}

impl<T> SlabInner<T> {
    /// Creates an empty, uninitialized slab.
    ///
    /// This is a const function that performs no allocation. Call [`init()`](Self::init)
    /// to configure chunk capacity before use.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // For use with thread_local! const initialization
    /// thread_local! {
    ///     static SLAB: SlabInner<u64> = const { SlabInner::new() };
    /// }
    /// ```
    #[inline]
    pub const fn new() -> Self {
        Self {
            chunks: std::cell::UnsafeCell::new(Vec::new()),
            chunk_shift: Cell::new(0),
            chunk_mask: Cell::new(0),
            head_with_space: Cell::new(CHUNK_NONE),
            chunk_capacity: Cell::new(0),
        }
    }

    /// Initializes the slab with the given chunk capacity.
    ///
    /// This configures the chunk parameters. Chunks are allocated on-demand
    /// when slots are requested. Must be called exactly once before any allocations.
    ///
    /// # Panics
    ///
    /// - Panics if the slab is already initialized (chunk_capacity > 0)
    /// - Panics if chunk_capacity is zero
    /// - Panics if chunk_capacity exceeds maximum (2^30)
    pub fn init(&self, chunk_capacity: u32) {
        assert!(
            self.chunk_capacity.get() == 0,
            "SlabInner already initialized"
        );
        assert!(chunk_capacity > 0, "chunk_capacity must be non-zero");
        assert!(chunk_capacity <= 1 << 30, "chunk_capacity exceeds maximum");

        let chunk_capacity = chunk_capacity.next_power_of_two();
        let chunk_shift = chunk_capacity.trailing_zeros();
        let chunk_mask = chunk_capacity - 1;

        self.chunk_capacity.set(chunk_capacity);
        self.chunk_shift.set(chunk_shift);
        self.chunk_mask.set(chunk_mask);
    }

    /// Returns true if the slab has been initialized.
    #[inline]
    #[allow(dead_code)]
    pub fn is_initialized(&self) -> bool {
        self.chunk_capacity.get() > 0
    }

    /// Creates a new SlabInner with the given chunk capacity.
    ///
    /// This is a convenience method equivalent to `new()` followed by `init()`.
    #[doc(hidden)]
    pub fn with_chunk_capacity(chunk_capacity: usize) -> Self {
        let inner = Self::new();
        inner.init(chunk_capacity as u32);
        inner
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

    /// Returns the total length (number of occupied slots across all chunks).
    ///
    /// This scans all chunks - O(chunks * slots). Use only for diagnostics/shutdown.
    #[doc(hidden)]
    pub fn len(&self) -> usize {
        self.chunks().iter().map(|c| c.inner.len() as usize).sum()
    }

    /// Returns true if the slab is empty.
    ///
    /// This scans chunks - O(chunks * slots). Use only for diagnostics/shutdown.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.chunks().iter().all(|c| c.inner.is_empty())
    }

    /// Decodes a global index into (chunk_idx, local_idx).
    #[doc(hidden)]
    #[inline]
    pub fn decode(&self, index: u32) -> (u32, u32) {
        let chunk_idx = index >> self.chunk_shift.get();
        let local_idx = index & self.chunk_mask.get();
        (chunk_idx, local_idx)
    }

    #[inline]
    fn encode(&self, chunk_idx: u32, local_idx: u32) -> u32 {
        (chunk_idx << self.chunk_shift.get()) | local_idx
    }

    /// Returns a reference to the chunk at the given index.
    #[doc(hidden)]
    #[inline]
    pub fn chunk(&self, chunk_idx: u32) -> &ChunkEntry<T> {
        let chunks = self.chunks();
        debug_assert!((chunk_idx as usize) < chunks.len());
        unsafe { chunks.get_unchecked(chunk_idx as usize) }
    }

    /// Grows the slab by adding a new chunk.
    #[doc(hidden)]
    pub fn grow(&self) {
        let chunks = self.chunks_mut();
        let chunk_idx = chunks.len() as u32;
        let inner = Box::new(BoundedSlabInner::with_capacity(self.chunk_capacity.get()));

        let entry = ChunkEntry {
            inner,
            next_with_space: Cell::new(self.head_with_space.get()),
        };

        chunks.push(entry);
        self.head_with_space.set(chunk_idx);
    }

    /// Returns the total capacity across all chunks.
    #[doc(hidden)]
    pub fn capacity(&self) -> usize {
        self.chunks().len() * self.chunk_capacity.get() as usize
    }

    /// Returns the chunk capacity.
    #[doc(hidden)]
    #[inline]
    pub fn chunk_capacity(&self) -> u32 {
        self.chunk_capacity.get()
    }

    // =========================================================================
    // Helper methods for macro-generated code
    // =========================================================================

    /// Returns true if no chunk has available space.
    #[doc(hidden)]
    #[inline]
    pub fn head_with_space_is_none(&self) -> bool {
        self.head_with_space.get() == CHUNK_NONE
    }

    /// Returns the chunk index and reference to the chunk with available space.
    ///
    /// # Panics
    ///
    /// Panics if no chunk has space (call `grow()` first if needed).
    #[doc(hidden)]
    #[inline]
    pub fn head_chunk(&self) -> (u32, &ChunkEntry<T>) {
        let chunk_idx = self.head_with_space.get();
        debug_assert!(chunk_idx != CHUNK_NONE, "no chunk with space");
        (chunk_idx, self.chunk(chunk_idx))
    }

    /// Encodes chunk index and local index into a Key.
    #[doc(hidden)]
    #[inline]
    pub fn encode_key(&self, chunk_idx: u32, local_idx: u32) -> Key {
        Key::new(self.encode(chunk_idx, local_idx))
    }

    /// Removes the head chunk from the available-space list.
    ///
    /// Called when the head chunk becomes full.
    #[doc(hidden)]
    #[inline]
    pub fn pop_head_chunk(&self) {
        let chunk_idx = self.head_with_space.get();
        debug_assert!(chunk_idx != CHUNK_NONE);
        let chunk = self.chunk(chunk_idx);
        self.head_with_space.set(chunk.next_with_space.get());
    }

    /// Adds a chunk to the available-space list.
    ///
    /// Called when a previously-full chunk has a slot freed.
    #[doc(hidden)]
    #[inline]
    pub fn push_chunk_to_available(&self, chunk_idx: u32) {
        let chunk = self.chunk(chunk_idx);
        chunk.next_with_space.set(self.head_with_space.get());
        self.head_with_space.set(chunk_idx);
    }

    /// Returns true if the chunk index is valid.
    #[doc(hidden)]
    #[inline]
    pub fn chunk_exists(&self, chunk_idx: u32) -> bool {
        (chunk_idx as usize) < self.chunks().len()
    }

    // =========================================================================
    // Allocation methods
    // =========================================================================

    /// Claims a slot, writes the value, and marks it occupied.
    ///
    /// Grows automatically if needed (always succeeds unless OOM).
    pub fn try_alloc(&self, value: T) -> *mut SlotCell<T> {
        // Ensure we have space (grow if needed)
        if self.head_with_space.get() == CHUNK_NONE {
            self.grow();
        }

        // Get the chunk with space
        let chunk_idx = self.head_with_space.get();
        let chunk = self.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;

        // Load freelist head pointer from chunk
        let slot_ptr = chunk_inner.free_head.get();
        debug_assert!(!slot_ptr.is_null(), "chunk on freelist has no free slots");

        // Get next pointer from slot
        // SAFETY: slot_ptr came from the freelist within this chunk
        let slot = unsafe { &*slot_ptr };
        let next_free = slot.next_free();

        // Mark slot as occupied
        slot.set_occupied();

        // Write the value
        // SAFETY: Slot is claimed, value area is uninitialized
        unsafe {
            slot.value.get().cast::<T>().write(value);
        }

        // Update chunk's freelist head
        chunk_inner.free_head.set(next_free);

        // If chunk is now full, remove from slab's available-chunk list
        if next_free.is_null() {
            self.head_with_space.set(chunk.next_with_space.get());
        }

        slot_ptr
    }

    /// Returns a slot to the freelist by key.
    ///
    /// Does NOT drop the value — caller must drop before calling.
    ///
    /// # Safety
    ///
    /// - `key` must refer to a previously claimed slot
    /// - Value must already be dropped or moved out
    pub unsafe fn dealloc(&self, key: Key) {
        let (chunk_idx, local_idx) = self.decode(key.index());

        let chunk = self.chunk(chunk_idx);
        let chunk_inner = &*chunk.inner;

        let slot_ptr = unsafe { chunk_inner.slots_ptr().add(local_idx as usize) };
        let slot = unsafe { &*slot_ptr };

        let free_head = chunk_inner.free_head.get();
        let was_full = free_head.is_null();

        slot.set_vacant(free_head);
        chunk_inner.free_head.set(slot_ptr);

        if was_full {
            chunk.next_with_space.set(self.head_with_space.get());
            self.head_with_space.set(chunk_idx);
        }
    }

    /// Returns a slot to the freelist by pointer.
    ///
    /// Does NOT drop the value — caller must drop before calling.
    /// This is the hot-path variant that avoids the key→pointer round-trip.
    /// Finds the owning chunk via linear scan (typically 1-5 chunks).
    ///
    /// # Safety
    ///
    /// - `slot_ptr` must point to a previously claimed slot within this slab
    /// - Value must already be dropped or moved out
    #[doc(hidden)]
    pub unsafe fn dealloc_ptr(&self, slot_ptr: *mut SlotCell<T>) {
        let chunks = self.chunks();
        let cap = self.chunk_capacity.get() as usize;

        // Find which chunk owns this pointer
        for (chunk_idx, chunk) in chunks.iter().enumerate() {
            let chunk_inner = &*chunk.inner;
            let base = chunk_inner.slots_ptr();
            let end = base.wrapping_add(cap);

            if slot_ptr >= base && slot_ptr < end {
                // SAFETY: slot_ptr is within this chunk
                let slot = unsafe { &*slot_ptr };

                let free_head = chunk_inner.free_head.get();
                let was_full = free_head.is_null();

                slot.set_vacant(free_head);
                chunk_inner.free_head.set(slot_ptr);

                if was_full {
                    chunk.next_with_space.set(self.head_with_space.get());
                    self.head_with_space.set(chunk_idx as u32);
                }
                return;
            }
        }

        // Should never reach here if slot_ptr is valid
        debug_assert!(false, "dealloc_ptr: slot_ptr not found in any chunk");
    }

    /// Computes the global key for a slot pointer.
    ///
    /// Finds the owning chunk via linear scan, then encodes chunk_idx + local_idx.
    /// This is a cold-path operation for lazy key computation.
    ///
    /// # Safety
    ///
    /// `slot_ptr` must point to a valid slot within this slab.
    #[doc(hidden)]
    pub unsafe fn slot_to_index_global(&self, slot_ptr: *const SlotCell<T>) -> Key {
        let chunks = self.chunks();
        let cap = self.chunk_capacity.get() as usize;

        for (chunk_idx, chunk) in chunks.iter().enumerate() {
            let chunk_inner = &*chunk.inner;
            let base = chunk_inner.slots_ptr().cast_const();
            let end = base.wrapping_add(cap);

            if slot_ptr >= base && slot_ptr < end {
                // SAFETY: slot_ptr is within this chunk
                let local_idx = unsafe { chunk_inner.slot_to_index(slot_ptr) };
                return Key::new(self.encode(chunk_idx as u32, local_idx));
            }
        }

        // Should never reach here if slot_ptr is valid
        debug_assert!(false, "slot_to_index_global: slot_ptr not found in any chunk");
        Key::NONE
    }

    /// Gets the slot cell pointer for a key.
    ///
    /// # Safety
    ///
    /// `key` must refer to a valid slot within the slab.
    #[doc(hidden)]
    #[inline]
    pub unsafe fn slot_cell(&self, key: Key) -> *mut SlotCell<T> {
        let (chunk_idx, local_idx) = self.decode(key.index());
        let chunk = self.chunk(chunk_idx);
        unsafe { chunk.inner.slots_ptr().add(local_idx as usize) }
    }

    /// Checks if a key refers to an occupied slot.
    pub fn contains_key(&self, key: Key) -> bool {
        let (chunk_idx, local_idx) = self.decode(key.index());

        if (chunk_idx as usize) >= self.chunks().len() {
            return false;
        }

        let chunk = self.chunk(chunk_idx);
        if local_idx >= chunk.inner.capacity() {
            return false;
        }

        chunk.inner.slot(local_idx).is_occupied()
    }
}

impl<T> Default for SlabInner<T> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Slab
// =============================================================================

/// Pre-allocated growable slab.
///
/// `Slab<T>` wraps a leaked `SlabInner<T>` for stable `'static` storage.
/// The storage lives for the lifetime of the program. Growth happens by
/// adding independent chunks — no copying.
///
/// # Thread Safety
///
/// `Slab` is `!Send` and `!Sync`. Each slab must be used from a single thread.
///
/// # Example
///
/// ```
/// use nexus_slab::unbounded::Slab;
///
/// let slab = Slab::new(4096);
/// let slot = slab.new_slot(42u64);
/// assert_eq!(*slot, 42);
/// ```
pub struct Slab<T: 'static> {
    inner: &'static SlabInner<T>,
}

// Manual Copy/Clone to avoid requiring T: Copy/Clone
impl<T: 'static> Clone for Slab<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Slab<T> {}

impl<T: 'static> Slab<T> {
    /// Creates a new unbounded slab with the given chunk capacity.
    ///
    /// The storage is leaked and lives for the lifetime of the program.
    /// Chunks are allocated on-demand when slots are requested.
    ///
    /// # Panics
    ///
    /// - Panics if chunk_capacity is zero
    /// - Panics if chunk_capacity exceeds maximum (2^30)
    pub fn new(chunk_capacity: u32) -> Self {
        let inner = Box::leak(Box::new(SlabInner::with_chunk_capacity(
            chunk_capacity as usize,
        )));
        Self { inner }
    }

    /// Creates a new slot containing the given value.
    ///
    /// Always succeeds — grows the slab if needed.
    #[inline]
    pub fn new_slot(&self, value: T) -> Slot<T> {
        let slot_ptr = self.inner.try_alloc(value);
        Slot {
            slot_ptr,
            inner: self.inner,
        }
    }

    /// Returns the number of occupied slots.
    ///
    /// O(n) scan across all chunks. Use for diagnostics, not hot path.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if no slots are occupied.
    ///
    /// O(n) scan.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the total capacity across all chunks.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Checks if a key refers to an occupied slot.
    #[inline]
    pub fn contains_key(&self, key: Key) -> bool {
        self.inner.contains_key(key)
    }

    /// Gets a reference to a value by key.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    ///
    /// # Safety
    ///
    /// Caller must ensure no mutable references to this slot exist.
    #[inline]
    pub unsafe fn get_by_key(&self, key: Key) -> Option<&T> {
        if !self.contains_key(key) {
            return None;
        }
        let slot_cell = unsafe { self.inner.slot_cell(key) };
        Some(unsafe { (*slot_cell).get_value() })
    }

    /// Gets a mutable reference to a value by key.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    ///
    /// # Safety
    ///
    /// Caller must ensure no other references to this slot exist.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_by_key_mut(&self, key: Key) -> Option<&mut T> {
        if !self.contains_key(key) {
            return None;
        }
        let slot_cell = unsafe { self.inner.slot_cell(key) };
        Some(unsafe { (*slot_cell).get_value_mut() })
    }

    /// Removes a value by key, returning it.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    ///
    /// # Safety
    ///
    /// Caller must ensure no references to this slot exist.
    pub unsafe fn remove_by_key(&self, key: Key) -> Option<T> {
        if !self.contains_key(key) {
            return None;
        }
        let slot_cell = unsafe { self.inner.slot_cell(key) };
        let value = unsafe { ptr::read((*slot_cell).value_ptr()) };
        unsafe { self.inner.dealloc(key) };
        Some(value)
    }
}

impl<T: 'static> fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            .field("len", &self.inner.len())
            .field("capacity", &self.inner.capacity())
            .finish()
    }
}

// =============================================================================
// Slot
// =============================================================================

/// RAII handle to a slot in an unbounded slab.
///
/// Analogous to `Box<T>`: owns the value and deallocates on drop.
/// The backing storage is a leaked `SlabInner<T>` with a `'static` lifetime.
///
/// # Size
///
/// 16 bytes (slot pointer + inner pointer).
#[must_use = "dropping Slot returns it to the slab"]
pub struct Slot<T: 'static> {
    slot_ptr: *mut SlotCell<T>,
    inner: &'static SlabInner<T>,
}

impl<T: 'static> Slot<T> {
    /// Returns the key for this slot.
    ///
    /// Lazily computed from pointer via chunk scan. This is a cold-path operation.
    #[inline]
    pub fn key(&self) -> Key {
        // SAFETY: slot_ptr is valid and within the slab's storage
        unsafe { self.inner.slot_to_index_global(self.slot_ptr) }
    }

    /// Leaks the slot, keeping the data alive and returning its key.
    ///
    /// After calling `leak()`, the slot remains occupied but has no
    /// Slot owner. Access the data via the slab's key-based methods.
    #[inline]
    pub fn leak(self) -> Key {
        let key = self.key();
        std::mem::forget(self);
        key
    }

    /// Consumes the slot, returning the value and deallocating.
    pub fn into_inner(self) -> T {
        // SAFETY: Slot owns the value
        let value = unsafe {
            let slot = &*self.slot_ptr;
            ptr::read(slot.value_ptr())
        };

        // SAFETY: Value moved out, slot_ptr valid
        unsafe { self.inner.dealloc_ptr(self.slot_ptr) };

        std::mem::forget(self);
        value
    }

    /// Replaces the value, returning the old one.
    #[inline]
    pub fn replace(&mut self, value: T) -> T {
        // SAFETY: We own the slot exclusively (&mut self)
        unsafe {
            let slot = &*self.slot_ptr;
            let ptr = slot.value_ptr_mut();
            let old = ptr::read(ptr);
            ptr::write(ptr, value);
            old
        }
    }

    #[inline]
    fn slot_cell(&self) -> &SlotCell<T> {
        // SAFETY: Slot holds a valid slot pointer
        unsafe { &*self.slot_ptr }
    }
}

impl<T: 'static> Drop for Slot<T> {
    fn drop(&mut self) {
        // Drop the value
        // SAFETY: We own the slot, value is initialized
        unsafe {
            let slot = &*self.slot_ptr;
            ptr::drop_in_place(slot.value_ptr_mut());
        }

        // Return slot to freelist by pointer (no key round-trip)
        // SAFETY: Value dropped, slot_ptr valid
        unsafe { self.inner.dealloc_ptr(self.slot_ptr) };
    }
}

impl<T: 'static> Deref for Slot<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: Slot owns the slot, value is initialized
        unsafe { self.slot_cell().get_value() }
    }
}

impl<T: 'static> DerefMut for Slot<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: Slot owns the slot, &mut ensures exclusivity
        unsafe { self.slot_cell().get_value_mut() }
    }
}

impl<T: 'static> AsRef<T> for Slot<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: 'static> AsMut<T> for Slot<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: 'static> Borrow<T> for Slot<T> {
    #[inline]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T: 'static> BorrowMut<T> for Slot<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: 'static + fmt::Debug> fmt::Debug for Slot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slot")
            .field("key", &self.key())
            .field("value", &**self)
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
    fn slab_basic() {
        let slab = Slab::<u64>::new(16);
        assert_eq!(slab.len(), 0);

        let slot = slab.new_slot(42);
        assert_eq!(*slot, 42);
        assert_eq!(slab.len(), 1);

        drop(slot);
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn slab_grows() {
        let slab = Slab::<u64>::new(4);

        let mut slots = Vec::new();
        for i in 0..10 {
            slots.push(slab.new_slot(i));
        }

        assert_eq!(slab.len(), 10);
        assert!(slab.capacity() >= 10);

        slots.clear();
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn slot_deref_mut() {
        let slab = Slab::<String>::new(16);
        let mut slot = slab.new_slot("hello".to_string());
        slot.push_str(" world");
        assert_eq!(&*slot, "hello world");
    }

    #[test]
    fn slot_key_and_leak() {
        let slab = Slab::<u64>::new(16);
        let slot = slab.new_slot(42);
        let key = slot.key();
        assert!(key.is_some());

        let leaked_key = slot.leak();
        assert_eq!(key, leaked_key);
        assert_eq!(slab.len(), 1);

        let value = unsafe { slab.remove_by_key(leaked_key) };
        assert_eq!(value, Some(42));
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn slot_into_inner() {
        let slab = Slab::<String>::new(16);
        let slot = slab.new_slot("hello".to_string());
        assert_eq!(slab.len(), 1);

        let value = slot.into_inner();
        assert_eq!(value, "hello");
        assert_eq!(slab.len(), 0);
    }

    #[test]
    fn slot_replace() {
        let slab = Slab::<u64>::new(16);
        let mut slot = slab.new_slot(42);
        let old = slot.replace(100);
        assert_eq!(old, 42);
        assert_eq!(*slot, 100);
    }

    #[test]
    fn key_based_access() {
        let slab = Slab::<u64>::new(16);
        let slot = slab.new_slot(42);
        let key = slot.leak();

        assert!(slab.contains_key(key));
        assert_eq!(unsafe { slab.get_by_key(key) }, Some(&42));

        let value = unsafe { slab.remove_by_key(key) };
        assert_eq!(value, Some(42));
        assert!(!slab.contains_key(key));
    }

    #[test]
    fn slab_is_copy() {
        let slab = Slab::<u64>::new(16);
        let slab2 = slab; // Copy
        let slab3 = slab; // Copy again

        let slot = slab.new_slot(42);
        assert_eq!(slab2.len(), 1);
        assert_eq!(slab3.len(), 1);
        drop(slot);
    }

    #[test]
    fn chunk_freelist_maintenance() {
        let slab = Slab::<u64>::new(2);

        // Fill first chunk
        let s1 = slab.new_slot(1);
        let s2 = slab.new_slot(2);
        // Triggers growth
        let s3 = slab.new_slot(3);
        assert_eq!(slab.len(), 3);

        // Free from first chunk — should add it back to available list
        drop(s1);
        assert_eq!(slab.len(), 2);

        // Should reuse the freed slot
        let _s4 = slab.new_slot(4);
        assert_eq!(slab.len(), 3);

        drop(s2);
        drop(s3);
        drop(_s4);
    }

    #[test]
    fn slot_size() {
        assert_eq!(std::mem::size_of::<Slot<u64>>(), 16);
    }

    #[test]
    fn borrow_traits() {
        use std::borrow::{Borrow, BorrowMut};

        let slab = Slab::<u64>::new(16);
        let mut slot = slab.new_slot(42);

        let borrowed: &u64 = slot.borrow();
        assert_eq!(*borrowed, 42);

        let borrowed_mut: &mut u64 = slot.borrow_mut();
        *borrowed_mut = 100;
        assert_eq!(*slot, 100);
    }
}
