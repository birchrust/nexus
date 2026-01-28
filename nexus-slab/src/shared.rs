//! Shared internals for bounded and unbounded slab implementations.

use std::cell::{Cell, UnsafeCell};
use std::mem::MaybeUninit;

// =============================================================================
// Constants
// =============================================================================

/// Bit 31: Vacant flag (1 = vacant)
pub(crate) const VACANT_BIT: u32 = 1 << 31;

/// Bit 30: Borrowed flag (1 = borrowed)
pub(crate) const BORROWED_BIT: u32 = 1 << 30;

/// Mask for next_free index (bits 0-29)
pub(crate) const INDEX_MASK: u32 = (1 << 30) - 1;

/// Sentinel for end of freelist (~1 billion max capacity)
pub(crate) const SLOT_NONE: u32 = INDEX_MASK;

// =============================================================================
// SlotCell
// =============================================================================

/// Internal slot storage with tag for state tracking.
///
/// Tag encoding (32-bit):
/// - Bit 31: Vacant flag (1 = vacant, 0 = occupied)
/// - Bit 30: Borrowed flag (1 = borrowed, 0 = available) - only when occupied
/// - Bits 0-29: When vacant, next free slot index
#[repr(C)]
pub(crate) struct SlotCell<T> {
    tag: Cell<u32>,
    pub(crate) value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> SlotCell<T> {
    pub(crate) fn new_vacant(next_free: u32) -> Self {
        Self {
            tag: Cell::new(VACANT_BIT | (next_free & INDEX_MASK)),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    #[inline]
    pub(crate) fn is_vacant(&self) -> bool {
        self.tag.get() & VACANT_BIT != 0
    }

    #[inline]
    pub(crate) fn is_occupied(&self) -> bool {
        !self.is_vacant()
    }

    #[inline]
    pub(crate) fn is_borrowed(&self) -> bool {
        self.tag.get() == BORROWED_BIT
    }

    /// Returns true if slot is occupied and not borrowed.
    /// Branchless: only tag == 0 means available.
    #[inline]
    pub(crate) fn is_available(&self) -> bool {
        self.tag.get() == 0
    }

    #[inline]
    pub(crate) fn next_free(&self) -> u32 {
        debug_assert!(self.is_vacant(), "next_free called on occupied slot");
        self.tag.get() & INDEX_MASK
    }

    #[inline]
    pub(crate) fn set_occupied(&self) {
        self.tag.set(0);
    }

    #[inline]
    pub(crate) fn set_vacant(&self, next_free: u32) {
        self.tag.set(VACANT_BIT | (next_free & INDEX_MASK));
    }

    #[inline]
    pub(crate) fn set_borrowed(&self) {
        debug_assert!(self.is_occupied(), "set_borrowed on vacant slot");
        debug_assert!(!self.is_borrowed(), "already borrowed");
        self.tag.set(BORROWED_BIT);
    }

    #[inline]
    pub(crate) fn clear_borrowed(&self) {
        debug_assert!(self.is_borrowed(), "clear_borrowed on non-borrowed slot");
        self.tag.set(0);
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
