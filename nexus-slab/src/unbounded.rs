//! Growable slab allocator internals.
//!
//! This module contains the internal implementation for unbounded (growable) slabs.
//! Use the [`create_allocator!`](crate::create_allocator) macro to create
//! a type-safe allocator with RAII slots.
//!
//! # Example
//!
//! ```
//! use nexus_slab::create_allocator;
//!
//! create_allocator!(my_alloc, u64);
//!
//! my_alloc::init()
//!     .unbounded()
//!     .chunk_capacity(4096)
//!     .build();
//!
//! let slot = my_alloc::insert(42);
//! assert_eq!(*slot, 42);
//! // slot drops, storage freed
//! ```

use std::cell::{Cell, UnsafeCell};
use std::mem::ManuallyDrop;

use crate::Key;
use crate::bounded::BoundedSlabInner;
use crate::shared::{ClaimedSlot, SLOT_NONE, SlotCell, VTable};

// =============================================================================
// Constants
// =============================================================================

/// Sentinel for chunk freelist
const CHUNK_NONE: u32 = u32::MAX;

// =============================================================================
// ChunkEntry
// =============================================================================

/// Internal wrapper for a chunk in the growable slab.
pub(crate) struct ChunkEntry<T> {
    pub(crate) inner: ManuallyDrop<Box<BoundedSlabInner<T>>>,
    pub(crate) next_with_space: Cell<u32>,
}

// =============================================================================
// SlabInner
// =============================================================================

/// Internal state for a growable slab.
///
/// This is the storage backend for unbounded allocators. Use
/// [`create_allocator!`](crate::create_allocator) to create a user-facing API.
#[doc(hidden)]
pub struct SlabInner<T> {
    chunks: UnsafeCell<Vec<ChunkEntry<T>>>,
    chunk_shift: u32,
    chunk_mask: u32,
    head_with_space: Cell<u32>,
    chunk_capacity: u32,
}

impl<T> SlabInner<T> {
    /// Creates a VTable for this unbounded slab.
    ///
    /// The returned VTable has `inner` set to null. Call `set_inner` after
    /// the slab is at a stable address (e.g., after leaking).
    #[inline]
    pub fn vtable() -> VTable<T> {
        VTable::new(
            unbounded_try_claim::<T>,
            unbounded_free::<T>,
            unbounded_slot_ptr::<T>,
            unbounded_contains_key::<T>,
        )
    }

    /// Creates a new SlabInner with the given chunk capacity.
    #[doc(hidden)]
    pub fn with_chunk_capacity(chunk_capacity: usize) -> Self {
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
    pub fn is_empty(&self) -> bool {
        self.chunks().iter().all(|c| c.inner.is_empty())
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

    /// Grows the slab by adding a new chunk.
    #[doc(hidden)]
    pub fn grow(&self) {
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

    /// Returns the total capacity across all chunks.
    #[doc(hidden)]
    pub fn capacity(&self) -> usize {
        self.chunks().len() * self.chunk_capacity as usize
    }
}

// Note: No Drop impl - this is leaked and never dropped

// =============================================================================
// VTable Functions for Unbounded Slab
// =============================================================================

/// Claims a slot from an unbounded slab.
///
/// Grows automatically if needed (always succeeds unless OOM).
/// Returns `Some` always (never fails for unbounded), but uses `Option` for
/// vtable compatibility with bounded slabs which can fail.
///
/// # Safety
/// `inner` must be a valid `*mut SlabInner<T>`.
#[allow(clippy::unnecessary_wraps)]
unsafe fn unbounded_try_claim<T>(inner: *mut ()) -> Option<ClaimedSlot> {
    let inner = unsafe { &*(inner as *mut SlabInner<T>) };

    // Ensure we have space (grow if needed)
    if inner.head_with_space.get() == CHUNK_NONE {
        inner.grow();
    }

    // Group all loads together first
    let chunk_idx = inner.head_with_space.get();
    let chunk = inner.chunk(chunk_idx);
    let chunk_inner = &*chunk.inner;

    let free_head = chunk_inner.free_head.get();
    debug_assert!(free_head != SLOT_NONE);

    let slot = chunk_inner.slot(free_head);
    let next_free = slot.claim_next_free();

    // Prepare return value before stores
    let global_idx = inner.encode(chunk_idx, free_head);
    let slot_ptr = (slot as *const SlotCell<T>).cast_mut() as *mut ();
    let key = Key::new(global_idx);

    // All stores grouped at end:
    // 1. Update chunk's freelist
    chunk_inner.free_head.set(next_free);

    // 2. If chunk is now full, remove from slab's available-chunk list
    //    Use next_free directly instead of is_full() - avoids redundant load
    if next_free == SLOT_NONE {
        inner.head_with_space.set(chunk.next_with_space.get());
    }

    Some(ClaimedSlot { slot_ptr, key })
}

/// Frees a slot in an unbounded slab.
///
/// Handles chunk freelist maintenance - if the chunk was full, adds it back.
/// Does NOT drop the value - caller must drop before calling.
///
/// # Safety
/// - `inner` must be a valid `*mut SlabInner<T>`
/// - `key` must refer to a previously claimed slot
/// - Value must already be dropped
unsafe fn unbounded_free<T>(inner: *mut (), key: Key) {
    let inner = unsafe { &*(inner as *mut SlabInner<T>) };
    let (chunk_idx, local_idx) = inner.decode(key.index());

    // Group all loads together
    let chunk = inner.chunk(chunk_idx);
    let chunk_inner = &*chunk.inner;
    let slot = chunk_inner.slot(local_idx);

    // Single load for free_head - also tells us if chunk was full
    // (is_full() would redundantly load this again)
    let free_head = chunk_inner.free_head.get();
    let was_full = free_head == SLOT_NONE;

    // Stores grouped together:
    // 1. Mark slot as vacant (stamp write)
    slot.set_vacant(free_head);

    // 2. Update chunk's freelist head
    chunk_inner.free_head.set(local_idx);

    // 3. If chunk was full, add it back to slab's available-chunk list
    if was_full {
        chunk.next_with_space.set(inner.head_with_space.get());
        inner.head_with_space.set(chunk_idx);
    }
}

/// Gets the slot pointer for a key in an unbounded slab.
///
/// # Safety
/// - `inner` must be a valid `*const SlabInner<T>`
/// - `key` must be within bounds (caller's responsibility)
unsafe fn unbounded_slot_ptr<T>(inner: *const (), key: Key) -> *mut () {
    let inner = unsafe { &*(inner as *const SlabInner<T>) };
    let (chunk_idx, local_idx) = inner.decode(key.index());
    let chunk = inner.chunk(chunk_idx);
    let slot = chunk.inner.slot(local_idx);
    (slot as *const SlotCell<T>).cast_mut() as *mut ()
}

/// Checks if a key is valid and occupied in an unbounded slab.
///
/// # Safety
/// `inner` must be a valid `*const SlabInner<T>`.
unsafe fn unbounded_contains_key<T>(inner: *const (), key: Key) -> bool {
    let inner = unsafe { &*(inner as *const SlabInner<T>) };
    let (chunk_idx, local_idx) = inner.decode(key.index());

    if (chunk_idx as usize) >= inner.chunks().len() {
        return false;
    }

    let chunk = inner.chunk(chunk_idx);
    if local_idx >= chunk.inner.capacity {
        return false;
    }

    chunk.inner.slot(local_idx).is_occupied()
}
