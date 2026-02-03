//! Shared internals for bounded and unbounded slab implementations.

use std::cell::{Cell, UnsafeCell};
use std::mem::MaybeUninit;

// =============================================================================
// Constants
// =============================================================================

/// Vacant flag - bit 31 of stamp (u32)
pub const VACANT_BIT: u32 = 1 << 31;

/// Mask for key (lower 31 bits of stamp)
pub(crate) const KEY_MASK: u32 = (1 << 31) - 1;

/// Sentinel for Key::NONE. Maximum valid key index.
pub const SLOT_NONE: u32 = KEY_MASK;

/// Offset in bytes from SlotCell pointer to value field.
/// Layout: next_free (8) + stamp (4) + _pad (4) = 16 bytes before value.
pub const VALUE_OFFSET: usize = 16;

// =============================================================================
// SlotCell
// =============================================================================

/// Internal slot storage with pointer-based freelist.
///
/// Layout (repr(C)):
/// - `next_free`: 8 bytes - pointer to next free slot (NULL when occupied)
/// - `stamp`: 4 bytes - vacant bit (31) + key (bits 30-0)
/// - `_pad`: 4 bytes - alignment padding
/// - `value`: T - the actual value
///
/// Total header: 16 bytes before value.
#[repr(C)]
pub struct SlotCell<T> {
    /// Pointer to next free slot. NULL when slot is occupied.
    next_free: Cell<*mut SlotCell<T>>,
    /// Stamp encoding: bit 31 = vacant flag, bits 30-0 = key
    stamp: Cell<u32>,
    /// Explicit padding for consistent layout
    _pad: u32,
    /// The value storage.
    pub value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> SlotCell<T> {
    /// Creates a new vacant slot with the given next_free pointer.
    pub(crate) fn new_vacant(next_free: *mut SlotCell<T>) -> Self {
        Self {
            next_free: Cell::new(next_free),
            stamp: Cell::new(VACANT_BIT),
            _pad: 0,
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Returns `true` if the slot is vacant.
    #[inline]
    pub fn is_vacant(&self) -> bool {
        self.stamp.get() & VACANT_BIT != 0
    }

    /// Returns `true` if the slot is occupied.
    #[inline]
    pub fn is_occupied(&self) -> bool {
        !self.is_vacant()
    }

    /// Returns a pointer to the value.
    ///
    /// # Safety
    ///
    /// The slot must be occupied. Caller must ensure no mutable references exist.
    #[inline]
    pub unsafe fn value_ptr(&self) -> *const T {
        unsafe { (*self.value.get()).as_ptr() }
    }

    /// Returns a mutable pointer to the value.
    ///
    /// # Safety
    ///
    /// The slot must be occupied. Caller must ensure exclusive access.
    #[inline]
    pub unsafe fn value_ptr_mut(&self) -> *mut T {
        unsafe { (*self.value.get()).as_mut_ptr() }
    }

    /// Returns a reference to the value.
    ///
    /// # Safety
    ///
    /// The slot must be occupied. Caller must ensure no mutable references exist.
    #[inline]
    pub unsafe fn get_value(&self) -> &T {
        unsafe { (*self.value.get()).assume_init_ref() }
    }

    /// Returns a mutable reference to the value.
    ///
    /// # Safety
    ///
    /// The slot must be occupied. Caller must ensure exclusive access.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_value_mut(&self) -> &mut T {
        unsafe { (*self.value.get()).assume_init_mut() }
    }

    /// Returns the next_free pointer (for freelist traversal).
    ///
    /// Only valid when slot is vacant.
    #[doc(hidden)]
    #[inline]
    pub fn next_free(&self) -> *mut SlotCell<T> {
        debug_assert!(self.is_vacant(), "next_free on non-vacant slot");
        self.next_free.get()
    }

    /// Returns the key stored in the stamp.
    /// Valid regardless of vacant/occupied state (key is set when slot is claimed).
    #[inline]
    pub fn key_from_stamp(&self) -> u32 {
        self.stamp.get() & KEY_MASK
    }

    /// Sets key and marks slot as occupied.
    /// Also sets next_free to NULL to indicate occupied state.
    #[inline]
    pub fn set_key_occupied(&self, key: u32) {
        debug_assert!(key <= KEY_MASK, "key exceeds maximum");
        self.stamp.set(key); // No VACANT_BIT = occupied
        self.next_free.set(std::ptr::null_mut());
    }

    /// Marks slot as vacant with given next_free pointer.
    #[doc(hidden)]
    #[inline]
    pub fn set_vacant(&self, next_free: *mut SlotCell<T>) {
        self.stamp.set(VACANT_BIT | (self.stamp.get() & KEY_MASK));
        self.next_free.set(next_free);
    }
}
