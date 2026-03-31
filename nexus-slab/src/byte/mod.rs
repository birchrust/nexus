//! Type-erased byte slab allocation.
//!
//! Store heterogeneous types in a single slab. Each slot is `N` bytes
//! with pointer alignment. Any `T` that fits (`size_of::<T>() <= N`,
//! `align_of::<T>() <= 8`) can be stored.
//!
//! Two variants mirror the typed slab API:
//! - [`bounded::Slab`] — fixed capacity, returns `Err(Full)` when full
//! - [`unbounded::Slab`] — grows via chunks, never fails
//!
//! # Example
//!
//! ```
//! use nexus_slab::byte::bounded::Slab;
//!
//! // SAFETY: caller guarantees slab contract (see struct docs)
//! let slab: Slab<128> = unsafe { Slab::with_capacity(64) };
//!
//! let ptr = slab.alloc(42u64);
//! assert_eq!(*ptr, 42);
//! slab.free(ptr);
//!
//! // Different type, same slab
//! let ptr = slab.alloc([1.0f64; 8]);
//! assert_eq!(ptr[0], 1.0);
//! slab.free(ptr);
//! ```

use core::marker::PhantomData;

// =============================================================================
// AlignedBytes
// =============================================================================

/// Fixed-size byte storage with pointer alignment.
///
/// Used as the backing type for byte slabs. The 8-byte alignment
/// matches the `next_free` pointer in [`SlotCell`](crate::SlotCell)
/// and covers all common types (up to `u64`, pointers, most structs).
///
/// Types requiring greater than 8-byte alignment (e.g., SIMD vectors)
/// cannot be stored in a byte slab.
///
/// `Copy` ensures `drop_in_place` is a compile-time no-op.
#[derive(Clone, Copy)]
#[repr(C, align(8))]
pub struct AlignedBytes<const N: usize> {
    bytes: [u8; N],
}

// =============================================================================
// Slot
// =============================================================================

/// Typed handle to a value stored in a byte slab.
///
/// The slab stores raw bytes, but this handle remembers the original
/// type `T`. Provides safe `Deref`/`DerefMut` access. Move-only —
/// cannot be copied or cloned.
///
/// # Debug Leak Detection
///
/// Same as [`Slot`](crate::Slot) — panics on drop in debug
/// builds if not freed.
pub struct Slot<T> {
    ptr: *mut u8,
    _marker: PhantomData<T>,
}

impl<T> Slot<T> {
    /// Creates a duplicate pointer to the same slot.
    ///
    /// # Safety
    ///
    /// Caller must ensure the slot is not freed while any clone exists.
    #[inline]
    pub unsafe fn clone_ptr(&self) -> Self {
        Slot {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }

    /// Returns a raw pointer to the underlying byte storage.
    #[inline]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// Consumes the handle, returning the raw pointer without running Drop.
    ///
    /// Reconstruct via [`from_raw()`](Self::from_raw). Disarms the debug
    /// leak detector.
    #[inline]
    pub fn into_raw(self) -> *mut u8 {
        let ptr = self.ptr;
        core::mem::forget(self);
        ptr
    }

    /// Reconstructs a `Slot` from a raw pointer previously obtained
    /// via [`into_raw()`](Self::into_raw).
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid, initialized `T` within a byte slab,
    /// originally obtained from `into_raw()`.
    #[inline]
    pub unsafe fn from_raw(ptr: *mut u8) -> Self {
        Slot {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Returns a pinned reference to the value.
    ///
    /// Byte slab memory never moves, so `Pin` is sound without `T: Unpin`.
    #[inline]
    pub fn pin(&self) -> core::pin::Pin<&T> {
        unsafe { core::pin::Pin::new_unchecked(&**self) }
    }

    /// Returns a pinned mutable reference to the value.
    #[inline]
    pub fn pin_mut(&mut self) -> core::pin::Pin<&mut T> {
        unsafe { core::pin::Pin::new_unchecked(&mut **self) }
    }
}

impl<T> core::ops::Deref for Slot<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: ptr points to a valid, initialized T within the slab.
        unsafe { &*self.ptr.cast::<T>() }
    }
}

impl<T> core::ops::DerefMut for Slot<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: We have &mut self, guaranteeing exclusive access.
        unsafe { &mut *self.ptr.cast::<T>() }
    }
}

impl<T> core::convert::AsRef<T> for Slot<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T> core::convert::AsMut<T> for Slot<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

impl<T> core::borrow::Borrow<T> for Slot<T> {
    #[inline]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T> core::borrow::BorrowMut<T> for Slot<T> {
    #[inline]
    fn borrow_mut(&mut self) -> &mut T {
        self
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for Slot<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("byte::Slot")
            .field("value", &**self)
            .finish()
    }
}

#[cfg(debug_assertions)]
impl<T> Drop for Slot<T> {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        if std::thread::panicking() {
            return;
        }
        panic!(
            "byte::Slot<{}> dropped without being freed — call slab.free(ptr) or slab.take(ptr)",
            core::any::type_name::<T>()
        );
    }
}

/// Validates that `T` fits in `N` bytes with appropriate alignment.
#[inline]
fn validate_type<T, const N: usize>() {
    assert!(
        core::mem::size_of::<T>() <= N,
        "type {} ({} bytes) exceeds byte slab slot size ({N} bytes)",
        core::any::type_name::<T>(),
        core::mem::size_of::<T>(),
    );
    assert!(
        core::mem::align_of::<T>() <= 8,
        "type {} (align {}) exceeds byte slab alignment (8)",
        core::any::type_name::<T>(),
        core::mem::align_of::<T>(),
    );
}

pub mod bounded;
pub mod unbounded;
