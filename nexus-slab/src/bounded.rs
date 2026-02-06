//! Fixed-capacity slab allocator.
//!
//! This module provides a bounded (fixed-capacity) slab allocator.
//!
//! # Example
//!
//! ```
//! use nexus_slab::bounded::Slab;
//!
//! // SAFETY: slab must outlive all allocated Slots
//! let slab = unsafe { Slab::new(1024) };
//! let slot = slab.alloc(42u64);
//! assert_eq!(*slot, 42);
//! // Raw slot - must explicitly free
//! // SAFETY: slot was allocated from this slab
//! unsafe { slab.dealloc(slot) };
//! ```

use std::cell::Cell;
use std::fmt;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ptr;

use crate::alloc::Full;
use crate::shared::{SLOT_NONE, Slot, SlotCell};

// =============================================================================
// SlabInner
// =============================================================================

/// Internal state for a fixed-capacity slab.
///
/// Uses pointer-based freelist for O(1) allocation without index arithmetic.
///
/// # Const Construction
///
/// This type supports const construction via [`new()`](Self::new) followed by
/// runtime initialization via [`init()`](Self::init). This enables use with
/// `thread_local!` using the `const { }` block syntax for zero-overhead TLS access.
///
/// ```ignore
/// thread_local! {
///     static SLAB: SlabInner<MyType> = const { SlabInner::new() };
/// }
///
/// // Later, at runtime:
/// SLAB.with(|s| s.init(1024));
/// ```
#[doc(hidden)]
pub struct SlabInner<T> {
    /// Slot storage. Wrapped in UnsafeCell for interior mutability during init.
    slots: std::cell::UnsafeCell<Vec<SlotCell<T>>>,
    /// Capacity. Wrapped in Cell so it can be set during init.
    capacity: Cell<u32>,
    /// Head of freelist - raw pointer for fast allocation.
    /// NULL when slab is full or uninitialized.
    #[doc(hidden)]
    pub free_head: Cell<*mut SlotCell<T>>,
}

impl<T> SlabInner<T> {
    /// Creates an empty, uninitialized slab.
    ///
    /// This is a const function that performs no allocation. Call [`init()`](Self::init)
    /// to allocate storage before use.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // For use with thread_local! const initialization
    /// thread_local! {
    ///     static SLAB: SlabInner<u64> = const { SlabInner::new() };
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

    /// Initializes the slab with the given capacity.
    ///
    /// This allocates slot storage and builds the freelist. Must be called
    /// exactly once before any allocations.
    ///
    /// # Panics
    ///
    /// - Panics if the slab is already initialized (capacity > 0)
    /// - Panics if capacity is zero
    /// - Panics if capacity exceeds maximum (SLOT_NONE)
    pub fn init(&self, capacity: u32) {
        assert!(self.capacity.get() == 0, "SlabInner already initialized");
        assert!(capacity > 0, "capacity must be non-zero");
        assert!(capacity < SLOT_NONE, "capacity exceeds maximum");

        // SAFETY: We have &self and verified capacity == 0, so no other code
        // can be accessing slots. This is the only mutation point.
        let slots = unsafe { &mut *self.slots.get() };

        // Allocate slots — all initially vacant
        slots.reserve_exact(capacity as usize);
        for _ in 0..capacity {
            slots.push(SlotCell::vacant(ptr::null_mut()));
        }

        // Wire up the freelist: each slot's next_free points to the next slot
        for i in 0..(capacity as usize - 1) {
            let next_ptr = slots.as_mut_ptr().wrapping_add(i + 1);
            // Slot is vacant, we're building the freelist during init.
            // Writing to a union field is safe when we own the value.
            slots[i].next_free = next_ptr;
        }
        // Last slot points to NULL (end of freelist) — already null from vacant()

        let free_head = slots.as_mut_ptr(); // First slot
        self.capacity.set(capacity);
        self.free_head.set(free_head);
    }

    /// Returns true if the slab has been initialized.
    #[inline]
    #[allow(dead_code)] // Used by macro-generated code
    pub fn is_initialized(&self) -> bool {
        self.capacity.get() > 0
    }

    /// Creates a new slab inner with the given capacity.
    ///
    /// This is a convenience method equivalent to `new()` followed by `init()`.
    pub fn with_capacity(capacity: u32) -> Self {
        let inner = Self::new();
        inner.init(capacity);
        inner
    }

