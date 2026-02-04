//! Shared internals for bounded and unbounded slab implementations.

use std::cell::Cell;
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

// =============================================================================
// RcInner
// =============================================================================

/// Strong/weak refcount header + value for reference-counted slab allocation.
///
/// `ManuallyDrop<T>` ensures `Slot`'s `drop_in_place` on this type is a no-op
/// for `T` — `RcSlot` manages `T`'s lifetime manually (drops when strong hits 0).
///
/// # Layout
///
/// Two `u32` refcounts (8 bytes) followed by the value. Zero padding waste
/// for types with alignment >= 4.
#[repr(C)]
pub struct RcInner<T> {
    strong: Cell<u32>,
    weak: Cell<u32>,
    value: ManuallyDrop<T>,
}

impl<T> RcInner<T> {
    /// Creates a new `RcInner` with strong=1, weak=1 (implicit weak for strong > 0).
    #[inline]
    pub fn new(value: T) -> Self {
        RcInner {
            strong: Cell::new(1),
            weak: Cell::new(1),
            value: ManuallyDrop::new(value),
        }
    }

    /// Returns the strong reference count.
    #[inline]
    pub fn strong(&self) -> u32 {
        self.strong.get()
    }

    /// Returns the weak reference count (includes implicit +1 while strong > 0).
    #[inline]
    pub fn weak(&self) -> u32 {
        self.weak.get()
    }

    /// Sets the strong count.
    #[inline]
    pub(crate) fn set_strong(&self, val: u32) {
        self.strong.set(val);
    }

    /// Sets the weak count.
    #[inline]
    pub(crate) fn set_weak(&self, val: u32) {
        self.weak.set(val);
    }

    /// Returns a reference to the value.
    #[inline]
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Consumes self and returns the inner value, bypassing `ManuallyDrop`.
    ///
    /// Used by `try_new` error path to recover the value from a failed allocation.
    #[inline]
    pub(crate) fn into_value(self) -> T {
        ManuallyDrop::into_inner(self.value)
    }

    /// Returns a mutable reference to the `ManuallyDrop<T>` value field.
    ///
    /// # Safety
    ///
    /// Caller must ensure exclusive access and that the value is still alive.
    #[inline]
    pub(crate) unsafe fn value_manual_drop_mut(&mut self) -> &mut ManuallyDrop<T> {
        &mut self.value
    }
}
