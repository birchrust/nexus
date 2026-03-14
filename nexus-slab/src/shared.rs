//! Shared internals for bounded and unbounded slab implementations.

use std::borrow::{Borrow, BorrowMut};
use std::cell::Cell;
use std::fmt;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::{Deref, DerefMut};

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
        unsafe { std::ptr::read(self.value.as_ptr()) }
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
            std::ptr::drop_in_place((*self.value).as_mut_ptr());
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
// RcInner
// =============================================================================

/// Strong/weak refcount header + value for reference-counted slab allocation.
///
/// `ManuallyDrop<T>` ensures `RawSlot`'s `drop_in_place` on this type is a no-op
/// for `T` — `RcSlot` manages `T`'s lifetime manually (drops when strong hits 0).
///
/// # Layout
///
/// Two `u32` refcounts (8 bytes) followed by the value. The 8-byte header
/// aligns naturally with the value for types with alignment <= 8.
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
// RawSlot<T> — Raw Pointer Wrapper
// =============================================================================

/// Raw slot handle — pointer wrapper, NOT RAII.
///
/// `RawSlot<T>` is a thin wrapper around a pointer to a [`SlotCell<T>`]. It is
/// analogous to `malloc` returning a pointer: the caller owns the memory and
/// must explicitly free it via [`Slab::free()`](crate::bounded::Slab::free).
///
/// # Size
///
/// 8 bytes (one pointer).
///
/// # Thread Safety
///
/// `RawSlot` is `!Send` and `!Sync`. It must only be used from the thread that
/// created it.
///
/// # Debug-Mode Leak Detection
///
/// In debug builds, dropping a `RawSlot` without calling `free()` or
/// `take()` panics. Use [`into_ptr()`](Self::into_ptr) to extract the
/// pointer and disarm the detector. In release builds there is no `Drop`
/// impl — forgetting to call `free()` silently leaks the slot.
///
/// # Borrow Traits
///
/// `RawSlot<T>` implements `Borrow<T>` and `BorrowMut<T>`, enabling use as
/// HashMap keys that borrow `T` for lookups.
#[repr(transparent)]
pub struct RawSlot<T>(*mut SlotCell<T>);

impl<T> RawSlot<T> {
    /// Creates a slot from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid pointer to an occupied `SlotCell<T>` within a slab.
    #[inline]
    pub unsafe fn from_ptr(ptr: *mut SlotCell<T>) -> Self {
        RawSlot(ptr)
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
        std::mem::forget(self);
        ptr
    }
}

impl<T> Deref for RawSlot<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: RawSlot was created from a valid, occupied SlotCell.
        unsafe { (*self.0).value_ref() }
    }
}

impl<T> DerefMut for RawSlot<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: We have &mut self, guaranteeing exclusive access.
        unsafe { (*self.0).value_mut() }
    }
}

impl<T> AsRef<T> for RawSlot<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T> AsMut<T> for RawSlot<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T> Borrow<T> for RawSlot<T> {
    #[inline]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T> BorrowMut<T> for RawSlot<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self
    }
}

// RawSlot is intentionally NOT Clone/Copy.
// Move-only semantics prevent double-free at compile time.

impl<T: fmt::Debug> fmt::Debug for RawSlot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawSlot").field("value", &**self).finish()
    }
}

#[cfg(debug_assertions)]
impl<T> Drop for RawSlot<T> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            // During unwinding: log but don't abort. Leak is lesser evil than abort.
            eprintln!(
                "RawSlot<{}> leaked during panic unwind (was not freed)",
                std::any::type_name::<T>()
            );
        } else {
            panic!(
                "RawSlot<{}> dropped without being freed — \
                 call slab.free() or slab.take()",
                std::any::type_name::<T>()
            );
        }
    }
}
