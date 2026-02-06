//! Fixed-capacity slab allocator.
//!
//! This module provides a bounded (fixed-capacity) slab allocator.
//!
//! # Example
//!
//! ```
//! use nexus_slab::bounded::Slab;
//!
//! let slab = Slab::with_capacity(1024);
//! let slot = slab.alloc(42u64);
//! assert_eq!(*slot, 42);
//! // SAFETY: slot was allocated from this slab
//! unsafe { slab.free(slot) };
//! ```

use std::cell::Cell;
use std::fmt;
use std::mem::{self, ManuallyDrop, MaybeUninit};
use std::ptr;

use crate::alloc::Full;
use crate::shared::{Slot, SlotCell};

// =============================================================================
// Claim
// =============================================================================

/// A claimed slot that has not yet been written to.
///
/// Created by [`Slab::claim()`]. Must be consumed via [`write()`](Self::write)
/// to complete the allocation. If dropped without calling `write()`, the slot
/// is returned to the freelist.
///
/// The `write()` method is `#[inline]`, enabling the compiler to potentially
/// optimize the value write as a placement new (constructing directly into
/// the slot memory).
pub struct Claim<'a, T> {
    slot_ptr: *mut SlotCell<T>,
    slab: &'a Slab<T>,
}

impl<T> Claim<'_, T> {
    /// Writes the value to the claimed slot and returns the [`Slot`] handle.
    ///
    /// This consumes the claim. The value is written directly to the slot's
    /// memory, which may enable placement new optimization.
    #[inline]
    pub fn write(self, value: T) -> Slot<T> {
        let slot_ptr = self.slot_ptr;
        // SAFETY: We own this slot from claim(), it's valid and vacant
        unsafe {
            (*slot_ptr).value = ManuallyDrop::new(MaybeUninit::new(value));
        }
        // Don't run Drop - we're completing the allocation
        mem::forget(self);
        // SAFETY: slot_ptr is valid and now occupied
        unsafe { Slot::from_ptr(slot_ptr) }
    }
}

impl<T> Drop for Claim<'_, T> {
    fn drop(&mut self) {
        // Abandoned claim - return slot to freelist
        // SAFETY: slot_ptr is valid and still vacant (never written to)
        let free_head = self.slab.free_head.get();
        unsafe {
            (*self.slot_ptr).next_free = free_head;
        }
        self.slab.free_head.set(self.slot_ptr);
    }
}

// =============================================================================
// Slab
// =============================================================================

/// Fixed-capacity slab allocator.
///
/// Uses pointer-based freelist for O(1) allocation.
///
/// # Const Construction
///
/// Supports const construction via [`new()`](Self::new) followed by
/// runtime initialization via [`init()`](Self::init). This enables use with
/// `thread_local!` using the `const { }` block syntax for zero-overhead TLS access.
///
/// ```ignore
/// thread_local! {
///     static SLAB: Slab<MyType> = const { Slab::new() };
/// }
///
/// // Later, at runtime:
/// SLAB.with(|s| s.init(1024));
/// ```
///
/// For direct usage, prefer [`with_capacity()`](Self::with_capacity).
pub struct Slab<T> {
    /// Slot storage. Wrapped in UnsafeCell for interior mutability.
    slots: std::cell::UnsafeCell<Vec<SlotCell<T>>>,
    /// Capacity. Wrapped in Cell so it can be set during init.
    capacity: Cell<usize>,
    /// Head of freelist - raw pointer for fast allocation.
    /// NULL when slab is full or uninitialized.
    #[doc(hidden)]
    pub free_head: Cell<*mut SlotCell<T>>,
}

impl<T> Slab<T> {
    /// Creates an empty, uninitialized slab.
    ///
    /// This is a const function that performs no allocation. Call [`init()`](Self::init)
    /// to allocate storage before use.
    ///
    /// For direct usage, prefer [`with_capacity()`](Self::with_capacity).
    ///
    /// # Example
    ///
    /// ```ignore
    /// // For use with thread_local! const initialization
    /// thread_local! {
    ///     static SLAB: Slab<u64> = const { Slab::new() };
    /// }
    /// ```
    #[inline]
    pub const fn new() -> Self {
        Self {
            slots: std::cell::UnsafeCell::new(Vec::new()),
            capacity: Cell::new(0),
            free_head: Cell::new(ptr::null_mut()),
        }
    }

