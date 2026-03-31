//! Growable reference-counted slab.

use super::{RcCell, RcSlot};

/// Growable slab with reference-counted handles.
///
/// Wraps [`crate::unbounded::Slab`] with `RcCell<T>` storage.
/// Never fails — grows via chunks when full.
pub struct Slab<T> {
    inner: crate::unbounded::Slab<RcCell<T>>,
}

impl<T> Slab<T> {
    /// Creates a new Rc slab with the given chunk capacity.
    ///
    /// # Safety
    ///
    /// See [`crate::bounded::Slab`] safety contract.
    #[inline]
    pub unsafe fn with_chunk_capacity(chunk_capacity: usize) -> Self {
        Self {
            inner: unsafe { crate::unbounded::Slab::with_chunk_capacity(chunk_capacity) },
        }
    }

    /// Allocates a value. Never fails — grows if needed.
    #[inline]
    pub fn alloc(&self, value: T) -> RcSlot<T> {
        let slot = self.inner.alloc(RcCell::new(value));
        unsafe { RcSlot::from_ptr(slot.into_raw().cast()) }
    }

    /// Frees a handle. Decrements refcount; deallocates on last free.
    #[inline]
    #[allow(clippy::needless_pass_by_value)]
    pub fn free(&self, handle: RcSlot<T>) {
        let count = handle.dec_ref();
        if count == 0 {
            unsafe { handle.drop_value() };
            let cell_ptr = handle.slot_cell_ptr();
            core::mem::forget(handle);
            unsafe { self.inner.free_ptr(cell_ptr) };
        } else {
            core::mem::forget(handle);
        }
    }
}

impl<T> core::fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("rc::unbounded::Slab").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_borrow_free() {
        let slab = unsafe { Slab::with_chunk_capacity(4) };
        let h1 = slab.alloc(42u64);
        let h2 = h1.clone();

        assert_eq!(h1.refcount(), 2);
        { let g = h1.borrow(); assert_eq!(*g, 42); }

        slab.free(h2);
        slab.free(h1);
    }

    #[test]
    fn grows_automatically() {
        let slab = unsafe { Slab::with_chunk_capacity(2) };
        let mut handles = alloc::vec::Vec::new();
        for i in 0..100u64 {
            handles.push(slab.alloc(i));
        }
        for (i, h) in handles.iter().enumerate() {
            let g = h.borrow();
            assert_eq!(*g, i as u64);
        }
        for h in handles {
            slab.free(h);
        }
    }
}
