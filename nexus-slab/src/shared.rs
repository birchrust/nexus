//! Shared internals for bounded and unbounded slab implementations.

use std::cell::{Cell, UnsafeCell};
use std::mem::MaybeUninit;

// =============================================================================
// Constants
// =============================================================================

/// Vacant flag - bit 63 of stamp
const VACANT_BIT: u64 = 1 << 63;

/// Mask for key (lower 32 bits of stamp)
const KEY_MASK: u64 = 0x0000_0000_FFFF_FFFF;

/// Mask for next_free index within state (bits 29-0 after shifting)
const INDEX_MASK: u64 = (1 << 30) - 1;

/// Sentinel for end of freelist (~1 billion max capacity).
///
/// This is the max 30-bit value, limiting addressable slots to ~1 billion.
pub const SLOT_NONE: u32 = INDEX_MASK as u32;

// =============================================================================
// SlotCell
// =============================================================================

/// Internal slot storage with stamp for state tracking and key storage.
///
/// Stamp encoding (64-bit):
/// - Bits 63-32: State
///   - Bit 63: Vacant flag (1 = vacant, 0 = occupied)
///   - Bits 61-32: When vacant, next free slot index (30 bits)
/// - Bits 31-0: Key (stored when slot is claimed, valid regardless of state)
#[repr(C)]
pub(crate) struct SlotCell<T> {
    stamp: Cell<u64>,
    pub(crate) value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> SlotCell<T> {
    pub(crate) fn new_vacant(next_free: u32) -> Self {
        Self {
            stamp: Cell::new(VACANT_BIT | ((next_free as u64 & INDEX_MASK) << 32)),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    #[inline]
    pub(crate) fn is_vacant(&self) -> bool {
        self.stamp.get() & VACANT_BIT != 0
    }

    #[inline]
    pub(crate) fn is_occupied(&self) -> bool {
        !self.is_vacant()
    }

    #[inline]
    pub(crate) fn next_free(&self) -> u32 {
        debug_assert!(self.is_vacant(), "next_free called on occupied slot");
        ((self.stamp.get() >> 32) & INDEX_MASK) as u32
    }

    /// Returns the key stored in the stamp.
    /// Valid regardless of vacant/occupied state (key is set when slot is claimed).
    #[inline]
    pub(crate) fn key_from_stamp(&self) -> u32 {
        (self.stamp.get() & KEY_MASK) as u32
    }

    /// Sets the key in the stamp without changing state bits.
    /// Called when claiming a slot, before marking occupied.
    #[inline]
    pub(crate) fn set_key(&self, key: u32) {
        self.stamp.set((self.stamp.get() & !KEY_MASK) | key as u64);
    }

    /// Marks slot as occupied by clearing state bits. Preserves key.
    #[inline]
    pub(crate) fn set_occupied(&self) {
        self.stamp.set(self.stamp.get() & KEY_MASK);
    }

    /// Marks slot as vacant with given next_free index. Clobbers key.
    #[inline]
    pub(crate) fn set_vacant(&self, next_free: u32) {
        self.stamp
            .set(VACANT_BIT | ((next_free as u64 & INDEX_MASK) << 32));
    }

    /// # Safety
    /// Slot must be occupied.
    #[inline]
    pub(crate) unsafe fn value_ref(&self) -> &T {
        unsafe { (*self.value.get()).assume_init_ref() }
    }

    /// # Safety
    /// Slot must be occupied and caller must have exclusive access.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub(crate) unsafe fn value_mut(&self) -> &mut T {
        unsafe { (*self.value.get()).assume_init_mut() }
    }
}