    /// Returns the capacity.
    #[inline]
    pub fn capacity(&self) -> u32 {
        self.capacity.get()
    }

    /// Returns the base pointer to the slots array.
    ///
    /// Returns mutable pointer because slot mutation uses raw pointer access.
    #[doc(hidden)]
    #[inline]
    pub fn slots_ptr(&self) -> *mut SlotCell<T> {
        // SAFETY: We're returning a pointer for use with raw pointer access
        let slots = unsafe { &*self.slots.get() };
        slots.as_ptr().cast_mut()
    }

    // =========================================================================
    // Allocation methods
    // =========================================================================

    /// Claims a slot and writes the value.
    ///
    /// Returns `Err(Full(value))` if the slab is full.
    pub fn try_alloc(&self, value: T) -> Result<*mut SlotCell<T>, Full<T>> {
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

        Ok(slot_ptr)
    }

    /// Returns a slot to the freelist by pointer.
    ///
    /// Does NOT drop the value — caller must drop before calling.
    /// This is the hot-path variant that avoids the key→pointer round-trip.
    ///
    /// # Safety
    ///
    /// - `slot_ptr` must point to a previously claimed slot within this slab
    /// - Value must already be dropped or moved out
    #[doc(hidden)]
    #[inline]
    pub unsafe fn dealloc_ptr(&self, slot_ptr: *mut SlotCell<T>) {
        // Write freelist link — overwrites value bytes (union semantics)
        let free_head = self.free_head.get();
        unsafe {
            (*slot_ptr).next_free = free_head;
        }
        self.free_head.set(slot_ptr);
    }
}

impl<T> Default for SlabInner<T> {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Slab
// =============================================================================

/// Pre-allocated fixed-capacity slab.
///
/// `Slab<T>` owns its storage via a `Box<SlabInner<T>>`.
///
/// # Thread Safety
///
/// `Slab` is `!Send` and `!Sync`. Each slab must be used from a single thread.
///
/// # Example
///
/// ```
/// use nexus_slab::bounded::Slab;
///
/// // SAFETY: slab must outlive all allocated Slots
/// let slab = unsafe { Slab::new(1024) };
/// let slot = slab.alloc(42u64);
/// assert_eq!(*slot, 42);
/// // SAFETY: slot was allocated from this slab
/// unsafe { slab.dealloc(slot) };
/// ```
pub struct Slab<T> {
    inner: Box<SlabInner<T>>,
}

impl<T> Slab<T> {
    /// Creates a new bounded slab with the given capacity.
    ///
    /// # Safety
    ///
    /// The caller must ensure this `Slab` outlives all [`Slot`]s allocated from it.
    /// Dropping the `Slab` while `Slot`s are outstanding results in use-after-free.
    ///
    /// # Panics
    ///
    /// - Panics if capacity is zero
    /// - Panics if capacity exceeds maximum
    #[inline]
    pub unsafe fn new(capacity: u32) -> Self {
        let inner = Box::new(SlabInner::with_capacity(capacity));
        Self { inner }
    }

    /// Returns the capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity() as usize
    }

    // =========================================================================
    // Raw API (Layer 1)
    // =========================================================================

    /// Allocates a slot and writes the value. Returns a raw slot handle.
    ///
    /// # Panics
    ///
    /// Panics if the slab is full.
    #[inline]
    pub fn alloc(&self, value: T) -> Slot<T> {
        self.try_alloc(value).unwrap_or_else(|_| panic!("slab full"))
    }

    /// Tries to allocate a slot and write the value.
    ///
    /// Returns `Err(Full(value))` if the slab is at capacity.
    #[inline]
    pub fn try_alloc(&self, value: T) -> Result<Slot<T>, Full<T>> {
        let slot_ptr = self.inner.try_alloc(value)?;
        // SAFETY: try_alloc returns a valid, occupied slot pointer
        Ok(unsafe { Slot::from_ptr(slot_ptr) })
    }

