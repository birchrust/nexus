//! Internal trait for abstracting over bounded and unbounded slabs.
//!
//! This module provides a sealed trait that enables code reuse between
//! bounded and unbounded storage variants without exposing implementation
//! details to users.

use nexus_slab::{bounded, unbounded, CapacityError, Full, Key};

/// Operations common to bounded::Slab and unbounded::Slab.
///
/// This is a sealed trait - only implemented for `nexus_slab` types.
/// It enables the specialized storage types to share implementation
/// between bounded and unbounded variants.
///
/// # Associated Types
///
/// - `Slot`: The RAII handle returned by insert operations
/// - `VacantSlot`: The pre-allocation handle for self-referential patterns
pub(crate) trait SlabOps<T>: sealed::Sealed + Copy {
    /// The RAII slot handle type (bounded::Slot or unbounded::Slot).
    type Slot;

    /// The vacant slot handle type for pre-allocation patterns.
    type VacantSlot;

    /// Returns the number of occupied slots.
    fn slab_len(&self) -> usize;

    /// Returns `true` if no slots are occupied.
    fn slab_is_empty(&self) -> bool;

    /// Returns the total capacity.
    fn slab_capacity(&self) -> usize;

    /// Returns `true` if the slab is at capacity.
    fn slab_is_full(&self) -> bool;

    /// Returns `true` if the key points to a valid, occupied slot.
    fn slab_contains_key(&self, key: Key) -> bool;

    /// Inserts a value, returning a Slot handle.
    ///
    /// For bounded slabs, returns `Err(Full)` if at capacity.
    /// For unbounded slabs, grows automatically (never fails).
    fn slab_try_insert(&self, value: T) -> Result<Self::Slot, Full<T>>;

    /// Inserts with a closure that receives the slot before the value exists.
    ///
    /// Enables self-referential patterns where the value needs its own key.
    fn slab_insert_with<F>(&self, f: F) -> Result<Self::Slot, CapacityError>
    where
        F: FnOnce(&Self::Slot) -> T;

    /// Reserves a slot without filling it.
    ///
    /// Returns a VacantSlot that can be filled later or dropped to release.
    fn slab_vacant_slot(&self) -> Result<Self::VacantSlot, CapacityError>;

    /// Creates a Slot handle from a key.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    fn slab_slot(&self, key: Key) -> Option<Self::Slot>;

    /// Returns a reference to the value at `key`.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    ///
    /// # Safety
    ///
    /// Caller must ensure no mutable references to this slot exist.
    unsafe fn slab_get(&self, key: Key) -> Option<&T>;

    /// Returns a mutable reference to the value at `key`.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    ///
    /// # Safety
    ///
    /// Caller must ensure no other references to this slot exist.
    #[allow(clippy::mut_from_ref)]
    unsafe fn slab_get_mut(&self, key: Key) -> Option<&mut T>;

    /// Returns a reference without validity checks.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied. No mutable references may exist.
    unsafe fn slab_get_unchecked(&self, key: Key) -> &T;

    /// Returns a mutable reference without validity checks.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied. No other references may exist.
    #[allow(clippy::mut_from_ref)]
    unsafe fn slab_get_unchecked_mut(&self, key: Key) -> &mut T;

    /// Removes the value at `key`, returning it if present.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    fn slab_try_remove(&self, key: Key) -> Option<T>;

    /// Removes the value at `key` without validity checks.
    ///
    /// # Safety
    ///
    /// Key must be valid and occupied.
    unsafe fn slab_remove_unchecked(&self, key: Key) -> T;

    /// Removes all values from the slab.
    fn slab_clear(&self);
}

// =============================================================================
// bounded::Slab implementation
// =============================================================================

impl<T> SlabOps<T> for bounded::Slab<T> {
    type Slot = bounded::Slot<T>;
    type VacantSlot = bounded::VacantSlot<T>;

    #[inline]
    fn slab_len(&self) -> usize {
        self.len()
    }

    #[inline]
    fn slab_is_empty(&self) -> bool {
        self.is_empty()
    }

    #[inline]
    fn slab_capacity(&self) -> usize {
        self.capacity()
    }

    #[inline]
    fn slab_is_full(&self) -> bool {
        self.is_full()
    }

    #[inline]
    fn slab_contains_key(&self, key: Key) -> bool {
        self.contains_key(key)
    }

    #[inline]
    fn slab_try_insert(&self, value: T) -> Result<Self::Slot, Full<T>> {
        self.try_insert(value)
    }

    #[inline]
    fn slab_insert_with<F>(&self, f: F) -> Result<Self::Slot, CapacityError>
    where
        F: FnOnce(&Self::Slot) -> T,
    {
        self.insert_with(f)
    }

