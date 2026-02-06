//! Growable slab allocator.
//!
//! This module provides an unbounded (growable) slab allocator.
//! Growth happens by adding independent chunks — no copying.
//!
//! # Example
//!
//! ```
//! use nexus_slab::unbounded::Slab;
//!
//! // SAFETY: slab must outlive all allocated Slots
//! let slab = unsafe { Slab::new(4096) };
//! let slot = slab.alloc(42u64);
//! assert_eq!(*slot, 42);
//! // Raw slot - must explicitly free
//! // SAFETY: slot was allocated from this slab
//! unsafe { slab.dealloc(slot) };
//! ```

use std::cell::Cell;
use std::fmt;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ptr;

use crate::bounded::SlabInner as BoundedSlabInner;
use crate::shared::{Slot, SlotCell};

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
    chunk_capacity: Cell<u32>,
    head_with_space: Cell<u32>,
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
            chunk_capacity: Cell::new(0),
            head_with_space: Cell::new(CHUNK_NONE),
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

        self.chunk_capacity.set(chunk_capacity);
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

    // =========================================================================
    // Allocation methods
    // =========================================================================

    /// Claims a slot and writes the value.
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

        // SAFETY: slot_ptr came from the freelist. Slot is vacant, so next_free is active.
        let next_free = unsafe { (*slot_ptr).next_free };

        // Write the value — overwrites next_free (union semantics)
        // SAFETY: Slot is claimed from freelist, we have exclusive access
        unsafe {
            (*slot_ptr).value = ManuallyDrop::new(MaybeUninit::new(value));
        }

        // Update chunk's freelist head
        chunk_inner.free_head.set(next_free);

        // If chunk is now full, remove from slab's available-chunk list
        if next_free.is_null() {
            self.head_with_space.set(chunk.next_with_space.get());
        }

        slot_ptr
    }

    /// Returns a slot to the freelist by pointer.
    ///
    /// Does NOT drop the value — caller must drop before calling.
    /// This is the hot-path variant that avoids the key→pointer round-trip.
    /// Finds the owning chunk via linear scan (typically 1-5 chunks).
    ///
    /// # Performance
    ///
    /// O(n) where n = chunk count. Typically 1-5 chunks in practice.
    /// For bounded slabs, use `bounded::Slab` which has O(1) deallocation.
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
                let free_head = chunk_inner.free_head.get();
                let was_full = free_head.is_null();

                // Write freelist link — overwrites value bytes (union semantics)
                // SAFETY: slot_ptr is within this chunk
                unsafe {
                    (*slot_ptr).next_free = free_head;
                }
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
/// `Slab<T>` owns its storage via a `Box<SlabInner<T>>`. Growth happens by
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
/// // SAFETY: slab must outlive all allocated Slots
/// let slab = unsafe { Slab::new(4096) };
/// let slot = slab.alloc(42u64);
/// assert_eq!(*slot, 42);
/// // SAFETY: slot was allocated from this slab
/// unsafe { slab.dealloc(slot) };
/// ```
pub struct Slab<T> {
    inner: Box<SlabInner<T>>,
}

impl<T> Slab<T> {
    /// Creates a new unbounded slab with the given chunk capacity.
    ///
    /// Chunks are allocated on-demand when slots are requested.
    ///
    /// # Safety
    ///
    /// The caller must ensure this `Slab` outlives all [`Slot`]s allocated from it.
    /// Dropping the `Slab` while `Slot`s are outstanding results in use-after-free.
    ///
    /// # Panics
    ///
    /// - Panics if chunk_capacity is zero
    /// - Panics if chunk_capacity exceeds maximum (2^30)
    #[inline]
    pub unsafe fn new(chunk_capacity: u32) -> Self {
        let inner = Box::new(SlabInner::with_chunk_capacity(chunk_capacity as usize));
        Self { inner }
    }

    /// Returns the total capacity across all chunks.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    // =========================================================================
    // Raw API (Layer 1)
    // =========================================================================

    /// Allocates a slot and writes the value. Returns a raw slot handle.
    ///
    /// Always succeeds — grows the slab if needed.
    #[inline]
    pub fn alloc(&self, value: T) -> Slot<T> {
        let slot_ptr = self.inner.try_alloc(value);
        // SAFETY: try_alloc returns a valid, occupied slot pointer
        unsafe { Slot::from_ptr(slot_ptr) }
    }

