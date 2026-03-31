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
//! // SAFETY: caller guarantees slab contract (see struct docs)
//! let slab = unsafe { Slab::with_chunk_capacity(4096) };
//! let slot = slab.alloc(42u64);
//! assert_eq!(*slot, 42);
//! slab.free(slot);
//! ```

use core::cell::Cell;
use core::fmt;
use core::mem;

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::bounded::Slab as BoundedSlab;
use crate::shared::{SlotCell, SlotPtr};

// =============================================================================
// Claim
// =============================================================================

/// A claimed slot that has not yet been written to.
///
/// Created by [`Slab::claim()`]. Must be consumed via [`write()`](Self::write)
/// to complete the allocation. If dropped without calling `write()`, the slot
/// is returned to the freelist.
///
/// The `write()` method is `#[inline]`, enabling the compiler to potentially
/// optimize the value write as a placement new (constructing directly into
/// the slot memory).
pub struct Claim<'a, T> {
    slot_ptr: *mut SlotCell<T>,
    slab: &'a Slab<T>,
    chunk_idx: usize,
}

impl<T> Claim<'_, T> {
    /// Writes the value to the claimed slot and returns the [`SlotPtr`] handle.
    ///
    /// This consumes the claim. The value is written directly to the slot's
    /// memory, which may enable placement new optimization.
    #[inline]
    pub fn write(self, value: T) -> SlotPtr<T> {
        let slot_ptr = self.slot_ptr;
        // SAFETY: We own this slot from claim(), it's valid and vacant
        unsafe {
            (*slot_ptr).write_value(value);
        }
        // Don't run Drop - we're completing the allocation
        mem::forget(self);
        // SAFETY: slot_ptr is valid and now occupied
        unsafe { SlotPtr::from_ptr(slot_ptr) }
    }
}

impl<T> fmt::Debug for Claim<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Claim")
            .field("slot_ptr", &self.slot_ptr)
            .field("chunk_idx", &self.chunk_idx)
            .finish()
    }
}

impl<T> Drop for Claim<'_, T> {
    fn drop(&mut self) {
        // Abandoned claim - return slot to the correct chunk's freelist
        let chunk = self.slab.chunk(self.chunk_idx);
        let chunk_slab = &*chunk.inner;

        let free_head = chunk_slab.free_head.get();
        let was_full = free_head.is_null();

        // SAFETY: slot_ptr is valid and still vacant (never written to)
        unsafe {
            (*self.slot_ptr).set_next_free(free_head);
        }
        chunk_slab.free_head.set(self.slot_ptr);

        // If chunk was full, add it back to the available-space list
        if was_full {
            chunk.next_with_space.set(self.slab.head_with_space.get());
            self.slab.head_with_space.set(self.chunk_idx);
        }
    }
}

// =============================================================================
// Constants
// =============================================================================

/// Sentinel for chunk freelist
const CHUNK_NONE: usize = usize::MAX;

// =============================================================================
// ChunkEntry
// =============================================================================

/// Internal wrapper for a chunk in the growable slab.
struct ChunkEntry<T> {
    inner: Box<BoundedSlab<T>>,
    next_with_space: Cell<usize>,
}

// =============================================================================
// Slab
// =============================================================================

/// Growable slab allocator.
///
/// Uses independent chunks for growth — no copying when the slab grows.
///
/// Construction is `unsafe` — by creating a slab, you accept the contract:
///
/// - **Free everything you allocate.** Dropping the slab does NOT drop
///   values in occupied slots. Unfree'd slots leak silently.
/// - **Free from the same slab.** Passing a [`SlotPtr`] to a different
///   slab's `free()` corrupts the freelist.
/// - **Don't share across threads.** The slab is `!Send` and `!Sync`.
///
/// # Const Construction
///
/// ```ignore
/// // SAFETY: single slab per type, freed before thread exit
/// thread_local! {
///     static SLAB: Slab<MyType> = const { unsafe { Slab::new() } };
/// }
/// SLAB.with(|s| unsafe { s.init(4096) });
/// ```
///
/// For direct usage, prefer [`with_chunk_capacity()`](Self::with_chunk_capacity).
pub struct Slab<T> {
    chunks: core::cell::UnsafeCell<Vec<ChunkEntry<T>>>,
    chunk_capacity: Cell<usize>,
    head_with_space: Cell<usize>,
}

