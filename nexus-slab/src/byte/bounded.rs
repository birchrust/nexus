//! Fixed-capacity type-erased byte slab.

use core::marker::PhantomData;
use core::mem;

use crate::shared::{Full, SlotCell};

use super::{AlignedBytes, SlotPtr, validate_type};

/// Fixed-capacity byte slab. Mirrors [`crate::bounded::Slab`] but stores
/// heterogeneous types in fixed-size byte slots.
pub struct Slab<const N: usize> {
    inner: crate::bounded::Slab<AlignedBytes<N>>,
}

impl<const N: usize> Slab<N> {
    /// Creates a byte slab with the given capacity.
    ///
    /// # Safety
    ///
    /// See [`crate::bounded::Slab`] safety contract.
    ///
    /// # Panics
    ///
    /// Panics if capacity is zero.
    #[inline]
    pub unsafe fn with_capacity(capacity: usize) -> Self {
        Self {
            // SAFETY: caller upholds the slab contract
            inner: unsafe { crate::bounded::Slab::with_capacity(capacity) },
        }
    }

    /// Allocates a value in the byte slab.
    ///
    /// # Panics
    ///
    /// - Panics if `size_of::<T>() > N`
    /// - Panics if `align_of::<T>() > 8`
    /// - Panics if the slab is full
    #[inline]
    pub fn alloc<T>(&self, value: T) -> SlotPtr<T> {
        self.try_alloc(value)
            .unwrap_or_else(|_| panic!("byte slab full"))
    }

    /// Tries to allocate a value. Returns `Err(Full(value))` if full.
    ///
    /// # Panics
    ///
    /// - Panics if `size_of::<T>() > N`
    /// - Panics if `align_of::<T>() > 8`
    pub fn try_alloc<T>(&self, value: T) -> Result<SlotPtr<T>, Full<T>> {
        validate_type::<T, N>();

        let Some(slot_ptr) = self.inner.claim_ptr() else {
            return Err(Full(value));
        };

        // SAFETY: slot_ptr is a valid, vacant SlotCell<AlignedBytes<N>>.
        // AlignedBytes<N> is repr(C, align(8)) so the value region is
        // suitably aligned for T (asserted above: align <= 8).
        unsafe {
            let data_ptr = slot_ptr.cast::<T>();
            core::ptr::write(data_ptr, value);
        }

        Ok(SlotPtr {
            ptr: slot_ptr.cast::<u8>(),
            _marker: PhantomData,
        })
    }

    /// Frees a value, dropping it and returning the slot to the freelist.
    ///
    /// Consumes the handle — the slot cannot be used after this call.
    #[inline]
    pub fn free<T>(&self, ptr: SlotPtr<T>) {
        let data_ptr = ptr.ptr;
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
    pub fn take<T>(&self, ptr: SlotPtr<T>) -> T {
        let data_ptr = ptr.ptr;
        mem::forget(ptr);

        unsafe {
            let value = core::ptr::read(data_ptr.cast::<T>());
            self.inner
                .free_ptr(data_ptr.cast::<SlotCell<AlignedBytes<N>>>());
            value
        }
    }

    /// Returns the capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }
}

impl<const N: usize> core::fmt::Debug for Slab<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("byte::bounded::Slab")
            .field("slot_size", &N)
            .field("capacity", &self.capacity())
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn basic_alloc_free() {
        let slab: Slab<128> = unsafe { Slab::with_capacity(10) };
        let ptr = slab.alloc(42u64);
        assert_eq!(*ptr, 42);
        slab.free(ptr);
    }

    #[test]
    fn heterogeneous_types() {
        let slab: Slab<128> = unsafe { Slab::with_capacity(10) };

        let p1 = slab.alloc(42u64);
        let p2 = slab.alloc([1.0f64; 4]);
        let p3 = slab.alloc(String::from("hello"));

        assert_eq!(*p1, 42);
        assert_eq!(p2[0], 1.0);
        assert_eq!(&*p3, "hello");

        slab.free(p3);
        slab.free(p2);
        slab.free(p1);
    }

    #[test]
    fn take_returns_value() {
        let slab: Slab<64> = unsafe { Slab::with_capacity(10) };
        let ptr = slab.alloc(String::from("owned"));
        let val = slab.take(ptr);
        assert_eq!(val, "owned");
    }

    #[test]
    fn full_returns_error() {
        let slab: Slab<64> = unsafe { Slab::with_capacity(1) };
        let p1 = slab.alloc(1u64);
        let result = slab.try_alloc(2u64);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 2);
        slab.free(p1);
    }

    #[test]
    #[should_panic(expected = "exceeds byte slab slot size")]
    fn rejects_oversized_type() {
        let slab: Slab<8> = unsafe { Slab::with_capacity(1) };
        let _p = slab.alloc([0u64; 2]);
    }

    #[test]
    fn deref_mut() {
        let slab: Slab<64> = unsafe { Slab::with_capacity(10) };
        let mut ptr = slab.alloc(String::from("hello"));
        ptr.push_str(" world");
        assert_eq!(&*ptr, "hello world");
        slab.free(ptr);
    }

    #[test]
    fn reuse_after_free() {
        let slab: Slab<64> = unsafe { Slab::with_capacity(1) };
        let ptr = slab.alloc(1u64);
        slab.free(ptr);
        let ptr = slab.alloc(2u64);
        assert_eq!(*ptr, 2);
        slab.free(ptr);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_drop_panics() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let slab: Slab<64> = unsafe { Slab::with_capacity(10) };
            let _ptr = slab.alloc(42u64);
        }));
        assert!(result.is_err());
    }
}
