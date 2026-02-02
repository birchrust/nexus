//! Shared internals for bounded and unbounded slab implementations.

use std::cell::{Cell, UnsafeCell};
use std::mem::MaybeUninit;

use crate::Key;

// =============================================================================
// Constants
// =============================================================================

/// Vacant flag - bit 63 of stamp
pub const VACANT_BIT: u64 = 1 << 63;

/// Mask for key (lower 32 bits of stamp)
pub(crate) const KEY_MASK: u64 = 0x0000_0000_FFFF_FFFF;

/// Mask for next_free index within state (bits 29-0 after shifting)
pub(crate) const INDEX_MASK: u64 = (1 << 30) - 1;

/// Sentinel for end of freelist (~1 billion max capacity).
///
/// This is the max 30-bit value, limiting addressable slots to ~1 billion.
pub const SLOT_NONE: u32 = INDEX_MASK as u32;

// =============================================================================
// SlotCell
// =============================================================================

/// Internal slot storage with stamp for state tracking and key storage.
///
/// Stamp encoding (64-bit):
/// - Bits 63-32: State
///   - Bit 63: Vacant flag (1 = vacant, 0 = occupied)
///   - Bits 61-32: When vacant, next free slot index (30 bits)
/// - Bits 31-0: Key (stored when slot is claimed, valid regardless of state)
#[repr(C)]
pub struct SlotCell<T> {
    stamp: Cell<u64>,
    /// The value storage.
    pub value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> SlotCell<T> {
    pub(crate) fn new_vacant(next_free: u32) -> Self {
        Self {
            stamp: Cell::new(VACANT_BIT | ((next_free as u64 & INDEX_MASK) << 32)),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Returns `true` if the slot is vacant.
    #[inline]
    pub fn is_vacant(&self) -> bool {
        self.stamp.get() & VACANT_BIT != 0
    }

    /// Returns `true` if the slot is occupied.
    #[inline]
    pub fn is_occupied(&self) -> bool {
        !self.is_vacant()
    }

    /// Returns a pointer to the value.
    ///
    /// # Safety
    ///
    /// The slot must be occupied. Caller must ensure no mutable references exist.
    #[inline]
    pub unsafe fn value_ptr(&self) -> *const T {
        unsafe { (*self.value.get()).as_ptr() }
    }

    /// Returns a mutable pointer to the value.
    ///
    /// # Safety
    ///
    /// The slot must be occupied. Caller must ensure exclusive access.
    #[inline]
    pub unsafe fn value_ptr_mut(&self) -> *mut T {
        unsafe { (*self.value.get()).as_mut_ptr() }
    }

    /// Returns a reference to the value.
    ///
    /// # Safety
    ///
    /// The slot must be occupied. Caller must ensure no mutable references exist.
    #[inline]
    pub unsafe fn get_value(&self) -> &T {
        unsafe { (*self.value.get()).assume_init_ref() }
    }

    /// Returns a mutable reference to the value.
    ///
    /// # Safety
    ///
    /// The slot must be occupied. Caller must ensure exclusive access.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_value_mut(&self) -> &mut T {
        unsafe { (*self.value.get()).assume_init_mut() }
    }

    /// Claims a vacant slot, returning the next_free index.
    /// Single stamp read. Use with set_key_occupied() for optimal insert.
    #[inline]
    pub(crate) fn claim_next_free(&self) -> u32 {
        let stamp = self.stamp.get();
        debug_assert!(stamp & VACANT_BIT != 0, "claim on non-vacant slot");
        ((stamp >> 32) & INDEX_MASK) as u32
    }

    /// Returns the key stored in the stamp.
    /// Valid regardless of vacant/occupied state (key is set when slot is claimed).
    #[inline]
    pub fn key_from_stamp(&self) -> u32 {
        (self.stamp.get() & KEY_MASK) as u32
    }

    /// Sets key and marks slot as occupied in a single write.
    /// Use this for normal insert path after claim_next_free().
    #[inline]
    pub fn set_key_occupied(&self, key: u32) {
        // Key in bits 31-0, bits 63-32 = 0 means occupied
        self.stamp.set(key as u64);
    }

    /// Marks slot as vacant with given next_free index. Clobbers key.
    #[inline]
    pub(crate) fn set_vacant(&self, next_free: u32) {
        self.stamp
            .set(VACANT_BIT | ((next_free as u64 & INDEX_MASK) << 32));
    }
}

// =============================================================================
// VTable
// =============================================================================

use std::marker::PhantomData;

/// Result of claiming a slot (before value is written).
///
/// The slot is reserved but not yet occupied. The caller must write the value
/// and mark the slot as occupied.
#[derive(Debug, Clone, Copy)]
pub struct ClaimedSlot {
    /// Type-erased pointer to the SlotCell.
    pub slot_ptr: *mut (),
    /// The key for this slot.
    pub key: Key,
}

/// Claims a slot from the slab, returning slot pointer and key.
///
/// Returns `None` if the slab is full (bounded) or allocation fails.
/// The slot is reserved but not occupied - caller must write the value.
///
/// # Safety
/// - `inner` must be a valid pointer to the slab's inner state
/// - Must be called from single thread (slab is !Send)
pub type TryClaimFn = unsafe fn(inner: *mut ()) -> Option<ClaimedSlot>;

/// Frees a slot, returning it to the freelist.
///
/// Does NOT drop the value - caller is responsible for dropping before calling.
///
/// # Safety
/// - `inner` must be a valid pointer to the slab's inner state
/// - `key` must refer to a slot that was previously claimed
/// - Value must already be dropped or moved out
pub type FreeFn = unsafe fn(inner: *mut (), key: Key);

/// Gets the slot pointer for a key.
///
/// # Safety
/// - `inner` must be a valid pointer to the slab's inner state
/// - `key` must refer to a valid slot (bounds checking is caller's responsibility)
pub type SlotPtrFn = unsafe fn(inner: *const (), key: Key) -> *mut ();

/// Checks if a key refers to an occupied slot.
///
/// # Safety
/// - `inner` must be a valid pointer to the slab's inner state
pub type ContainsKeyFn = unsafe fn(inner: *const (), key: Key) -> bool;

/// VTable for type-erased slab operations.
///
/// Contains both the function pointers for slab operations and the pointer
/// to the slab's inner state.
///
/// The type parameter `T` provides type safety - a `VTable<Order>` can only
/// be used with `Order` values, even though the function pointers are
/// type-erased internally.
///
/// # Usage
///
/// ```ignore
/// let alloc: Allocator<Order> = Allocator::builder().bounded(1024).build();
/// let vtable = alloc.vtable();
///
/// // Use methods for operations
/// unsafe {
///     let claimed = vtable.try_claim()?;
///     // ... write value ...
///     vtable.free(key);
/// }
/// ```
pub struct VTable<T> {
    /// Pointer to the slab's inner state (type-erased).
    inner: *mut (),
    /// Claims a slot from the slab.
    try_claim_fn: TryClaimFn,
    /// Frees a slot back to the slab.
    free_fn: FreeFn,
    /// Gets slot pointer from key.
    slot_ptr_fn: SlotPtrFn,
    /// Checks if key is valid and occupied.
    contains_key_fn: ContainsKeyFn,
    /// Whether this is a bounded allocator.
    bounded: bool,
    /// Marker for type safety.
    _marker: PhantomData<T>,
}

impl<T> VTable<T> {
    /// Creates a new VTable with the given function pointers.
    ///
    /// `inner` should be set to the slab's inner pointer after creation.
    #[inline]
    pub const fn new(
        try_claim_fn: TryClaimFn,
        free_fn: FreeFn,
        slot_ptr_fn: SlotPtrFn,
        contains_key_fn: ContainsKeyFn,
    ) -> Self {
        Self {
            inner: std::ptr::null_mut(),
            try_claim_fn,
            free_fn,
            slot_ptr_fn,
            contains_key_fn,
            bounded: false,
            _marker: PhantomData,
        }
    }

    /// Sets the inner pointer.
    ///
    /// # Safety
    /// - Must only be called once
    /// - `inner` must be a valid pointer to the slab's inner state
    #[inline]
    pub unsafe fn set_inner(&mut self, inner: *mut ()) {
        self.inner = inner;
    }

    /// Returns the inner pointer.
    #[inline]
    pub fn inner(&self) -> *mut () {
        self.inner
    }

    /// Sets whether this is a bounded allocator.
    #[inline]
    pub fn set_bounded(&mut self, bounded: bool) {
        self.bounded = bounded;
    }

    /// Returns whether this is a bounded allocator.
    #[inline]
    pub fn is_bounded(&self) -> bool {
        self.bounded
    }

    // =========================================================================
    // Public methods - these are the intended API for external use
    // =========================================================================

    /// Claims a slot from the slab.
    ///
    /// Returns `None` if the slab is full (bounded) or allocation fails.
    /// The slot is reserved but not occupied - caller must write the value
    /// and call `SlotCell::set_key_occupied()`.
    ///
    /// # Safety
    ///
    /// Must be called from the thread that owns this slab.
    #[inline]
    pub unsafe fn try_claim(&self) -> Option<ClaimedSlot> {
        unsafe { (self.try_claim_fn)(self.inner) }
    }

    /// Frees a slot, returning it to the freelist.
    ///
    /// Does NOT drop the value - caller is responsible for dropping before calling.
    ///
    /// # Safety
    ///
    /// - `key` must refer to a slot that was previously claimed
    /// - Value must already be dropped or moved out
    /// - Must be called from the thread that owns this slab
    #[inline]
    pub unsafe fn free(&self, key: Key) {
        unsafe { (self.free_fn)(self.inner, key) }
    }

    /// Gets the slot pointer for a key.
    ///
    /// # Safety
    ///
    /// - `key` must refer to a valid slot (bounds checking is caller's responsibility)
    /// - Must be called from the thread that owns this slab
    #[inline]
    pub unsafe fn slot_ptr(&self, key: Key) -> *mut SlotCell<T> {
        unsafe { (self.slot_ptr_fn)(self.inner.cast_const(), key) as *mut SlotCell<T> }
    }

    /// Checks if a key refers to an occupied slot.
    ///
    /// # Safety
    ///
    /// Must be called from the thread that owns this slab.
    #[inline]
    pub unsafe fn contains_key(&self, key: Key) -> bool {
        unsafe { (self.contains_key_fn)(self.inner.cast_const(), key) }
    }
}