impl<T> Slab<T> {
    /// Creates an empty, uninitialized slab.
    ///
    /// This is a const function that performs no allocation. Call [`init()`](Self::init)
    /// to configure chunk capacity before use.
    ///
    /// For direct usage, prefer [`with_chunk_capacity()`](Self::with_chunk_capacity).
    ///
    /// # Safety
    ///
    /// See [struct-level safety contract](Self).
    ///
    /// # Example
    ///
    /// ```ignore
    /// // SAFETY: single slab per type, freed before thread exit
    /// thread_local! {
    ///     static SLAB: Slab<u64> = const { unsafe { Slab::new() } };
    /// }
    /// ```
    #[inline]
    pub const unsafe fn new() -> Self {
        Self {
            chunks: core::cell::UnsafeCell::new(Vec::new()),
            chunk_capacity: Cell::new(0),
            head_with_space: Cell::new(CHUNK_NONE),
        }
    }

    /// Creates a new slab with the given chunk capacity.
    ///
    /// Chunks are allocated on-demand when slots are requested.
    ///
    /// # Panics
    ///
    /// # Safety
    ///
    /// See [struct-level safety contract](Self).
    ///
    /// # Panics
    ///
    /// Panics if chunk_capacity is zero.
    #[inline]
    pub unsafe fn with_chunk_capacity(chunk_capacity: usize) -> Self {
        // SAFETY: caller upholds the slab contract
        let slab = unsafe { Self::new() };
        // SAFETY: caller upholds the slab contract
        unsafe { slab.init(chunk_capacity) };
        slab
    }

    /// Initializes the slab with the given chunk capacity.
    ///
    /// This configures the chunk parameters. Chunks are allocated on-demand
    /// when slots are requested. Must be called exactly once before any allocations.
    ///
    /// # Safety
    ///
    /// See [struct-level safety contract](Self).
    ///
    /// # Panics
    ///
    /// - Panics if the slab is already initialized (chunk_capacity > 0)
    /// - Panics if chunk_capacity is zero
    pub unsafe fn init(&self, chunk_capacity: usize) {
        assert!(self.chunk_capacity.get() == 0, "Slab already initialized");
        assert!(chunk_capacity > 0, "chunk_capacity must be non-zero");

        self.chunk_capacity.set(chunk_capacity);
    }

    /// Returns true if the slab has been initialized.
    #[inline]
    pub fn is_initialized(&self) -> bool {
        self.chunk_capacity.get() > 0
    }

