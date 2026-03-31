//! Shared internals for bounded and unbounded slab implementations.

use core::borrow::{Borrow, BorrowMut};
use core::fmt;
use core::mem::{ManuallyDrop, MaybeUninit};
use core::ops::{Deref, DerefMut};

// =============================================================================
// Full<T>
// =============================================================================

/// Error returned when a bounded allocator is full.
///
/// Contains the value that could not be allocated, allowing recovery.
pub struct Full<T>(pub T);

impl<T> Full<T> {
    /// Consumes the error, returning the value that could not be allocated.
    #[inline]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Full<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Full(..)")
    }
}

impl<T> fmt::Display for Full<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("allocator full")
    }
}

#[cfg(feature = "std")]
impl<T> std::error::Error for Full<T> {}

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
    next_free: *mut SlotCell<T>,
    value: ManuallyDrop<MaybeUninit<T>>,
}

impl<T> SlotCell<T> {
    /// Creates a new vacant slot with the given next_free pointer.
    #[inline]
    pub(crate) fn vacant(next_free: *mut SlotCell<T>) -> Self {
        SlotCell { next_free }
    }

    /// Writes a value into this slot, transitioning it from vacant to occupied.
    ///
    /// # Safety
    ///
    /// The slot must be vacant (no live value present).
    #[inline]
    pub unsafe fn write_value(&mut self, value: T) {
        self.value = ManuallyDrop::new(MaybeUninit::new(value));
    }

    /// Reads the value out of this slot without dropping it.
    ///
    /// # Safety
    ///
    /// The slot must be occupied with a valid `T`.
    /// After this call, the caller owns the value and the slot must not be
    /// read again without a subsequent write.
    #[inline]
    pub unsafe fn read_value(&self) -> T {
        // SAFETY: Caller guarantees the slot is occupied.
        unsafe { core::ptr::read(self.value.as_ptr()) }
    }

    /// Drops the value in place without returning it.
    ///
    /// # Safety
    ///
    /// The slot must be occupied with a valid `T`.
    #[inline]
    pub unsafe fn drop_value_in_place(&mut self) {
        // SAFETY: Caller guarantees the slot is occupied.
        unsafe {
            core::ptr::drop_in_place((*self.value).as_mut_ptr());
        }
    }

    /// Returns a reference to the occupied value.
    ///
    /// # Safety
    ///
    /// The slot must be occupied with a valid `T`.
    #[inline]
    pub unsafe fn value_ref(&self) -> &T {
        // SAFETY: Caller guarantees the slot is occupied.
        unsafe { self.value.assume_init_ref() }
    }

    /// Returns a mutable reference to the occupied value.
    ///
    /// # Safety
    ///
    /// The slot must be occupied with a valid `T`.
    /// Caller must have exclusive access.
    #[inline]
    pub unsafe fn value_mut(&mut self) -> &mut T {
        // SAFETY: Caller guarantees the slot is occupied.
        unsafe { (*self.value).assume_init_mut() }
    }

    /// Returns a raw const pointer to the value storage.
    ///
    /// # Safety
    ///
    /// The slot must be occupied.
    #[inline]
    pub unsafe fn value_ptr(&self) -> *const T {
        // SAFETY: Caller guarantees the slot is occupied.
        unsafe { self.value.as_ptr() }
    }

    /// Returns the next_free pointer.
    ///
    /// # Safety
    ///
    /// The slot must be vacant.
    #[inline]
    pub(crate) unsafe fn get_next_free(&self) -> *mut SlotCell<T> {
        // SAFETY: Caller guarantees the slot is vacant.
        unsafe { self.next_free }
    }

    /// Sets the next_free pointer.
    ///
    /// # Safety
    ///
    /// Caller must be transitioning this slot to vacant.
    #[inline]
    pub(crate) unsafe fn set_next_free(&mut self, next: *mut SlotCell<T>) {
        self.next_free = next;
    }
}

// =============================================================================
// SlotPtr<T> — Raw Pointer Wrapper
// =============================================================================

/// Raw slot handle — pointer wrapper, NOT RAII.
///
/// `SlotPtr<T>` is a thin wrapper around a pointer to a [`SlotCell<T>`]. It is
/// analogous to `malloc` returning a pointer: the caller owns the memory and
/// must explicitly free it via [`Slab::free()`](crate::bounded::Slab::free).
///
/// # Size
///
/// 8 bytes (one pointer).
///
/// # Thread Safety
///
/// `SlotPtr` is `!Send` and `!Sync`. It must only be used from the thread that
/// created it.
///
/// # Debug-Mode Leak Detection
///
/// In debug builds, dropping a `SlotPtr` without calling `free()` or
/// `take()` panics. Use [`into_ptr()`](Self::into_ptr) to extract the
/// pointer and disarm the detector. In release builds there is no `Drop`
/// impl — forgetting to call `free()` silently leaks the slot.
///
/// # Borrow Traits
///
/// `SlotPtr<T>` implements `Borrow<T>` and `BorrowMut<T>`, enabling use as
/// HashMap keys that borrow `T` for lookups.
#[repr(transparent)]
pub struct SlotPtr<T>(*mut SlotCell<T>);

impl<T> SlotPtr<T> {
    /// Creates a slot from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid pointer to an occupied `SlotCell<T>` within a slab.
    #[inline]
    pub unsafe fn from_ptr(ptr: *mut SlotCell<T>) -> Self {
        SlotPtr(ptr)
    }

    /// Returns the raw pointer to the slot cell.
    #[inline]
    pub fn as_ptr(&self) -> *mut SlotCell<T> {
        self.0
    }

    /// Consumes the slot, returning the raw pointer.
    ///
    /// Unlike [`as_ptr()`](Self::as_ptr), this is a consuming operation that
    /// disarms the debug-mode leak detector. In release mode, compiles
    /// identically to `as_ptr()`.
    #[inline]
    pub fn into_ptr(self) -> *mut SlotCell<T> {
        let ptr = self.0;
        core::mem::forget(self);
        ptr
    }

    /// Creates a duplicate pointer to the same slot.
    ///
    /// # Safety
    ///
    /// Caller must ensure the slot is not freed while any clone exists.
    /// Intended for refcounting wrappers (e.g., nexus-collections' RcHandle).
    #[inline]
    pub unsafe fn clone_ptr(&self) -> Self {
        SlotPtr(self.0)
    }
}

impl<T> Deref for SlotPtr<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: SlotPtr was created from a valid, occupied SlotCell.
        unsafe { (*self.0).value_ref() }
    }
}

impl<T> DerefMut for SlotPtr<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: We have &mut self, guaranteeing exclusive access.
        unsafe { (*self.0).value_mut() }
    }
}

impl<T> AsRef<T> for SlotPtr<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T> AsMut<T> for SlotPtr<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T> Borrow<T> for SlotPtr<T> {
    #[inline]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T> BorrowMut<T> for SlotPtr<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self
    }
}

// SlotPtr is intentionally NOT Clone/Copy.
// Move-only semantics prevent double-free at compile time.

impl<T: fmt::Debug> fmt::Debug for SlotPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlotPtr").field("value", &**self).finish()
    }
}

#[cfg(debug_assertions)]
impl<T> Drop for SlotPtr<T> {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        if std::thread::panicking() {
            return; // Don't double-panic during unwind
        }
        panic!(
            "SlotPtr<{}> dropped without being freed — call slab.free()",
            core::any::type_name::<T>()
        );
    }
}