    /// Deallocates a slot, dropping the value and returning storage to the freelist.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from **this** slab (not a different slab)
    /// - No references to the slot's value may exist
    ///
    /// Note: Double-free is prevented at compile time (`Slot` is move-only).
    #[inline]
    #[allow(clippy::needless_pass_by_value)] // Intentional: consumes slot to prevent reuse
    pub unsafe fn dealloc(&self, slot: Slot<T>) {
        // Drop the value in place
        // SAFETY: Caller guarantees slot is valid and occupied
        unsafe {
            ptr::drop_in_place((*(*slot.as_ptr()).value).as_mut_ptr());
        }
        // Return to freelist
        // SAFETY: Value dropped, slot valid
        unsafe { self.inner.dealloc_ptr(slot.as_ptr()) };
    }

    /// Deallocates a slot and returns the value without dropping it.
    ///
    /// # Safety
    ///
    /// - `slot` must have been allocated from **this** slab (not a different slab)
    /// - No references to the slot's value may exist
    ///
    /// Note: Double-free is prevented at compile time (`Slot` is move-only).
    #[inline]
    #[allow(clippy::needless_pass_by_value)] // Intentional: consumes slot to prevent reuse
    pub unsafe fn dealloc_take(&self, slot: Slot<T>) -> T {
        // Move the value out
        // SAFETY: Caller guarantees slot is valid and occupied
        let value = unsafe { ptr::read((*slot.as_ptr()).value.as_ptr()) };
        // Return to freelist
        // SAFETY: Value moved out, slot valid
        unsafe { self.inner.dealloc_ptr(slot.as_ptr()) };
        value
    }
}

impl<T> fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slab")
            .field("capacity", &self.inner.capacity())
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
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<u64>::new(100) };
        assert_eq!(slab.capacity(), 100);

        let slot = slab.alloc(42);
        assert_eq!(*slot, 42);
        // SAFETY: slot was allocated from this slab
        unsafe { slab.dealloc(slot) };
    }

    #[test]
    fn slab_full() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<u64>::new(2) };
        let s1 = slab.alloc(1);
        let s2 = slab.alloc(2);

        let result = slab.try_alloc(3);
        assert!(result.is_err());
        let recovered = result.unwrap_err().into_inner();
        assert_eq!(recovered, 3);

        // SAFETY: slots were allocated from this slab
        unsafe {
            slab.dealloc(s1);
            slab.dealloc(s2);
        }
    }

    #[test]
    fn slot_deref_mut() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<String>::new(10) };
        let mut slot = slab.alloc("hello".to_string());
        slot.push_str(" world");
        assert_eq!(&*slot, "hello world");
        // SAFETY: slot was allocated from this slab
        unsafe { slab.dealloc(slot) };
    }

    #[test]
    fn slot_dealloc_take() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<String>::new(10) };
        let slot = slab.alloc("hello".to_string());

        // SAFETY: slot was allocated from this slab
        let value = unsafe { slab.dealloc_take(slot) };
        assert_eq!(value, "hello");
    }

    #[test]
    fn slot_size() {
        // Raw Slot<T> is 8 bytes (one pointer)
        assert_eq!(std::mem::size_of::<Slot<u64>>(), 8);
    }

    #[test]
    fn slab_debug() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<u64>::new(10) };
        let s = slab.alloc(42);
        let debug = format!("{:?}", slab);
        assert!(debug.contains("Slab"));
        assert!(debug.contains("capacity"));
        // SAFETY: slot was allocated from slab
        unsafe { slab.dealloc(s) };
    }

    #[test]
    fn borrow_traits() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<u64>::new(10) };
        let mut slot = slab.alloc(42);

        let borrowed: &u64 = slot.borrow();
        assert_eq!(*borrowed, 42);

        let borrowed_mut: &mut u64 = slot.borrow_mut();
        *borrowed_mut = 100;
        assert_eq!(*slot, 100);

        // SAFETY: slot was allocated from slab
        unsafe { slab.dealloc(slot) };
    }

    #[test]
    fn capacity_one() {
        // SAFETY: slab outlives all slots
        let slab = unsafe { Slab::<u64>::new(1) };

        assert_eq!(slab.capacity(), 1);

        let slot = slab.alloc(42);
        assert!(slab.try_alloc(100).is_err());

        // SAFETY: slot was allocated from this slab
        unsafe { slab.dealloc(slot) };

        let slot2 = slab.alloc(100);
        assert_eq!(*slot2, 100);
        // SAFETY: slot2 was allocated from this slab
        unsafe { slab.dealloc(slot2) };
    }
}