    /// Deallocates a slot, dropping the value and returning storage to the freelist.
    ///
    /// # Performance
    ///
    /// O(n) where n = chunk count, due to chunk lookup. Typically 1-5 chunks.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from **this** slab (not a different slab)
    /// - No references to the slot's value may exist
    ///
    /// Note: Double-free is prevented at compile time (`Slot` is move-only).
    #[inline]
    #[allow(clippy::needless_pass_by_value)] // Intentional: consumes slot to prevent reuse
    pub unsafe fn dealloc(&self, slot: Slot<T>) {
        // Drop the value in place
        // SAFETY: Caller guarantees slot is valid and occupied
        unsafe {
            ptr::drop_in_place((*(*slot.as_ptr()).value).as_mut_ptr());
        }
        // Return to freelist
        // SAFETY: Value dropped, slot valid
        unsafe { self.inner.dealloc_ptr(slot.as_ptr()) };
    }

    /// Deallocates a slot and returns the value without dropping it.
    ///
    /// # Performance
    ///
    /// O(n) where n = chunk count, due to chunk lookup. Typically 1-5 chunks.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from **this** slab (not a different slab)
    /// - No references to the slot's value may exist
    ///
    /// Note: Double-free is prevented at compile time (`Slot` is move-only).
    #[inline]
    #[allow(clippy::needless_pass_by_value)] // Intentional: consumes slot to prevent reuse
    pub unsafe fn dealloc_take(&self, slot: Slot<T>) -> T {
        // Move the value out
        // SAFETY: Caller guarantees slot is valid and occupied
        let value = unsafe { ptr::read((*slot.as_ptr()).value.as_ptr()) };
        // Return to freelist
        // SAFETY: Value moved out, slot valid
        unsafe { self.inner.dealloc_ptr(slot.as_ptr()) };
        value
    }
}

impl<T> fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            .field("capacity", &self.inner.capacity())
            .finish()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::{Borrow, BorrowMut};

    #[test]
    fn slab_basic() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<u64>::new(16) };

        let slot = slab.alloc(42);
        assert_eq!(*slot, 42);
        // SAFETY: slot was allocated from this slab
        unsafe { slab.dealloc(slot) };
    }

    #[test]
    fn slab_grows() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<u64>::new(4) };

        let mut slots = Vec::new();
        for i in 0..10 {
            slots.push(slab.alloc(i));
        }

        assert!(slab.capacity() >= 10);

        // Free all slots
        for slot in slots {
            // SAFETY: each slot was allocated from this slab
            unsafe { slab.dealloc(slot) };
        }
    }

    #[test]
    fn slot_deref_mut() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<String>::new(16) };
        let mut slot = slab.alloc("hello".to_string());
        slot.push_str(" world");
        assert_eq!(&*slot, "hello world");
        // SAFETY: slot was allocated from this slab
        unsafe { slab.dealloc(slot) };
    }

    #[test]
    fn slot_dealloc_take() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<String>::new(16) };
        let slot = slab.alloc("hello".to_string());

        // SAFETY: slot was allocated from this slab
        let value = unsafe { slab.dealloc_take(slot) };
        assert_eq!(value, "hello");
    }

    #[test]
    fn chunk_freelist_maintenance() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<u64>::new(2) };

        // Fill first chunk
        let s1 = slab.alloc(1);
        let s2 = slab.alloc(2);
        // Triggers growth
        let s3 = slab.alloc(3);

        // Free from first chunk — should add it back to available list
        // SAFETY: s1 was allocated from this slab
        unsafe { slab.dealloc(s1) };

        // Should reuse the freed slot
        let s4 = slab.alloc(4);

        // SAFETY: all slots were allocated from this slab
        unsafe {
            slab.dealloc(s2);
            slab.dealloc(s3);
            slab.dealloc(s4);
        }
    }

    #[test]
    fn slot_size() {
        // Raw Slot<T> is 8 bytes (one pointer)
        assert_eq!(std::mem::size_of::<Slot<u64>>(), 8);
    }

    #[test]
    fn borrow_traits() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<u64>::new(16) };
        let mut slot = slab.alloc(42);

        let borrowed: &u64 = slot.borrow();
        assert_eq!(*borrowed, 42);

        let borrowed_mut: &mut u64 = slot.borrow_mut();
        *borrowed_mut = 100;
        assert_eq!(*slot, 100);

        // SAFETY: slot was allocated from slab
        unsafe { slab.dealloc(slot) };
    }
}