    /// Creates a new slab with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if capacity is zero.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        let slab = Self::new();
        slab.init(capacity);
        slab
    }

    /// Initializes the slab with the given capacity.
    ///
    /// This allocates slot storage and builds the freelist. Must be called
    /// exactly once before any allocations.
    ///
    /// # Panics
    ///
    /// - Panics if the slab is already initialized (capacity > 0)
    /// - Panics if capacity is zero
    pub fn init(&self, capacity: usize) {
        assert!(self.capacity.get() == 0, "Slab already initialized");
        assert!(capacity > 0, "capacity must be non-zero");

        // SAFETY: We have &self and verified capacity == 0, so no other code
        // can be accessing slots. This is the only mutation point.
        let slots = unsafe { &mut *self.slots.get() };

        // Allocate slots — all initially vacant
        slots.reserve_exact(capacity);
        for _ in 0..capacity {
            slots.push(SlotCell::vacant(ptr::null_mut()));
        }

        // Wire up the freelist: each slot's next_free points to the next slot
        for i in 0..(capacity - 1) {
            let next_ptr = slots.as_mut_ptr().wrapping_add(i + 1);
            slots[i].next_free = next_ptr;
        }
        // Last slot points to NULL (end of freelist) — already null from vacant()

        let free_head = slots.as_mut_ptr();
        self.capacity.set(capacity);
        self.free_head.set(free_head);
    }

    /// Returns true if the slab has been initialized.
    #[inline]
    pub fn is_initialized(&self) -> bool {
        self.capacity.get() > 0
    }

    /// Returns the capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity.get()
    }

    /// Returns the base pointer to the slots array.
    #[inline]
    pub(crate) fn slots_ptr(&self) -> *mut SlotCell<T> {
        // SAFETY: We're returning a pointer for use with raw pointer access
        let slots = unsafe { &*self.slots.get() };
        slots.as_ptr().cast_mut()
    }

    // =========================================================================
    // Allocation API
    // =========================================================================

    /// Claims a slot from the freelist without writing a value.
    ///
    /// Returns `None` if the slab is full. The returned [`Claim`] must be
    /// consumed via [`Claim::write()`] to complete the allocation.
    ///
    /// This two-phase allocation enables placement new optimization: the
    /// value can be constructed directly into the slot memory.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::bounded::Slab;
    ///
    /// let slab = Slab::with_capacity(10);
    /// if let Some(claim) = slab.claim() {
    ///     let slot = claim.write(42u64);
    ///     assert_eq!(*slot, 42);
    ///     // SAFETY: slot was allocated from this slab
    ///     unsafe { slab.free(slot) };
    /// }
    /// ```
    #[inline]
    pub fn claim(&self) -> Option<Claim<'_, T>> {
        self.claim_ptr().map(|slot_ptr| Claim {
            slot_ptr,
            slab: self,
        })
    }

    /// Claims a slot from the freelist, returning the raw pointer.
    ///
    /// Returns `None` if the slab is full. This is a low-level API for
    /// macro-generated code that needs to escape TLS closures.
    ///
    /// # Safety Contract
    ///
    /// The caller MUST either:
    /// - Write a value to the slot and use it as an allocated slot, OR
    /// - Return the pointer to the freelist via `free_ptr()` if abandoning
    #[doc(hidden)]
    #[inline]
    pub fn claim_ptr(&self) -> Option<*mut SlotCell<T>> {
        let slot_ptr = self.free_head.get();

        if slot_ptr.is_null() {
            return None;
        }

        // SAFETY: slot_ptr came from the freelist within this slab.
        // The slot is vacant, so next_free is the active union field.
        let next_free = unsafe { (*slot_ptr).next_free };

        // Update freelist head
        self.free_head.set(next_free);

        Some(slot_ptr)
    }

    /// Allocates a slot and writes the value.
    ///
    /// # Panics
    ///
    /// Panics if the slab is full.
    #[inline]
    pub fn alloc(&self, value: T) -> Slot<T> {
        self.try_alloc(value)
            .unwrap_or_else(|_| panic!("slab full"))
    }

    /// Tries to allocate a slot and write the value.
    ///
    /// Returns `Err(Full(value))` if the slab is at capacity.
    #[inline]
    pub fn try_alloc(&self, value: T) -> Result<Slot<T>, Full<T>> {
        let slot_ptr = self.free_head.get();

        if slot_ptr.is_null() {
            return Err(Full(value));
        }

        // SAFETY: slot_ptr came from the freelist within this slab.
        // The slot is vacant, so next_free is the active union field.
        let next_free = unsafe { (*slot_ptr).next_free };

        // Write the value — this overwrites next_free (union semantics)
        // SAFETY: Slot is claimed from freelist, we have exclusive access
        unsafe {
            (*slot_ptr).value = ManuallyDrop::new(MaybeUninit::new(value));
        }

        // Update freelist head
        self.free_head.set(next_free);

        // SAFETY: slot_ptr is valid and occupied
        Ok(unsafe { Slot::from_ptr(slot_ptr) })
    }

    /// Frees a slot, dropping the value and returning storage to the freelist.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from **this** slab
    /// - No references to the slot's value may exist
    #[inline]
    #[allow(clippy::needless_pass_by_value)]
    pub unsafe fn free(&self, slot: Slot<T>) {
        let slot_ptr = slot.as_ptr();
        // SAFETY: Caller guarantees slot is valid and occupied
        unsafe {
            ptr::drop_in_place((*(*slot_ptr).value).as_mut_ptr());
            self.free_ptr(slot_ptr);
        }
    }

    /// Frees a slot and returns the value without dropping it.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from **this** slab
    /// - No references to the slot's value may exist
    #[inline]
    #[allow(clippy::needless_pass_by_value)]
    pub unsafe fn take(&self, slot: Slot<T>) -> T {
        let slot_ptr = slot.as_ptr();
        // SAFETY: Caller guarantees slot is valid and occupied
        unsafe {
            let value = ptr::read((*slot_ptr).value.as_ptr());
            self.free_ptr(slot_ptr);
            value
        }
    }

    /// Returns a slot to the freelist by pointer.
    ///
    /// Does NOT drop the value — caller must drop before calling.
    ///
    /// # Safety
    ///
    /// - `slot_ptr` must point to a slot within this slab
    /// - Value must already be dropped or moved out
    #[doc(hidden)]
    #[inline]
    pub unsafe fn free_ptr(&self, slot_ptr: *mut SlotCell<T>) {
        let free_head = self.free_head.get();
        // SAFETY: Caller guarantees slot_ptr is valid
        unsafe {
            (*slot_ptr).next_free = free_head;
        }
        self.free_head.set(slot_ptr);
    }
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            .field("capacity", &self.capacity.get())
            .finish()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::{Borrow, BorrowMut};

    #[test]
    fn slab_basic() {
        let slab = Slab::<u64>::with_capacity(100);
        assert_eq!(slab.capacity(), 100);

        let slot = slab.alloc(42);
        assert_eq!(*slot, 42);
        // SAFETY: slot was allocated from this slab
        unsafe { slab.free(slot) };
    }

    #[test]
    fn slab_full() {
        let slab = Slab::<u64>::with_capacity(2);
        let s1 = slab.alloc(1);
        let s2 = slab.alloc(2);

        let result = slab.try_alloc(3);
        assert!(result.is_err());
        let recovered = result.unwrap_err().into_inner();
        assert_eq!(recovered, 3);

        // SAFETY: slots were allocated from this slab
        unsafe {
            slab.free(s1);
            slab.free(s2);
        }
    }

    #[test]
    fn slot_deref_mut() {
        let slab = Slab::<String>::with_capacity(10);
        let mut slot = slab.alloc("hello".to_string());
        slot.push_str(" world");
        assert_eq!(&*slot, "hello world");
        // SAFETY: slot was allocated from this slab
        unsafe { slab.free(slot) };
    }

    #[test]
    fn slot_dealloc_take() {
        let slab = Slab::<String>::with_capacity(10);
        let slot = slab.alloc("hello".to_string());

        // SAFETY: slot was allocated from this slab
        let value = unsafe { slab.take(slot) };
        assert_eq!(value, "hello");
    }

    #[test]
    fn slot_size() {
        assert_eq!(std::mem::size_of::<Slot<u64>>(), 8);
    }

    #[test]
    fn slab_debug() {
        let slab = Slab::<u64>::with_capacity(10);
        let s = slab.alloc(42);
        let debug = format!("{:?}", slab);
        assert!(debug.contains("Slab"));
        assert!(debug.contains("capacity"));
        // SAFETY: slot was allocated from slab
        unsafe { slab.free(s) };
    }

    #[test]
    fn borrow_traits() {
        let slab = Slab::<u64>::with_capacity(10);
        let mut slot = slab.alloc(42);

        let borrowed: &u64 = slot.borrow();
        assert_eq!(*borrowed, 42);

        let borrowed_mut: &mut u64 = slot.borrow_mut();
        *borrowed_mut = 100;
        assert_eq!(*slot, 100);

        // SAFETY: slot was allocated from slab
        unsafe { slab.free(slot) };
    }

    #[test]
    fn capacity_one() {
        let slab = Slab::<u64>::with_capacity(1);

        assert_eq!(slab.capacity(), 1);

        let slot = slab.alloc(42);
        assert!(slab.try_alloc(100).is_err());

        // SAFETY: slot was allocated from this slab
        unsafe { slab.free(slot) };

        let slot2 = slab.alloc(100);
        assert_eq!(*slot2, 100);
        // SAFETY: slot2 was allocated from this slab
        unsafe { slab.free(slot2) };
    }
}
