//! Shared internals for bounded and unbounded slab implementations.

use std::mem::{ManuallyDrop, MaybeUninit};

// =============================================================================
// Constants
// =============================================================================

/// Sentinel for Key::NONE. Maximum valid key index.
pub const SLOT_NONE: u32 = (1 << 31) - 1;

// =============================================================================
// SlotCell
// =============================================================================

/// SLUB-style slot: freelist pointer overlaid on value storage.
///
/// When vacant: `next_free` is active — points to next free slot (or null).
/// When occupied: `value` is active — contains the user's `T`.
///
/// These fields occupy the SAME bytes. Writing `value` overwrites `next_free`
/// and vice versa. There is no header, no tag, no sentinel — the Slot RAII
/// handle is the proof of occupancy.
///
/// Size: `max(8, size_of::<T>())`.
#[repr(C)]
pub union SlotCell<T> {
    /// Points to next free slot (or null). Active when slot is vacant.
    #[doc(hidden)]
    pub next_free: *mut SlotCell<T>,
    /// Contains the user's `T`. Active when slot is occupied.
    #[doc(hidden)]
    pub value: ManuallyDrop<MaybeUninit<T>>,
}

impl<T> SlotCell<T> {
    /// Creates a new vacant slot with the given next_free pointer.
    #[inline]
    pub(crate) fn vacant(next_free: *mut SlotCell<T>) -> Self {
        SlotCell { next_free }
    }
}
