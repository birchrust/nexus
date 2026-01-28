//! Internal trait for abstracting over BoundedSlab and Slab.
//!
//! This module provides a sealed trait that enables code reuse between
//! bounded and unbounded storage variants without exposing implementation
//! details to users.

use nexus_slab::{BoundedSlab, Key, Slab};

/// Operations common to BoundedSlab and Slab.
///
/// This is a sealed trait - only implemented for `nexus_slab` types.
/// It enables the specialized storage types to share implementation
/// between bounded and unbounded variants.
pub(crate) trait SlabOps<T>: sealed::Sealed {
    /// Returns the number of occupied slots.
    fn slab_len(&self) -> usize;

    /// Returns `true` if no slots are occupied.
    fn slab_is_empty(&self) -> bool;

    /// Returns `true` if the key points to a valid, occupied slot.
    fn slab_contains(&self, key: Key) -> bool;

    /// Returns an untracked reference to the value at `key`.
    ///
    /// # Safety
    ///
    /// Caller must ensure no mutable references to this slot exist,
    /// and no Entry handles for this slot are active.
    unsafe fn slab_get_untracked(&self, key: Key) -> Option<&T>;

    /// Returns an untracked mutable reference to the value at `key`.
    ///
    /// # Safety
    ///
    /// Caller must ensure no other references to this slot exist,
    /// and no Entry handles for this slot are active.
    #[allow(clippy::mut_from_ref)] // nexus-slab uses interior mutability
    unsafe fn slab_get_untracked_mut(&self, key: Key) -> Option<&mut T>;

    /// Returns an untracked reference without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied. No other mutable references
    /// or Entry handles may exist for this slot.
    unsafe fn slab_get_unchecked(&self, key: Key) -> &T;

    /// Returns an untracked mutable reference without bounds checking.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied. No other references or Entry
    /// handles may exist for this slot.
    #[allow(clippy::mut_from_ref)] // nexus-slab uses interior mutability
    unsafe fn slab_get_unchecked_mut(&self, key: Key) -> &mut T;

    /// Attempts to remove the value at `key`, returning it if present.
    fn slab_try_remove(&self, key: Key) -> Option<T>;

    /// Removes the value at `key` without checking if it exists.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn slab_remove_unchecked(&self, key: Key) -> T;
}

impl<T> SlabOps<T> for BoundedSlab<T> {
    #[inline]
    fn slab_len(&self) -> usize {
        BoundedSlab::len(self)
    }

    #[inline]
    fn slab_is_empty(&self) -> bool {
        BoundedSlab::is_empty(self)
    }

    #[inline]
    fn slab_contains(&self, key: Key) -> bool {
        BoundedSlab::contains(self, key)
    }

    #[inline]
    unsafe fn slab_get_untracked(&self, key: Key) -> Option<&T> {
        unsafe { BoundedSlab::get_untracked(self, key) }
    }

    #[inline]
    unsafe fn slab_get_untracked_mut(&self, key: Key) -> Option<&mut T> {
        unsafe { BoundedSlab::get_untracked_mut(self, key) }
    }

    #[inline]
    unsafe fn slab_get_unchecked(&self, key: Key) -> &T {
        unsafe { BoundedSlab::get_unchecked(self, key) }
    }

    #[inline]
    unsafe fn slab_get_unchecked_mut(&self, key: Key) -> &mut T {
        unsafe { BoundedSlab::get_unchecked_mut(self, key) }
    }

    #[inline]
    fn slab_try_remove(&self, key: Key) -> Option<T> {
        BoundedSlab::try_remove_by_key(self, key)
    }

    #[inline]
    unsafe fn slab_remove_unchecked(&self, key: Key) -> T {
        unsafe { BoundedSlab::remove_unchecked_by_key(self, key) }
    }
}

impl<T> SlabOps<T> for Slab<T> {
    #[inline]
    fn slab_len(&self) -> usize {
        Slab::len(self)
    }

    #[inline]
    fn slab_is_empty(&self) -> bool {
        Slab::is_empty(self)
    }

    #[inline]
    fn slab_contains(&self, key: Key) -> bool {
        Slab::contains(self, key)
    }

    #[inline]
    unsafe fn slab_get_untracked(&self, key: Key) -> Option<&T> {
        unsafe { Slab::get_untracked(self, key) }
    }

    #[inline]
    unsafe fn slab_get_untracked_mut(&self, key: Key) -> Option<&mut T> {
        unsafe { Slab::get_untracked_mut(self, key) }
    }

    #[inline]
    unsafe fn slab_get_unchecked(&self, key: Key) -> &T {
        unsafe { Slab::get_unchecked(self, key) }
    }

    #[inline]
    unsafe fn slab_get_unchecked_mut(&self, key: Key) -> &mut T {
        unsafe { Slab::get_unchecked_mut(self, key) }
    }

    #[inline]
    fn slab_try_remove(&self, key: Key) -> Option<T> {
        Slab::try_remove_by_key(self, key)
    }

    #[inline]
    unsafe fn slab_remove_unchecked(&self, key: Key) -> T {
        unsafe { Slab::remove_unchecked_by_key(self, key) }
    }
}

mod sealed {
    use nexus_slab::{BoundedSlab, Slab};

    pub trait Sealed {}

    impl<T> Sealed for BoundedSlab<T> {}
    impl<T> Sealed for Slab<T> {}
}