    #[inline]
    fn slab_vacant_slot(&self) -> Result<Self::VacantSlot, CapacityError> {
        self.vacant_slot()
    }

    #[inline]
    fn slab_slot(&self, key: Key) -> Option<Self::Slot> {
        self.slot(key)
    }

    #[inline]
    unsafe fn slab_get(&self, key: Key) -> Option<&T> {
        if self.contains_key(key) {
            Some(unsafe { self.get_by_key(key) })
        } else {
            None
        }
    }

    #[inline]
    unsafe fn slab_get_mut(&self, key: Key) -> Option<&mut T> {
        if self.contains_key(key) {
            Some(unsafe { self.get_by_key_mut(key) })
        } else {
            None
        }
    }

    #[inline]
    unsafe fn slab_get_unchecked(&self, key: Key) -> &T {
        unsafe { self.get_by_key(key) }
    }

    #[inline]
    unsafe fn slab_get_unchecked_mut(&self, key: Key) -> &mut T {
        unsafe { self.get_by_key_mut(key) }
    }

    #[inline]
    fn slab_try_remove(&self, key: Key) -> Option<T> {
        if self.contains_key(key) {
            Some(unsafe { self.remove_by_key(key) })
        } else {
            None
        }
    }

    #[inline]
    unsafe fn slab_remove_unchecked(&self, key: Key) -> T {
        unsafe { self.remove_by_key(key) }
    }

    #[inline]
    fn slab_clear(&self) {
        self.clear();
    }
}

// =============================================================================
// unbounded::Slab implementation
// =============================================================================

impl<T> SlabOps<T> for unbounded::Slab<T> {
    type Slot = unbounded::Slot<T>;
    type VacantSlot = unbounded::VacantSlot<T>;

    #[inline]
    fn slab_len(&self) -> usize {
        self.len()
    }

    #[inline]
    fn slab_is_empty(&self) -> bool {
        self.is_empty()
    }

    #[inline]
    fn slab_capacity(&self) -> usize {
        self.capacity()
    }

    #[inline]
    fn slab_is_full(&self) -> bool {
        // Unbounded slab is never full
        false
    }

    #[inline]
    fn slab_contains_key(&self, key: Key) -> bool {
        self.contains_key(key)
    }

    #[inline]
    fn slab_try_insert(&self, value: T) -> Result<Self::Slot, Full<T>> {
        // Unbounded slab never fails, but we match the trait signature
        Ok(self.insert(value))
    }

    #[inline]
    fn slab_insert_with<F>(&self, f: F) -> Result<Self::Slot, CapacityError>
    where
        F: FnOnce(&Self::Slot) -> T,
    {
        // Unbounded slab never fails
        Ok(self.insert_with(f))
    }

    #[inline]
    fn slab_vacant_slot(&self) -> Result<Self::VacantSlot, CapacityError> {
        // Unbounded slab never fails
        Ok(self.vacant_slot())
    }

    #[inline]
    fn slab_slot(&self, key: Key) -> Option<Self::Slot> {
        self.slot(key)
    }

    #[inline]
    unsafe fn slab_get(&self, key: Key) -> Option<&T> {
        if self.contains_key(key) {
            Some(unsafe { self.get_by_key(key) })
        } else {
            None
        }
    }

    #[inline]
    unsafe fn slab_get_mut(&self, key: Key) -> Option<&mut T> {
        if self.contains_key(key) {
            Some(unsafe { self.get_by_key_mut(key) })
        } else {
            None
        }
    }

    #[inline]
    unsafe fn slab_get_unchecked(&self, key: Key) -> &T {
        unsafe { self.get_by_key(key) }
    }

    #[inline]
    unsafe fn slab_get_unchecked_mut(&self, key: Key) -> &mut T {
        unsafe { self.get_by_key_mut(key) }
    }

    #[inline]
    fn slab_try_remove(&self, key: Key) -> Option<T> {
        if self.contains_key(key) {
            Some(unsafe { self.remove_by_key(key) })
        } else {
            None
        }
    }

    #[inline]
    unsafe fn slab_remove_unchecked(&self, key: Key) -> T {
        unsafe { self.remove_by_key(key) }
    }

    #[inline]
    fn slab_clear(&self) {
        self.clear();
    }
}

// =============================================================================
// Sealed trait
// =============================================================================

mod sealed {
    use nexus_slab::{bounded, unbounded};

    pub trait Sealed {}

    impl<T> Sealed for bounded::Slab<T> {}
    impl<T> Sealed for unbounded::Slab<T> {}
}