    /// Returns the total capacity across all chunks.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.chunks().len() * self.chunk_capacity.get()
    }

    /// Returns the chunk capacity.
    #[inline]
    pub fn chunk_capacity(&self) -> usize {
        self.chunk_capacity.get()
    }

    #[inline]
    fn chunks(&self) -> &Vec<ChunkEntry<T>> {
        // SAFETY: !Sync prevents shared access across threads.
        // Only one thread can hold &self at a time.
        unsafe { &*self.chunks.get() }
    }

    #[inline]
    #[allow(clippy::mut_from_ref)]
    fn chunks_mut(&self) -> &mut Vec<ChunkEntry<T>> {
        // SAFETY: !Sync prevents shared access across threads.
        // Only one thread can hold &self at a time.
        unsafe { &mut *self.chunks.get() }
    }

    fn chunk(&self, chunk_idx: usize) -> &ChunkEntry<T> {
        let chunks = self.chunks();
        debug_assert!(chunk_idx < chunks.len());
        unsafe { chunks.get_unchecked(chunk_idx) }
    }

    /// Returns the number of allocated chunks.
    #[inline]
    pub fn chunk_count(&self) -> usize {
        self.chunks().len()
    }

    /// Returns `true` if `ptr` falls within any chunk's slot array.
    ///
    /// O(chunks) scan. Typically 1–5 chunks. Used in `debug_assert!`
    /// to validate provenance.
    #[doc(hidden)]
    pub fn contains_ptr(&self, ptr: *const ()) -> bool {
        let chunks = self.chunks();
        for chunk in chunks {
            let chunk_slab = &*chunk.inner;
            if chunk_slab.contains_ptr(ptr) {
                return true;
            }
        }
        false
    }

    /// Ensures at least `count` chunks are allocated.
    ///
    /// No-op if the slab already has `count` or more chunks. Only allocates
    /// the difference.
    pub fn reserve_chunks(&self, count: usize) {
        let current = self.chunks().len();
        for _ in current..count {
            self.grow();
        }
    }

    /// Grows the slab by adding a single new chunk.
    fn grow(&self) {
        let chunks = self.chunks_mut();
        let chunk_idx = chunks.len();
        // SAFETY: The outer slab's construction was unsafe, so the caller
        // already accepted the slab contract. Inner chunks inherit that contract.
        let inner = Box::new(unsafe { BoundedSlab::with_capacity(self.chunk_capacity.get()) });

        let entry = ChunkEntry {
            inner,
            next_with_space: Cell::new(self.head_with_space.get()),
        };

        chunks.push(entry);
        self.head_with_space.set(chunk_idx);
    }

    // =========================================================================
    // Allocation API
    // =========================================================================

    /// Claims a slot from the freelist without writing a value.
    ///
    /// Always succeeds — grows the slab if needed. The returned [`Claim`]
    /// must be consumed via [`Claim::write()`] to complete the allocation.
    ///
    /// This two-phase allocation enables placement new optimization: the
    /// value can be constructed directly into the slot memory.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::unbounded::Slab;
    ///
    /// // SAFETY: caller guarantees slab contract (see struct docs)
    /// let slab = unsafe { Slab::with_chunk_capacity(16) };
    /// let claim = slab.claim();
    /// let slot = claim.write(42u64);
    /// assert_eq!(*slot, 42);
    /// slab.free(slot);
    /// ```
    #[inline]
    pub fn claim(&self) -> Claim<'_, T> {
        let (slot_ptr, chunk_idx) = self.claim_ptr();
        Claim {
            slot_ptr,
            slab: self,
            chunk_idx,
        }
    }

    /// Claims a slot from the freelist, returning the raw pointer and chunk index.
    ///
    /// Always succeeds — grows the slab if needed. This is a low-level API for
    /// macro-generated code that needs to escape TLS closures.
    ///
    /// # Safety Contract
    ///
    /// The caller MUST either:
    /// - Write a value to the slot and use it as an allocated slot, OR
    /// - Return the pointer to the freelist via `free_ptr()` if abandoning
    #[doc(hidden)]
    #[inline]
    pub fn claim_ptr(&self) -> (*mut SlotCell<T>, usize) {
        // Ensure we have space (grow if needed)
        if self.head_with_space.get() == CHUNK_NONE {
            self.grow();
        }

        // Get the chunk with space
        let chunk_idx = self.head_with_space.get();
        let chunk = self.chunk(chunk_idx);
        let chunk_slab = &*chunk.inner;

        // Load freelist head pointer from chunk
        let slot_ptr = chunk_slab.free_head.get();
        debug_assert!(!slot_ptr.is_null(), "chunk on freelist has no free slots");

        // SAFETY: slot_ptr came from the freelist. Slot is vacant, so next_free is active.
        let next_free = unsafe { (*slot_ptr).get_next_free() };

        // Update chunk's freelist head
        chunk_slab.free_head.set(next_free);

        // If chunk is now full, remove from slab's available-chunk list
        if next_free.is_null() {
            self.head_with_space.set(chunk.next_with_space.get());
        }

        (slot_ptr, chunk_idx)
    }

    /// Allocates a slot and writes the value.
    ///
    /// Always succeeds — grows the slab if needed.
    #[inline]
    pub fn alloc(&self, value: T) -> SlotPtr<T> {
        self.claim().write(value)
    }

    /// Frees a slot, dropping the value and returning storage to the freelist.
    ///
    /// Consumes the handle — the slot cannot be used after this call.
    ///
    /// # Performance
    ///
    /// O(n) where n = chunk count, due to chunk lookup. Typically 1-5 chunks.
    #[inline]
    #[allow(clippy::needless_pass_by_value)]
    pub fn free(&self, slot: SlotPtr<T>) {
        let slot_ptr = slot.into_ptr();
        debug_assert!(
            self.contains_ptr(slot_ptr as *const ()),
            "slot was not allocated from this slab"
        );
        // SAFETY: Caller guarantees slot is valid and occupied
        unsafe {
            (*slot_ptr).drop_value_in_place();
            self.free_ptr(slot_ptr);
        }
    }

    /// Frees a slot and returns the value without dropping it.
    ///
    /// Consumes the handle — the slot cannot be used after this call.
    ///
    /// # Performance
    ///
    /// O(n) where n = chunk count, due to chunk lookup. Typically 1-5 chunks.
    #[inline]
    #[allow(clippy::needless_pass_by_value)]
    pub fn take(&self, slot: SlotPtr<T>) -> T {
        let slot_ptr = slot.into_ptr();
        debug_assert!(
            self.contains_ptr(slot_ptr as *const ()),
            "slot was not allocated from this slab"
        );
        // SAFETY: Caller guarantees slot is valid and occupied
        unsafe {
            let value = (*slot_ptr).read_value();
            self.free_ptr(slot_ptr);
            value
        }
    }

    /// Returns a slot to the freelist by pointer.
    ///
    /// Does NOT drop the value — caller must drop before calling.
    /// Finds the owning chunk via linear scan (typically 1-5 chunks).
    ///
    /// # Safety
    ///
    /// - `slot_ptr` must point to a slot within this slab
    /// - Value must already be dropped or moved out
    #[doc(hidden)]
    pub unsafe fn free_ptr(&self, slot_ptr: *mut SlotCell<T>) {
        let chunks = self.chunks();
        let cap = self.chunk_capacity.get();

        // Find which chunk owns this pointer
        for (chunk_idx, chunk) in chunks.iter().enumerate() {
            let chunk_slab = &*chunk.inner;
            let base = chunk_slab.slots_ptr();
            let end = base.wrapping_add(cap);

            if slot_ptr >= base && slot_ptr < end {
                let free_head = chunk_slab.free_head.get();
                let was_full = free_head.is_null();

                // SAFETY: slot_ptr is within this chunk's range
                unsafe {
                    (*slot_ptr).set_next_free(free_head);
                }
                chunk_slab.free_head.set(slot_ptr);

                if was_full {
                    chunk.next_with_space.set(self.head_with_space.get());
                    self.head_with_space.set(chunk_idx);
                }
                return;
            }
        }

        unreachable!("free_ptr: slot_ptr not found in any chunk");
    }
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        // SAFETY: Default creates an uninitialized slab — caller must
        // call init() and uphold the slab contract before use.
        unsafe { Self::new() }
    }
}

