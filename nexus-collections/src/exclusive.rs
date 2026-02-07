//! Single-borrow cell for exclusive access enforcement.
//!
//! [`ExclusiveCell`] permits at most one outstanding borrow at a time,
//! whether shared or mutable. This is simpler than `RefCell` (which
//! tracks a shared borrow count) — a single `Cell<bool>` flag is all
//! that's needed.
//!
//! Designed for use with `RcSlot` in data structures where multiple
//! handles may reference the same value but only one should access it
//! at a time.
//!
//! # Example
//!
//! ```ignore
//! use nexus_collections::ExclusiveCell;
//!
//! let cell = ExclusiveCell::new(42);
//!
//! {
//!     let r = cell.borrow();
//!     assert_eq!(*r, 42);
//!     // cell.borrow(); // would panic — already borrowed
//! } // r drops, borrow released
//!
//! {
//!     let mut w = cell.borrow_mut();
//!     *w = 99;
//! }
//!
//! assert_eq!(*cell.borrow(), 99);
//! ```

use std::cell::{Cell, UnsafeCell};
use std::fmt;
use std::ops::{Deref, DerefMut};

/// A cell that permits at most one outstanding borrow at a time.
///
/// Unlike `RefCell`, which allows multiple shared borrows OR one mutable
/// borrow (tracked via a counter), `ExclusiveCell` uses a single boolean
/// flag. Any borrow — shared or mutable — is exclusive.
///
/// This trades `RefCell`'s shared-borrow flexibility for simplicity:
/// one `Cell<bool>` instead of a `Cell<isize>`, and identical checking
/// logic for both borrow types.
pub struct ExclusiveCell<T: ?Sized> {
    borrowed: Cell<bool>,
    value: UnsafeCell<T>,
}

// SAFETY: ExclusiveCell is !Sync via UnsafeCell. It is Send if T: Send
// (same semantics as RefCell).
unsafe impl<T: ?Sized + Send> Send for ExclusiveCell<T> {}

impl<T> ExclusiveCell<T> {
    /// Creates a new `ExclusiveCell` containing the given value.
    #[inline]
    pub const fn new(value: T) -> Self {
        ExclusiveCell {
            borrowed: Cell::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Consumes the cell, returning the wrapped value.
    #[inline]
    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }

    /// Replaces the wrapped value, returning the old value.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently borrowed.
    #[inline]
    pub fn replace(&self, value: T) -> T {
        assert!(
            !self.borrowed.get(),
            "ExclusiveCell: cannot replace while borrowed"
        );
        // SAFETY: No borrows are active (just checked)
        unsafe { self.value.get().replace(value) }
    }
}

impl<T: ?Sized> ExclusiveCell<T> {
    /// Borrows the value, returning an exclusive shared reference guard.
    ///
    /// # Panics
    ///
    /// Panics if the value is already borrowed.
    #[inline]
    pub fn borrow(&self) -> ExRef<'_, T> {
        assert!(!self.borrowed.get(), "ExclusiveCell: already borrowed");
        self.borrowed.set(true);
        ExRef { cell: self }
    }

    /// Borrows the value mutably, returning an exclusive mutable reference guard.
    ///
    /// # Panics
    ///
    /// Panics if the value is already borrowed.
    #[inline]
    pub fn borrow_mut(&self) -> ExMut<'_, T> {
        assert!(!self.borrowed.get(), "ExclusiveCell: already borrowed");
        self.borrowed.set(true);
        ExMut { cell: self }
    }

    /// Tries to borrow the value. Returns `None` if already borrowed.
    #[inline]
    pub fn try_borrow(&self) -> Option<ExRef<'_, T>> {
        if self.borrowed.get() {
            return None;
        }
        self.borrowed.set(true);
        Some(ExRef { cell: self })
    }

    /// Tries to borrow the value mutably. Returns `None` if already borrowed.
    #[inline]
    pub fn try_borrow_mut(&self) -> Option<ExMut<'_, T>> {
        if self.borrowed.get() {
            return None;
        }
        self.borrowed.set(true);
        Some(ExMut { cell: self })
    }

    /// Returns `true` if the value is currently borrowed.
    #[inline]
    pub fn is_borrowed(&self) -> bool {
        self.borrowed.get()
    }

    /// Returns a raw pointer to the underlying data.
    ///
    /// No borrow tracking is performed. The caller must ensure
    /// no `ExRef`/`ExMut` guards are active when dereferencing.
    #[inline]
    pub fn as_ptr(&self) -> *mut T {
        self.value.get()
    }

    /// Returns a mutable reference to the underlying data.
    ///
    /// Since this takes `&mut self`, no borrows can be active.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }
}

impl<T: Default> Default for ExclusiveCell<T> {
    #[inline]
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for ExclusiveCell<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_borrow() {
            Some(guard) => f
                .debug_struct("ExclusiveCell")
                .field("value", &&*guard)
                .finish(),
            None => f
                .debug_struct("ExclusiveCell")
                .field("value", &"<borrowed>")
                .finish(),
        }
    }
}

// =============================================================================
// ExRef — exclusive shared reference guard
// =============================================================================

/// Exclusive shared reference to the value in an [`ExclusiveCell`].
///
/// Provides `&T` access. The borrow flag is cleared when this guard drops.
pub struct ExRef<'a, T: ?Sized> {
    cell: &'a ExclusiveCell<T>,
}

