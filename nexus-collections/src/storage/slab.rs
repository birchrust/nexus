//! Storage trait implementations for nexus-slab types.
//!
//! This module provides the legacy [`Storage`] trait implementations for
//! [`BoundedSlab`] and [`Slab`]. These are used by the generic storage API
//! which is deprecated.
//!
//! The specialized storage types ([`ListStorage`](super::ListStorage), etc.)
//! use [`SlabOps`](crate::internal::SlabOps) directly instead.

use super::{BoundedStorage, Full, Storage, UnboundedStorage};
use nexus_slab::{BoundedSlab, Key as NexusKey, Slab};

// BoundedSlab: Storage + BoundedStorage
impl<T> Storage<T> for BoundedSlab<T> {
    type Key = NexusKey;

    #[inline]
    fn remove(&mut self, key: Self::Key) -> Option<T> {
        BoundedSlab::try_remove_by_key(self, key)
    }

    #[inline]
    fn get(&self, key: Self::Key) -> Option<&T> {
        // SAFETY: We have &self, so no mutable references can exist.
        // get_untracked returns direct reference without entry tracking.
        unsafe { BoundedSlab::get_untracked(self, key) }
    }

    #[inline]
    fn get_mut(&mut self, key: Self::Key) -> Option<&mut T> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { BoundedSlab::get_untracked_mut(self, key) }
    }

    #[inline]
    fn len(&self) -> usize {
        BoundedSlab::len(self)
    }

    #[inline]
    unsafe fn get_unchecked(&self, key: Self::Key) -> &T {
        unsafe { BoundedSlab::get_unchecked(self, key) }
    }

    #[inline]
    unsafe fn get_unchecked_mut(&mut self, key: Self::Key) -> &mut T {
        unsafe { BoundedSlab::get_unchecked_mut(self, key) }
    }

    #[inline]
    unsafe fn remove_unchecked(&mut self, key: Self::Key) -> T {
        unsafe { BoundedSlab::remove_unchecked_by_key(self, key) }
    }
}

impl<T> BoundedStorage<T> for BoundedSlab<T> {
    #[inline]
    fn try_insert(&mut self, value: T) -> Result<Self::Key, Full<T>> {
        BoundedSlab::insert(self, value)
            .map(|entry| entry.key())
            .map_err(|e| Full(e.0))
    }

    #[inline]
    fn capacity(&self) -> usize {
        BoundedSlab::capacity(self)
    }
}

// Slab: Storage + UnboundedStorage
impl<T> Storage<T> for Slab<T> {
    type Key = NexusKey;

    #[inline]
    fn remove(&mut self, key: Self::Key) -> Option<T> {
        Slab::try_remove_by_key(self, key)
    }

    #[inline]
    fn get(&self, key: Self::Key) -> Option<&T> {
        // SAFETY: We have &self, so no mutable references can exist.
        unsafe { Slab::get_untracked(self, key) }
    }

    #[inline]
    fn get_mut(&mut self, key: Self::Key) -> Option<&mut T> {
        // SAFETY: We have &mut self, so no other references can exist.
        unsafe { Slab::get_untracked_mut(self, key) }
    }

    #[inline]
    fn len(&self) -> usize {
        Slab::len(self)
    }

    #[inline]
    unsafe fn get_unchecked(&self, key: Self::Key) -> &T {
        unsafe { Slab::get_unchecked(self, key) }
    }

    #[inline]
    unsafe fn get_unchecked_mut(&mut self, key: Self::Key) -> &mut T {
        unsafe { Slab::get_unchecked_mut(self, key) }
    }

    #[inline]
    unsafe fn remove_unchecked(&mut self, key: Self::Key) -> T {
        unsafe { Slab::remove_unchecked_by_key(self, key) }
    }
}

impl<T> UnboundedStorage<T> for Slab<T> {
    #[inline]
    fn insert(&mut self, value: T) -> Self::Key {
        Slab::insert(self, value).key()
    }
}
