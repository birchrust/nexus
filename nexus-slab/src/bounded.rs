//! Fixed-capacity slab allocator internals.
//!
//! This module contains the internal implementation for bounded slabs.
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
//! my_alloc::init().bounded(1024).build();
//!
//! let slot = my_alloc::insert(42);
//! assert_eq!(*slot, 42);
//! // slot drops, storage freed
//! ```

use std::cell::Cell;
use std::mem::ManuallyDrop;

use crate::shared::{ClaimedSlot, SLOT_NONE, SlotCell, VTable};
use crate::Key;

// =============================================================================
// BoundedSlabInner
// =============================================================================

/// Internal state for a fixed-capacity slab.
///
/// This is the storage backend for bounded allocators. Use
/// [`create_allocator!`](crate::create_allocator) to create a user-facing API.
#[doc(hidden)]
pub struct BoundedSlabInner<T> {
    pub(crate) slots: ManuallyDrop<Vec<SlotCell<T>>>,
    pub(crate) capacity: u32,
    pub(crate) free_head: Cell<u32>,
}

impl<T> BoundedSlabInner<T> {
    /// Creates a new bounded slab inner with the given capacity.
    pub fn with_capacity(capacity: u32) -> Self {
        assert!(capacity > 0, "capacity must be non-zero");
        assert!(capacity < SLOT_NONE, "capacity exceeds maximum");

        let mut slots: Vec<SlotCell<T>> = Vec::with_capacity(capacity as usize);

        for i in 0..capacity {
            let next = if i + 1 < capacity { i + 1 } else { SLOT_NONE };
            slots.push(SlotCell::new_vacant(next));
        }

        Self {
            slots: ManuallyDrop::new(slots),
            capacity,
            free_head: Cell::new(0),
        }
    }

    /// Creates a VTable for this bounded slab.
    ///
    /// The returned VTable has `inner` set to null. Call `set_inner` after
    /// the slab is at a stable address (e.g., after leaking).
    #[inline]
    pub fn vtable() -> VTable<T> {
        VTable::new(
            bounded_try_claim::<T>,
            bounded_free::<T>,
            bounded_slot_ptr::<T>,
            bounded_contains_key::<T>,
        )
    }

    /// Returns the current length (number of occupied slots).
    ///
    /// This scans all slots - O(n). Use only for diagnostics/shutdown, not hot path.
    pub fn len(&self) -> u32 {
        self.slots.iter().filter(|s| SlotCell::is_occupied(s)).count() as u32
    }

    /// Returns true if no slots are occupied.
    ///
    /// This scans slots - O(n). Use only for diagnostics/shutdown, not hot path.
    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(SlotCell::is_vacant)
    }

    /// Returns the capacity.
    #[inline]
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    #[inline]
    pub(crate) fn slot(&self, index: u32) -> &SlotCell<T> {
        debug_assert!(index < self.capacity);
        unsafe { self.slots.get_unchecked(index as usize) }
    }
}

// Note: No Drop impl - this is leaked and never dropped

// =============================================================================
// VTable Functions for Bounded Slab
// =============================================================================

/// Claims a slot from a bounded slab.
///
/// # Safety
/// `inner` must be a valid `*mut BoundedSlabInner<T>`.
unsafe fn bounded_try_claim<T>(inner: *mut ()) -> Option<ClaimedSlot> {
    let inner = unsafe { &*(inner as *mut BoundedSlabInner<T>) };
    let free_head = inner.free_head.get();

    if free_head == SLOT_NONE {
        return None;
    }

    let slot = inner.slot(free_head);
    let next_free = slot.claim_next_free();

    // Prepare return value before the store - keeps all loads grouped,
    // then single store at end before return
    let slot_ptr = (slot as *const SlotCell<T>).cast_mut() as *mut ();
    let key = Key::new(free_head);

    // Single store to freelist - last operation before return
    inner.free_head.set(next_free);

    Some(ClaimedSlot { slot_ptr, key })
}

/// Frees a slot in a bounded slab.
///
/// Does NOT drop the value - caller must drop before calling.
///
/// # Safety
/// - `inner` must be a valid `*mut BoundedSlabInner<T>`
/// - `key` must refer to a previously claimed slot
/// - Value must already be dropped
unsafe fn bounded_free<T>(inner: *mut (), key: Key) {
    let inner = unsafe { &*(inner as *mut BoundedSlabInner<T>) };
    let index = key.index();

    // Group loads together
    let slot = inner.slot(index);
    let free_head = inner.free_head.get();

    // Stores: stamp first (marks slot as vacant), then freelist update
    slot.set_vacant(free_head);
    inner.free_head.set(index);
}

/// Gets the slot pointer for a key in a bounded slab.
///
/// # Safety
/// - `inner` must be a valid `*const BoundedSlabInner<T>`
/// - `key` must be within bounds (caller's responsibility)
unsafe fn bounded_slot_ptr<T>(inner: *const (), key: Key) -> *mut () {
    let inner = unsafe { &*(inner as *const BoundedSlabInner<T>) };
    let slot = inner.slot(key.index());
    (slot as *const SlotCell<T>).cast_mut() as *mut ()
}

/// Checks if a key is valid and occupied in a bounded slab.
///
/// # Safety
/// `inner` must be a valid `*const BoundedSlabInner<T>`.
unsafe fn bounded_contains_key<T>(inner: *const (), key: Key) -> bool {
    let inner = unsafe { &*(inner as *const BoundedSlabInner<T>) };
    let index = key.index();
    if index >= inner.capacity {
        return false;
    }
    inner.slot(index).is_occupied()
}