impl<T: ?Sized> Deref for ExRef<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: We hold the exclusive borrow flag, no other refs exist
        unsafe { &*self.cell.value.get() }
    }
}

impl<T: ?Sized> Drop for ExRef<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.cell.borrowed.set(false);
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for ExRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: fmt::Display + ?Sized> fmt::Display for ExRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

// =============================================================================
// ExMut — exclusive mutable reference guard
// =============================================================================

/// Exclusive mutable reference to the value in an [`ExclusiveCell`].
///
/// Provides `&mut T` access. The borrow flag is cleared when this guard drops.
pub struct ExMut<'a, T: ?Sized> {
    cell: &'a ExclusiveCell<T>,
}

impl<T: ?Sized> Deref for ExMut<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: We hold the exclusive borrow flag, no other refs exist
        unsafe { &*self.cell.value.get() }
    }
}

impl<T: ?Sized> DerefMut for ExMut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: We hold the exclusive borrow flag, no other refs exist
        unsafe { &mut *self.cell.value.get() }
    }
}

impl<T: ?Sized> Drop for ExMut<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.cell.borrowed.set(false);
    }
}

impl<T: fmt::Debug + ?Sized> fmt::Debug for ExMut<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: fmt::Display + ?Sized> fmt::Display for ExMut<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrow_and_read() {
        let cell = ExclusiveCell::new(42);
        let r = cell.borrow();
        assert_eq!(*r, 42);
    }

    #[test]
    fn borrow_mut_and_write() {
        let cell = ExclusiveCell::new(0);
        {
            let mut w = cell.borrow_mut();
            *w = 99;
        }
        assert_eq!(*cell.borrow(), 99);
    }

    #[test]
    fn borrow_released_on_drop() {
        let cell = ExclusiveCell::new(1);
        {
            let _r = cell.borrow();
        }
        // Should not panic — previous borrow was dropped
        let _r2 = cell.borrow();
    }

    #[test]
    fn borrow_mut_released_on_drop() {
        let cell = ExclusiveCell::new(1);
        {
            let _w = cell.borrow_mut();
        }
        let _r = cell.borrow();
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn double_borrow_panics() {
        let cell = ExclusiveCell::new(1);
        let _r1 = cell.borrow();
        let _r2 = cell.borrow(); // panic
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn borrow_then_borrow_mut_panics() {
        let cell = ExclusiveCell::new(1);
        let _r = cell.borrow();
        let _w = cell.borrow_mut(); // panic
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn borrow_mut_then_borrow_panics() {
        let cell = ExclusiveCell::new(1);
        let _w = cell.borrow_mut();
        let _r = cell.borrow(); // panic
    }

    #[test]
    fn try_borrow_returns_none_when_borrowed() {
        let cell = ExclusiveCell::new(1);
        let _r = cell.borrow();
        assert!(cell.try_borrow().is_none());
        assert!(cell.try_borrow_mut().is_none());
    }

    #[test]
    fn try_borrow_mut_returns_none_when_borrowed() {
        let cell = ExclusiveCell::new(1);
        let _w = cell.borrow_mut();
        assert!(cell.try_borrow().is_none());
        assert!(cell.try_borrow_mut().is_none());
    }

    #[test]
    fn try_borrow_succeeds_when_free() {
        let cell = ExclusiveCell::new(42);
        let r = cell.try_borrow();
        assert!(r.is_some());
        assert_eq!(*r.unwrap(), 42);
    }

    #[test]
    fn try_borrow_mut_succeeds_when_free() {
        let cell = ExclusiveCell::new(0);
        let w = cell.try_borrow_mut();
        assert!(w.is_some());
    }

    #[test]
    fn is_borrowed_reflects_state() {
        let cell = ExclusiveCell::new(1);
        assert!(!cell.is_borrowed());
        {
            let _r = cell.borrow();
            assert!(cell.is_borrowed());
        }
        assert!(!cell.is_borrowed());
    }

    #[test]
    fn get_mut_with_exclusive_access() {
        let mut cell = ExclusiveCell::new(0);
        *cell.get_mut() = 42;
        assert_eq!(*cell.borrow(), 42);
    }

    #[test]
    fn into_inner_extracts_value() {
        let cell = ExclusiveCell::new(String::from("hello"));
        let s = cell.into_inner();
        assert_eq!(s, "hello");
    }

    #[test]
    fn replace_swaps_value() {
        let cell = ExclusiveCell::new(1);
        let old = cell.replace(2);
        assert_eq!(old, 1);
        assert_eq!(*cell.borrow(), 2);
    }

    #[test]
    #[should_panic(expected = "cannot replace while borrowed")]
    fn replace_while_borrowed_panics() {
        let cell = ExclusiveCell::new(1);
        let _r = cell.borrow();
        cell.replace(2); // panic
    }

    #[test]
    fn debug_shows_value_when_free() {
        let cell = ExclusiveCell::new(42);
        let s = format!("{:?}", cell);
        assert!(s.contains("42"));
    }

    #[test]
    fn debug_shows_borrowed_when_held() {
        let cell = ExclusiveCell::new(42);
        let _r = cell.borrow();
        let s = format!("{:?}", cell);
        assert!(s.contains("<borrowed>"));
    }
}
