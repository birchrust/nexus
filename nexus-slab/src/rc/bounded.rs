//! Fixed-capacity reference-counted slab.

use crate::shared::Full;
use super::{RcCell, RcSlot};

/// Fixed-capacity slab with reference-counted handles.
///
/// Wraps [`crate::bounded::Slab`] with `RcCell<T>` storage. The user
/// works with `RcSlot<T>` — the refcount header is invisible.
///
/// # Contract
///
/// Same as [`crate::bounded::Slab`]: construction is `unsafe`, the
/// caller accepts manual memory management. Every `RcSlot` must be
/// freed via [`free()`](Self::free). The slot is deallocated when the
/// last handle is freed.
pub struct Slab<T> {
    inner: crate::bounded::Slab<RcCell<T>>,
}

impl<T> Slab<T> {
    /// Creates a new Rc slab with the given capacity.
    ///
    /// # Safety
    ///
    /// See [`crate::bounded::Slab`] safety contract.
    #[inline]
    pub unsafe fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: unsafe { crate::bounded::Slab::with_capacity(capacity) },
        }
    }

    /// Allocates a value, returning an `RcSlot` with refcount 1.
    ///
    /// # Panics
    ///
    /// Panics if the slab is full.
    #[inline]
    pub fn alloc(&self, value: T) -> RcSlot<T> {
        let slot = self.inner.alloc(RcCell::new(value));
        unsafe { RcSlot::from_ptr(slot.into_raw().cast()) }
    }

    /// Tries to allocate. Returns `Err(Full(value))` if full.
    #[inline]
    pub fn try_alloc(&self, value: T) -> Result<RcSlot<T>, Full<T>> {
        match self.inner.try_alloc(RcCell::new(value)) {
            Ok(slot) => Ok(unsafe { RcSlot::from_ptr(slot.into_raw().cast()) }),
            Err(Full(rc_cell)) => Err(Full(rc_cell.into_inner())),
        }
    }

    /// Frees a handle. Decrements refcount; deallocates on last free.
    ///
    /// Consumes the handle — cannot be used after.
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

    /// Returns the capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }
}

impl<T> core::fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("rc::bounded::Slab")
            .field("capacity", &self.capacity())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_borrow_free() {
        let slab = unsafe { Slab::with_capacity(10) };
        let handle = slab.alloc(42u64);

        assert_eq!(handle.refcount(), 1);
        {
            let guard = handle.borrow();
            assert_eq!(*guard, 42);
        }
        slab.free(handle);
    }

    #[test]
    fn clone_and_free_both() {
        let slab = unsafe { Slab::with_capacity(10) };
        let h1 = slab.alloc(42u64);
        let h2 = h1.clone();

        assert_eq!(h1.refcount(), 2);
        slab.free(h2);
        assert_eq!(h1.refcount(), 1);
        slab.free(h1);
    }

    #[test]
    fn borrow_and_borrow_mut() {
        let slab = unsafe { Slab::with_capacity(10) };
        let handle = slab.alloc(String::from("hello"));

        {
            let guard = handle.borrow();
            assert_eq!(&*guard, "hello");
        }
        {
            let mut guard = handle.borrow_mut();
            guard.push_str(" world");
        }
        {
            let guard = handle.borrow();
            assert_eq!(&*guard, "hello world");
        }
        slab.free(handle);
    }

    #[test]
    fn mutation_visible_across_clones() {
        let slab = unsafe { Slab::with_capacity(10) };
        let h1 = slab.alloc(1u64);
        let h2 = h1.clone();

        { let mut g = h1.borrow_mut(); *g = 99; }
        { let g = h2.borrow(); assert_eq!(*g, 99); }

        slab.free(h2);
        slab.free(h1);
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn double_borrow_panics() {
        let slab = unsafe { Slab::with_capacity(10) };
        let h1 = slab.alloc(42u64);
        let h2 = h1.clone();

        let _g1 = h1.borrow();
        let _g2 = h2.borrow();
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn borrow_while_mut_panics() {
        let slab = unsafe { Slab::with_capacity(10) };
        let h1 = slab.alloc(42u64);
        let h2 = h1.clone();

        let _g1 = h1.borrow_mut();
        let _g2 = h2.borrow();
    }

    #[test]
    fn try_alloc_full() {
        let slab = unsafe { Slab::with_capacity(1) };
        let h1 = slab.alloc(1u64);

        let result = slab.try_alloc(2u64);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 2);

        slab.free(h1);
    }

    #[test]
    fn drop_tracking() {
        use core::cell::Cell;

        struct DropCounter<'a> { count: &'a Cell<u32> }
        impl Drop for DropCounter<'_> {
            fn drop(&mut self) { self.count.set(self.count.get() + 1); }
        }

        let drops = Cell::new(0);
        let slab = unsafe { Slab::with_capacity(10) };

        let h1 = slab.alloc(DropCounter { count: &drops });
        let h2 = h1.clone();

        slab.free(h2);
        assert_eq!(drops.get(), 0);

        slab.free(h1);
        assert_eq!(drops.get(), 1);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_drop_panics() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let slab = unsafe { Slab::with_capacity(10) };
            let _h = slab.alloc(42u64);
        }));
        assert!(result.is_err());
    }
}