impl<T> fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            .field("capacity", &self.capacity())
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
        let slab = unsafe { Slab::<u64>::with_chunk_capacity(16) };

        let slot = slab.alloc(42);
        assert_eq!(*slot, 42);
        slab.free(slot);
    }

    #[test]
    fn slab_grows() {
        let slab = unsafe { Slab::<u64>::with_chunk_capacity(4) };

        let mut slots = Vec::new();
        for i in 0..10 {
            slots.push(slab.alloc(i));
        }

        assert!(slab.capacity() >= 10);

        for slot in slots {
            slab.free(slot);
        }
    }

    #[test]
    fn slot_deref_mut() {
        let slab = unsafe { Slab::<String>::with_chunk_capacity(16) };
        let mut slot = slab.alloc("hello".to_string());
        slot.push_str(" world");
        assert_eq!(&*slot, "hello world");
        slab.free(slot);
    }

    #[test]
    fn slot_dealloc_take() {
        let slab = unsafe { Slab::<String>::with_chunk_capacity(16) };
        let slot = slab.alloc("hello".to_string());

        let value = slab.take(slot);
        assert_eq!(value, "hello");
    }

    #[test]
    fn chunk_freelist_maintenance() {
        let slab = unsafe { Slab::<u64>::with_chunk_capacity(2) };

        // Fill first chunk
        let s1 = slab.alloc(1);
        let s2 = slab.alloc(2);
        // Triggers growth
        let s3 = slab.alloc(3);

        // Free from first chunk — should add it back to available list
        slab.free(s1);

        // Should reuse the freed slot
        let s4 = slab.alloc(4);

        slab.free(s2);
        slab.free(s3);
        slab.free(s4);
    }

    #[test]
    fn slot_size() {
        assert_eq!(std::mem::size_of::<SlotPtr<u64>>(), 8);
    }

    #[test]
    fn borrow_traits() {
        let slab = unsafe { Slab::<u64>::with_chunk_capacity(16) };
        let mut slot = slab.alloc(42);

        let borrowed: &u64 = slot.borrow();
        assert_eq!(*borrowed, 42);

        let borrowed_mut: &mut u64 = slot.borrow_mut();
        *borrowed_mut = 100;
        assert_eq!(*slot, 100);

        slab.free(slot);
    }
}
