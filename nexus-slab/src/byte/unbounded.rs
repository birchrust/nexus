//! Growable type-erased byte slab.

use core::marker::PhantomData;
use core::mem;

use crate::shared::SlotCell;

use super::{AlignedBytes, Slot, validate_type};

/// Growable byte slab. Mirrors [`crate::unbounded::Slab`] but stores
/// heterogeneous types in fixed-size byte slots.
///
/// Grows via independent chunks — no copying, no reallocation of
/// existing slots. Pointers remain valid.
pub struct Slab<const N: usize> {
    inner: crate::unbounded::Slab<AlignedBytes<N>>,
}

impl<const N: usize> Slab<N> {
    /// Creates a new unbounded byte slab with default chunk size (256 slots).
    ///
    /// # Safety
    ///
    /// See [`crate::bounded::Slab`] safety contract.
    #[inline]
    pub unsafe fn new() -> Self {
        // SAFETY: caller upholds the slab contract
        unsafe { Self::with_chunk_capacity(256) }
    }

    /// Creates a new unbounded byte slab with the given initial chunk capacity.
    ///
    /// # Safety
    ///
    /// See [`crate::bounded::Slab`] safety contract.
    #[inline]
    pub unsafe fn with_chunk_capacity(chunk_capacity: usize) -> Self {
        Self {
            // SAFETY: caller upholds the slab contract
            inner: unsafe { crate::unbounded::Slab::with_chunk_capacity(chunk_capacity) },
        }
    }

    /// Allocates a value. Never fails — grows if needed.
    ///
    /// # Panics
    ///
    /// - Panics if `size_of::<T>() > N`
    /// - Panics if `align_of::<T>() > 8`
    #[inline]
    pub fn alloc<T>(&self, value: T) -> Slot<T> {
        validate_type::<T, N>();

        let (slot_ptr, _chunk_idx) = self.inner.claim_ptr();

        // SAFETY: slot_ptr is valid and vacant. AlignedBytes<N> is
        // repr(C, align(8)), suitable for T (asserted above).
        unsafe {
            let data_ptr = slot_ptr.cast::<T>();
            core::ptr::write(data_ptr, value);
        }

        Slot {
            ptr: slot_ptr.cast::<u8>(),
            _marker: PhantomData,
        }
    }

    /// Claim a slot and copy raw bytes into it. Returns a raw pointer.
    ///
    /// # Safety
    ///
    /// - `src` must point to `size` valid bytes.
    /// - `size` must be <= `N`.
    ///
    /// # Panics
    ///
    /// - Panics if `size > N`.
    #[inline]
    pub unsafe fn alloc_raw(&self, src: *const u8, size: usize) -> *mut u8 {
        assert!(size <= N, "raw alloc size {size} exceeds slot size {N}");
        let (slot_ptr, _chunk_idx) = self.inner.claim_ptr();
        let dst = slot_ptr.cast::<u8>();
        unsafe { core::ptr::copy_nonoverlapping(src, dst, size) };
        dst
    }

    /// Frees a value, dropping it and returning the slot to the freelist.
    ///
    /// Consumes the handle — the slot cannot be used after this call.
    #[inline]
    pub fn free<T>(&self, ptr: Slot<T>) {
        let data_ptr = ptr.ptr;
        debug_assert!(
            self.inner.contains_ptr(data_ptr as *const ()),
            "slot was not allocated from this slab"
        );
        mem::forget(ptr);

        unsafe {
            core::ptr::drop_in_place(data_ptr.cast::<T>());
            self.inner
                .free_ptr(data_ptr.cast::<SlotCell<AlignedBytes<N>>>());
        }
    }

    /// Takes the value out without dropping it, freeing the slot.
    ///
    /// Consumes the handle — the slot cannot be used after this call.
    #[inline]
    pub fn take<T>(&self, ptr: Slot<T>) -> T {
        let data_ptr = ptr.ptr;
        debug_assert!(
            self.inner.contains_ptr(data_ptr as *const ()),
            "slot was not allocated from this slab"
        );
        mem::forget(ptr);

        unsafe {
            let value = core::ptr::read(data_ptr.cast::<T>());
            self.inner
                .free_ptr(data_ptr.cast::<SlotCell<AlignedBytes<N>>>());
            value
        }
    }
}

impl<const N: usize> core::fmt::Debug for Slab<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("byte::unbounded::Slab")
            .field("slot_size", &N)
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn basic_alloc_free() {
        let slab: Slab<64> = unsafe { Slab::new() };
        let ptr = slab.alloc(42u64);
        assert_eq!(*ptr, 42);
        slab.free(ptr);
    }

    #[test]
    fn heterogeneous_types() {
        let slab: Slab<128> = unsafe { Slab::new() };

        let p1 = slab.alloc(42u64);
        let p2 = slab.alloc(String::from("hello"));
        let p3 = slab.alloc([1.0f64; 8]);

        assert_eq!(*p1, 42);
        assert_eq!(&*p2, "hello");
        assert_eq!(p3[0], 1.0);

        slab.free(p3);
        slab.free(p2);
        slab.free(p1);
    }

    #[test]
    fn grows_automatically() {
        let slab: Slab<16> = unsafe { Slab::with_chunk_capacity(2) };
        let mut ptrs = alloc::vec::Vec::new();
        for i in 0..100u64 {
            ptrs.push(slab.alloc(i));
        }
        for (i, ptr) in ptrs.iter().enumerate() {
            assert_eq!(**ptr, i as u64);
        }
        for ptr in ptrs {
            slab.free(ptr);
        }
    }

    #[test]
    fn take_returns_value() {
        let slab: Slab<64> = unsafe { Slab::new() };
        let ptr = slab.alloc(String::from("taken"));
        let val = slab.take(ptr);
        assert_eq!(val, "taken");
    }
}
