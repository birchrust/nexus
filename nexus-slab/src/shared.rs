//! Shared internals for bounded and unbounded slab implementations.

use std::cell::Cell;
use std::mem::MaybeUninit;

// =============================================================================
// Constants
// =============================================================================

/// Sentinel for Key::NONE. Maximum valid key index.
pub const SLOT_NONE: u32 = (1 << 31) - 1;

// =============================================================================
// SlotCell
// =============================================================================

/// Returns the occupied sentinel pointer.
///
/// This is `1 as *mut SlotCell<T>` — an invalid pointer (alignment of SlotCell
/// is always >= 8) used to distinguish occupied slots from vacant ones in the
/// freelist. Vacant slots have either null (end of freelist) or a valid pointer.
#[inline]
fn occupied_sentinel<T>() -> *mut SlotCell<T> {
    1usize as *mut SlotCell<T>
}

/// Internal slot storage with pointer-based freelist.
///
/// Layout (repr(C)):
/// - `next_free`: 8 bytes - pointer to next free slot (OCCUPIED_SENTINEL when occupied)
/// - `value`: T - the actual value
///
/// Total header: 8 bytes before value.
#[repr(C)]
pub struct SlotCell<T> {
    /// Pointer to next free slot. OCCUPIED_SENTINEL (0x1) when slot is occupied.
    /// NULL when slot is last in freelist. Valid pointer otherwise.
    next_free: Cell<*mut SlotCell<T>>,
    /// The value storage.
    pub value: std::cell::UnsafeCell<MaybeUninit<T>>,
}

impl<T> SlotCell<T> {
    /// Creates a new vacant slot with the given next_free pointer.
    pub(crate) fn new_vacant(next_free: *mut SlotCell<T>) -> Self {
        Self {
            next_free: Cell::new(next_free),
            value: std::cell::UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Returns `true` if the slot is vacant.
    #[inline]
    pub fn is_vacant(&self) -> bool {
        self.next_free.get() != occupied_sentinel::<T>()
    }

    /// Returns `true` if the slot is occupied.
    #[inline]
    pub fn is_occupied(&self) -> bool {
        self.next_free.get() == occupied_sentinel::<T>()
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
        debug_assert!(self.is_vacant(), "next_free on occupied slot");
        self.next_free.get()
    }

    /// Marks slot as occupied.
    ///
    /// Sets next_free to OCCUPIED_SENTINEL.
    #[doc(hidden)]
    #[inline]
    pub fn set_occupied(&self) {
        self.next_free.set(occupied_sentinel::<T>());
    }

    /// Marks slot as vacant with given next_free pointer.
    #[doc(hidden)]
    #[inline]
    pub fn set_vacant(&self, next_free: *mut SlotCell<T>) {
        self.next_free.set(next_free);
    }
}
