//! Shared internals for bounded and unbounded slab implementations.

use std::borrow::{Borrow, BorrowMut};
use std::cell::Cell;
use std::fmt;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::{Deref, DerefMut};

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

// =============================================================================
// Slot<T> — Raw Pointer Wrapper
// =============================================================================

/// Raw slot handle — pointer wrapper, NOT RAII.
///
/// `Slot<T>` is a thin wrapper around a pointer to a [`SlotCell<T>`]. It is
/// analogous to `malloc` returning a pointer: the caller owns the memory and
/// must explicitly free it via [`Slab::free()`](crate::bounded::Slab::free).
///
/// # Size
///
/// 8 bytes (one pointer).
///
/// # Thread Safety
///
/// `Slot` is `!Send` and `!Sync`. It must only be used from the thread that
/// created it.
///
/// # No Drop
///
/// Unlike [`BoxSlot`](crate::alloc::BoxSlot), `Slot` does NOT deallocate on
/// drop. Forgetting to call `free()` will leak the slot.
///
/// # Borrow Traits
///
/// `Slot<T>` implements `Borrow<T>` and `BorrowMut<T>`, enabling use as
/// HashMap keys that borrow `T` for lookups.
#[repr(transparent)]
pub struct Slot<T>(*mut SlotCell<T>);

impl<T> Slot<T> {
    /// Creates a slot from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid pointer to an occupied `SlotCell<T>` within a slab.
    #[inline]
    pub unsafe fn from_ptr(ptr: *mut SlotCell<T>) -> Self {
        Slot(ptr)
    }

    /// Returns the raw pointer to the slot cell.
    #[inline]
    pub fn as_ptr(&self) -> *mut SlotCell<T> {
        self.0
    }
}

impl<T> Deref for Slot<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: Slot was created from a valid, occupied SlotCell.
        // Union field `value` is active because the slot is occupied.
        unsafe { (*self.0).value.assume_init_ref() }
    }
}

impl<T> DerefMut for Slot<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: We have &mut self, guaranteeing exclusive access.
        // Union field `value` is active because the slot is occupied.
        unsafe { (*(*self.0).value).assume_init_mut() }
    }
}

impl<T> AsRef<T> for Slot<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T> AsMut<T> for Slot<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T> Borrow<T> for Slot<T> {
    #[inline]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T> BorrowMut<T> for Slot<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self
    }
}

// Slot is intentionally NOT Clone/Copy.
// Move-only semantics prevent double-free at compile time.

impl<T: fmt::Debug> fmt::Debug for Slot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slot").field("value", &**self).finish()
    }
}
