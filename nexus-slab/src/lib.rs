//! # nexus-slab
//!
//! A high-performance slab allocator for **stable memory addresses** without heap
//! allocation overhead.
//!
//! # What Is This?
//!
//! `nexus-slab` is a **custom allocator pattern**—not a replacement for Rust's global
//! allocator, but a specialized allocator for:
//!
//! - **Stable memory addresses** - pointers remain valid until explicitly freed
//! - **Box-like semantics without Box** - RAII ownership with pre-allocated storage
//! - **Node-based data structures** - linked lists, trees, graphs with internal pointers
//! - **Predictable tail latency** - no reallocation spikes during growth
//!
//! Think of [`Slot<T>`] as analogous to `Box<T>`: an owning handle that provides access
//! to a value and deallocates on drop. The difference is that `Box` allocates from the
//! heap on every call, while `Slot` allocates from a pre-allocated slab—O(1) with no
//! syscalls.
//!
//! # Quick Start
//!
//! ```
//! use nexus_slab::Allocator;
//!
//! // Create an allocator (bounded = fixed capacity)
//! let orders: Allocator<u64> = Allocator::builder()
//!     .bounded(1024)
//!     .build();
//!
//! // Create slots like Box::new()
//! let slot = orders.new_slot(42);
//! assert_eq!(*slot, 42);
//!
//! // Slot auto-deallocates on drop
//! drop(slot);
//! assert_eq!(orders.len(), 0);
//! ```
//!
//! # Performance
//!
//! All measurements in CPU cycles (see `BENCHMARKS.md` for methodology):
//!
//! | Operation | nexus-slab | slab crate | Notes |
//! |-----------|------------|------------|-------|
//! | GET p50 | **2** | 3 | Direct pointer, no lookup |
//! | GET_MUT p50 | **2** | 3 | Direct pointer |
//! | INSERT p50 | **4** | 4 | No TLS overhead |
//! | REMOVE p50 | **3** | 3 | No TLS overhead |
//! | REPLACE p50 | **2** | 4 | Direct pointer, no lookup |
//!
//! # Bounded vs Unbounded
//!
//! ```
//! use nexus_slab::Allocator;
//!
//! // Bounded: fixed capacity, returns None when full
//! let bounded: Allocator<u64> = Allocator::builder()
//!     .bounded(100)
//!     .build();
//!
//! // Unbounded: grows by adding chunks (no copying)
//! let unbounded: Allocator<u64> = Allocator::builder()
//!     .unbounded()
//!     .chunk_capacity(4096)
//!     .capacity(10_000)  // pre-allocate
//!     .build();
//! ```
//!
//! # Key-based Access
//!
//! Leak a slot to get a [`Key`] for external storage:
//!
//! ```
//! use nexus_slab::Allocator;
//!
//! let alloc: Allocator<String> = Allocator::builder().bounded(100).build();
//!
//! let slot = alloc.new_slot("hello".to_string());
//! let key = slot.leak();  // Slot forgotten, data stays alive
//!
//! assert!(alloc.contains_key(key));
//!
//! // Access via key (unsafe - caller ensures validity)
//! let value = unsafe { alloc.get_by_key_unchecked(key) };
//! assert_eq!(value, "hello");
//! ```
//!
//! # Architecture
//!
//! ## Two-Level Freelist
//!
//! ```text
//! slabs_head ─► Slab 2 ─► Slab 0 ─► NONE
//!                 │         │
//!                 ▼         ▼
//!              [slots]   [slots]     Slab 1 (full, not on freelist)
//! ```
//!
//! - **Slab freelist**: Which slabs have available space (O(1) lookup)
//! - **Slot freelist**: Which slots within a slab are free (per-slab, LIFO)
//!
//! ## Slot Design
//!
//! Each `Slot<T>` is 16 bytes: a pointer to the slot cell plus a pointer to the
//! VTable. This enables RAII semantics without any TLS lookups.
//!
//! ## Stamp Encoding
//!
//! Each slot has a `stamp: u64` that encodes state and key:
//!
//! - **Bits 63-32**: State (vacant flag + next_free index)
//! - **Bits 31-0**: Key (valid regardless of state)
//!
//! Freelists are **intra-slab only** - chains never cross slab boundaries.

#![warn(missing_docs)]

pub(crate) mod bounded;
pub(crate) mod shared;
pub(crate) mod unbounded;

// Re-export sentinel for Key::NONE
pub use shared::SLOT_NONE;

// Re-export SlotCell for direct slot access (used by nexus-collections)
pub use shared::SlotCell;

// Re-export VTable for direct vtable access (used by nexus-collections)
pub use shared::VTable;

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use bounded::BoundedSlabInner;
use unbounded::SlabInner;

// =============================================================================
// Key
// =============================================================================

/// Opaque handle to an allocated slot.
///
/// A `Key` is simply an index into the slab. It does not contain a generation
/// counter or any other validation mechanism.
///
/// # Design Rationale: No Generational Indices
///
/// This slab intentionally omits generational indices (ABA protection). Why?
///
/// **The slab is dumb storage, not a source of truth.**
///
/// In real systems, your data has authoritative external identifiers:
/// - Exchange order IDs in trading systems
/// - Database primary keys in web services
/// - Session tokens in connection managers
///
/// When you receive a message referencing an entity, you must validate against
/// the authoritative identifier anyway:
///
/// ```ignore
/// fn on_fill(fill: Fill, key: Key) {
///     let Some(order) = slab.get(key) else { return };
///
///     // This check is REQUIRED regardless of generational indices
///     if order.exchange_id != fill.exchange_id {
///         panic!("order mismatch");
///     }
///
///     // Process...
/// }
/// ```
///
/// Generational indices would catch the same bug that domain validation catches,
/// but at a cost of ~8 cycles per operation. Since domain validation is
/// unavoidable, generations provide no additional safety—only overhead.
///
/// **If a stale key reaches the slab, your architecture has a bug.** The fix is
/// to correct the architecture (clear ownership, proper state machines), not to
/// add runtime checks that mask the underlying problem.
///
/// # Sentinel
///
/// [`Key::NONE`] represents an invalid/absent key, useful for optional key
/// fields without `Option` overhead.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Key(u32);

impl Key {
    /// Sentinel value representing no key / invalid key.
    ///
    /// Equivalent to `SLOT_NONE`. Check with [`is_none`](Self::is_none).
    pub const NONE: Self = Key(SLOT_NONE);

    /// Creates a new key from an index.
    ///
    /// This is primarily for internal use by the allocator.
    #[doc(hidden)]
    #[inline]
    pub const fn new(index: u32) -> Self {
        Key(index)
    }

    /// Returns the slot index.
    ///
    /// For bounded slabs, this is the direct slot index.
    /// For unbounded slabs, this encodes chunk and local index via
    /// power-of-2 arithmetic.
    #[inline]
    pub const fn index(self) -> u32 {
        self.0
    }

    /// Returns `true` if this is the [`Key::NONE`] sentinel.
    #[inline]
    pub const fn is_none(self) -> bool {
        self.0 == SLOT_NONE
    }

    /// Returns `true` if this is a valid key (not [`Key::NONE`]).
    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != SLOT_NONE
    }

    /// Returns the raw `u32` representation.
    ///
    /// Useful for serialization or FFI.
    #[inline]
    pub const fn into_raw(self) -> u32 {
        self.0
    }

    /// Constructs a key from a raw `u32` value.
    ///
    /// No safety invariants—any `u32` is valid. However, using a key not
    /// returned by this slab's `insert` will return `None` or wrong data.
    #[inline]
    pub const fn from_raw(value: u32) -> Self {
        Key(value)
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_none() {
            f.write_str("Key::NONE")
        } else {
            write!(f, "Key({})", self.0)
        }
    }
}

// =============================================================================
// Allocator
// =============================================================================

/// Pre-allocated storage with O(1) insert/remove.
///
/// `Allocator` is the entry point for creating and managing slots. It wraps
/// a leaked VTable that provides the actual storage operations.
///
/// # Thread Safety
///
/// `Allocator` is `!Send` and `!Sync`. Each allocator instance must be used
/// from a single thread. The storage is leaked and lives for the lifetime of
/// the program.
///
/// # Thread Safety
///
/// `Allocator` is `!Send` and `!Sync`. It must only be used from the thread
/// that created it. The underlying storage uses non-atomic operations for
/// performance.
///
/// ```compile_fail
/// use nexus_slab::Allocator;
///
/// fn assert_send<T: Send>() {}
/// assert_send::<Allocator<u64>>(); // Allocator is !Send
/// ```
///
/// ```compile_fail
/// use nexus_slab::Allocator;
///
/// fn assert_sync<T: Sync>() {}
/// assert_sync::<Allocator<u64>>(); // Allocator is !Sync
/// ```
///
/// # Copy
///
/// `Allocator` is `Copy` - it's just a pointer to leaked storage. You can
/// freely pass it around, clone it, etc. (within the same thread).
///
/// # Example
///
/// ```
/// use nexus_slab::Allocator;
///
/// let alloc: Allocator<u64> = Allocator::builder()
///     .bounded(1024)
///     .build();
///
/// let slot = alloc.new_slot(42);
/// assert_eq!(*slot, 42);
/// ```
pub struct Allocator<T: 'static> {
    vtable: &'static shared::VTable<T>,
}

// Manual Copy/Clone to avoid requiring T: Copy/Clone
impl<T: 'static> Clone for Allocator<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Allocator<T> {}

impl<T: 'static> Allocator<T> {
    /// Start building an allocator.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::Allocator;
    ///
    /// // Bounded (fixed capacity)
    /// let alloc: Allocator<u64> = Allocator::builder()
    ///     .bounded(1024)
    ///     .build();
    ///
    /// // Unbounded (growable)
    /// let alloc: Allocator<u64> = Allocator::builder()
    ///     .unbounded()
    ///     .chunk_capacity(4096)
    ///     .build();
    /// ```
    #[inline]
    pub fn builder() -> AllocatorBuilder<T> {
        AllocatorBuilder {
            _marker: PhantomData,
        }
    }

    // =========================================================================
    // Slot creation
    // =========================================================================

    /// Create a new slot with the given value.
    ///
    /// # Panics
    ///
    /// Panics if the allocator is full (bounded only).
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::Allocator;
    ///
    /// let alloc: Allocator<u64> = Allocator::builder().bounded(100).build();
    /// let slot = alloc.new_slot(42);
    /// assert_eq!(*slot, 42);
    /// ```
    #[inline]
    pub fn new_slot(&self, value: T) -> Slot<T> {
        self.try_new_slot(value).expect("allocator full")
    }

    /// Try to create a new slot with the given value.
    ///
    /// Returns `None` if the allocator is full (bounded only).
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_slab::Allocator;
    ///
    /// let alloc: Allocator<u64> = Allocator::builder().bounded(100).build();
    /// let slot = alloc.try_new_slot(42);
    /// assert!(slot.is_some());
    /// ```
    #[inline]
    pub fn try_new_slot(&self, value: T) -> Option<Slot<T>> {
        // SAFETY: VTable is valid (leaked, always initialized)
        let claimed = unsafe { self.vtable.try_claim() }?;

        let slot_ptr = claimed.slot_ptr as *mut SlotCell<T>;
        unsafe {
            // Write value at offset 8 (after stamp)
            let value_ptr = (slot_ptr as *mut u8).add(8) as *mut T;
            std::ptr::write(value_ptr, value);
            // Mark as occupied
            (*slot_ptr).set_key_occupied(claimed.key.index());
        }

        Some(Slot {
            slot: slot_ptr,
            vtable: self.vtable,
        })
    }

    // =========================================================================
    // Allocator state
    // =========================================================================

    /// Returns the number of occupied slots.
    ///
    /// This scans all slots - O(n). Use only for diagnostics, not hot path.
    #[inline]
    pub fn len(&self) -> usize {
        // SAFETY: inner is valid
        unsafe {
            let inner = self.vtable.inner();
            if self.is_bounded() {
                (*(inner as *const BoundedSlabInner<T>)).len() as usize
            } else {
                (*(inner as *const SlabInner<T>)).len()
            }
        }
    }

    /// Returns true if no slots are occupied.
    ///
    /// This scans slots - O(n). Use only for diagnostics, not hot path.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the current capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        // SAFETY: inner is valid
        unsafe {
            let inner = self.vtable.inner();
            if self.is_bounded() {
                (*(inner as *const BoundedSlabInner<T>)).capacity() as usize
            } else {
                (*(inner as *const SlabInner<T>)).capacity()
            }
        }
    }

    /// Returns `true` if this is a bounded allocator.
    #[inline]
    fn is_bounded(self) -> bool {
        // We can tell by checking if the try_claim function matches bounded
        // For now, store this in the vtable or use a different approach
        // Actually, let's add a flag to VTable
        self.vtable.is_bounded()
    }

    /// Returns the VTable for direct access.
    ///
    /// This is useful for advanced use cases where you need to bypass the
    /// normal API (e.g., nexus-collections).
    #[inline]
    pub fn vtable(&self) -> &'static shared::VTable<T> {
        self.vtable
    }

    // =========================================================================
    // Key-based access
    // =========================================================================

    /// Check if a key refers to a valid, occupied slot.
    #[inline]
    pub fn contains_key(&self, key: Key) -> bool {
        // SAFETY: VTable is valid
        unsafe { self.vtable.contains_key(key) }
    }

    /// Get a reference to a value by key.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    ///
    /// # Safety
    ///
    /// Caller must ensure no mutable references to this slot exist.
    #[inline]
    pub unsafe fn get_by_key(&self, key: Key) -> Option<&T> {
        if !self.contains_key(key) {
            return None;
        }
        Some(unsafe { self.get_by_key_unchecked(key) })
    }

    /// Get a mutable reference to a value by key.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    ///
    /// # Safety
    ///
    /// Caller must ensure no other references to this slot exist.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_by_key_mut(&self, key: Key) -> Option<&mut T> {
        if !self.contains_key(key) {
            return None;
        }
        Some(unsafe { self.get_by_key_unchecked_mut(key) })
    }

    /// Get a reference to a value by key without checking validity.
    ///
    /// # Safety
    ///
    /// - Key must refer to an occupied slot
    /// - No mutable references may exist to this slot
    #[inline]
    pub unsafe fn get_by_key_unchecked(&self, key: Key) -> &T {
        let slot_cell = unsafe { self.vtable.slot_ptr(key) };
        unsafe { (*slot_cell).get_value() }
    }

    /// Get a mutable reference to a value by key without checking validity.
    ///
    /// # Safety
    ///
    /// - Key must refer to an occupied slot
    /// - No other references may exist to this slot
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_by_key_unchecked_mut(&self, key: Key) -> &mut T {
        let slot_cell = unsafe { self.vtable.slot_ptr(key) };
        unsafe { (*slot_cell).get_value_mut() }
    }

    /// Try to remove a value by key, returning the value if present.
    ///
    /// Returns `None` if the key is invalid or the slot is vacant.
    ///
    /// # Safety
    ///
    /// Caller must ensure no references to this slot exist.
    #[inline]
    pub unsafe fn try_remove_by_key(&self, key: Key) -> Option<T> {
        if !self.contains_key(key) {
            return None;
        }
        Some(unsafe { self.remove_by_key(key) })
    }

    /// Remove a value by key, returning the value.
    ///
    /// # Safety
    ///
    /// - Key must refer to an occupied slot
    /// - No references may exist to this slot
    /// - The key must not be used after this call
    #[inline]
    pub unsafe fn remove_by_key(&self, key: Key) -> T {
        let slot_cell = unsafe { self.vtable.slot_ptr(key) };

        // Read the value out
        let value = unsafe { std::ptr::read((*slot_cell).value_ptr()) };

        // Free the slot
        unsafe { self.vtable.free(key) };

        value
    }
}

impl<T: 'static> std::fmt::Debug for Allocator<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Allocator")
            .field("len", &self.len())
            .field("capacity", &self.capacity())
            .finish()
    }
}

// =============================================================================
// Builders
// =============================================================================

/// Builder for configuring an allocator.
pub struct AllocatorBuilder<T> {
    _marker: PhantomData<T>,
}

impl<T> AllocatorBuilder<T> {
    /// Configure for a bounded (fixed capacity) allocator.
    #[inline]
    pub fn bounded(self, capacity: usize) -> BoundedBuilder<T> {
        BoundedBuilder {
            capacity,
            _marker: PhantomData,
        }
    }

    /// Configure for an unbounded (growable) allocator.
    #[inline]
    pub fn unbounded(self) -> UnboundedBuilder<T> {
        UnboundedBuilder {
            chunk_capacity: 4096,
            initial_capacity: 0,
            _marker: PhantomData,
        }
    }
}

/// Builder for bounded allocator.
pub struct BoundedBuilder<T> {
    capacity: usize,
    _marker: PhantomData<T>,
}

impl<T: 'static> BoundedBuilder<T> {
    /// Build the allocator.
    ///
    /// The allocator storage is leaked and will live for the lifetime of the
    /// program.
    ///
    /// # Panics
    ///
    /// Panics if capacity is 0 or exceeds maximum.
    pub fn build(self) -> Allocator<T> {
        assert!(self.capacity > 0, "capacity must be non-zero");
        assert!(self.capacity < SLOT_NONE as usize, "capacity exceeds maximum");

        // Create and leak inner
        let inner = Box::leak(Box::new(BoundedSlabInner::<T>::with_capacity(
            self.capacity as u32,
        )));

        // Create vtable pointing to inner
        let mut vtable = BoundedSlabInner::<T>::vtable();
        let inner_ptr = inner as *mut BoundedSlabInner<T> as *mut ();
        // SAFETY: inner is leaked, address is stable
        unsafe { vtable.set_inner(inner_ptr) };
        vtable.set_bounded(true);

        // Leak vtable
        let vtable: &'static shared::VTable<T> = Box::leak(Box::new(vtable));

        Allocator { vtable }
    }
}

/// Builder for unbounded allocator.
pub struct UnboundedBuilder<T> {
    chunk_capacity: usize,
    initial_capacity: usize,
    _marker: PhantomData<T>,
}

impl<T: 'static> UnboundedBuilder<T> {
    /// Set chunk capacity (default: 4096, rounded to power of 2).
    #[inline]
    pub fn chunk_capacity(mut self, cap: usize) -> Self {
        self.chunk_capacity = cap;
        self
    }

    /// Pre-allocate space for this many items (default: 0).
    #[inline]
    pub fn capacity(mut self, cap: usize) -> Self {
        self.initial_capacity = cap;
        self
    }

    /// Build the allocator.
    ///
    /// The allocator storage is leaked and will live for the lifetime of the
    /// program.
    pub fn build(self) -> Allocator<T> {
        // Create and leak inner
        let inner = Box::leak(Box::new(SlabInner::<T>::with_chunk_capacity(
            self.chunk_capacity,
        )));

        // Pre-allocate if requested
        if self.initial_capacity > 0 {
            while inner.capacity() < self.initial_capacity {
                inner.grow();
            }
        }

        // Create vtable pointing to inner
        let mut vtable = SlabInner::<T>::vtable();
        let inner_ptr = inner as *mut SlabInner<T> as *mut ();
        // SAFETY: inner is leaked, address is stable
        unsafe { vtable.set_inner(inner_ptr) };
        vtable.set_bounded(false);

        // Leak vtable
        let vtable: &'static shared::VTable<T> = Box::leak(Box::new(vtable));

        Allocator { vtable }
    }
}

// =============================================================================
// Slot
// =============================================================================

/// RAII handle to an occupied slot.
///
/// `Slot<T>` is analogous to `Box<T>`: it owns the value and deallocates on drop.
/// The difference is that `Slot` allocates from a pre-allocated slab, not the heap.
///
/// # Thread Safety
///
/// `Slot` is `!Send` and `!Sync`. It must only be used from the thread that
/// created it (via the parent `Allocator`).
///
/// ```compile_fail
/// use nexus_slab::{Allocator, Slot};
///
/// fn assert_send<T: Send>() {}
/// assert_send::<Slot<u64>>(); // Slot is !Send
/// ```
///
/// ```compile_fail
/// use nexus_slab::{Allocator, Slot};
///
/// fn assert_sync<T: Sync>() {}
/// assert_sync::<Slot<u64>>(); // Slot is !Sync
/// ```
///
/// # Size
///
/// 16 bytes (slot pointer + vtable pointer).
///
/// # Example
///
/// ```
/// use nexus_slab::Allocator;
///
/// let alloc: Allocator<String> = Allocator::builder().bounded(100).build();
///
/// let mut slot = alloc.new_slot("hello".to_string());
/// slot.push_str(" world");
/// assert_eq!(&*slot, "hello world");
///
/// // Slot auto-deallocates on drop
/// ```
#[must_use = "dropping Slot returns it to the allocator"]
pub struct Slot<T: 'static> {
    slot: *mut SlotCell<T>,
    vtable: &'static shared::VTable<T>,
}

impl<T: 'static> Slot<T> {
    /// Returns the key for this slot.
    #[inline]
    pub fn key(&self) -> Key {
        Key::new(self.slot_cell().key_from_stamp())
    }

    /// Returns the raw pointer to the underlying `SlotCell`.
    ///
    /// This pointer remains valid as long as the slot is not freed.
    #[inline]
    pub fn slot_ptr(&self) -> *mut SlotCell<T> {
        self.slot
    }

    /// Returns the VTable for this slot's allocator.
    #[inline]
    pub fn vtable(&self) -> &'static shared::VTable<T> {
        self.vtable
    }

    /// Leaks the slot, keeping the data alive and returning its key.
    ///
    /// After calling `leak()`, the slot remains occupied but has no
    /// Slot owner. Access the data via the allocator's key-based functions.
    #[inline]
    pub fn leak(self) -> Key {
        let key = self.key();
        std::mem::forget(self);
        key
    }

    /// Returns a reference to the value.
    #[inline]
    pub fn get(&self) -> &T {
        // SAFETY: Slot owns the slot. SlotCell is repr(C): [stamp: 8][value: T]
        unsafe { &*((self.slot as *const u8).add(8) as *const T) }
    }

    /// Returns a mutable reference to the value.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        // SAFETY: Slot owns the slot, &mut ensures exclusivity.
        unsafe { &mut *((self.slot as *mut u8).add(8) as *mut T) }
    }

    /// Returns a raw pointer to the value.
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        unsafe { (self.slot as *const u8).add(8) as *const T }
    }

    /// Returns a mutable raw pointer to the value.
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        unsafe { (self.slot as *mut u8).add(8) as *mut T }
    }

    /// Returns `true` if the slot is still occupied.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.slot_cell().is_occupied()
    }

    /// Replaces the value, returning the old one.
    #[inline]
    pub fn replace(&mut self, value: T) -> T {
        let value_ptr = unsafe { (self.slot as *mut u8).add(8) as *mut T };
        let old = unsafe { value_ptr.read() };
        unsafe { value_ptr.write(value) };
        old
    }

    /// Consumes the slot, returning the value and deallocating.
    #[inline]
    pub fn into_inner(self) -> T {
        let key = self.key();

        // SAFETY: Slot owns the slot
        let value = unsafe {
            let value_ptr = (self.slot as *const u8).add(8) as *const T;
            std::ptr::read(value_ptr)
        };

        // Free the slot
        // SAFETY: VTable is valid, slot was occupied
        unsafe { self.vtable.free(key) };

        std::mem::forget(self);
        value
    }

    #[inline]
    fn slot_cell(&self) -> &SlotCell<T> {
        // SAFETY: Slot holds a valid slot pointer
        unsafe { &*self.slot }
    }
}

impl<T: 'static> Drop for Slot<T> {
    fn drop(&mut self) {
        let key = self.key();

        // Drop the value
        unsafe {
            let value_ptr = (self.slot as *mut u8).add(8) as *mut T;
            std::ptr::drop_in_place(value_ptr);
        }

        // Free the slot via VTable - no TLS lookup!
        // SAFETY: VTable is valid
        unsafe { self.vtable.free(key) };
    }
}

impl<T: 'static> Deref for Slot<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T: 'static> DerefMut for Slot<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

impl<T: 'static + std::fmt::Debug> std::fmt::Debug for Slot<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slot")
            .field("key", &self.key())
            .field("value", self.get())
            .finish()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Key tests
    // =========================================================================

    #[test]
    fn key_new_and_index() {
        let key = Key::new(12345);
        assert_eq!(key.index(), 12345);
    }

    #[test]
    fn key_zero_index() {
        let key = Key::new(0);
        assert_eq!(key.index(), 0);
        assert!(key.is_some());
    }

    #[test]
    fn key_max_valid_index() {
        // Max valid index is SLOT_NONE - 1
        let key = Key::new(SLOT_NONE - 1);
        assert_eq!(key.index(), SLOT_NONE - 1);
        assert!(key.is_some());
    }

    #[test]
    fn key_none_sentinel() {
        assert!(Key::NONE.is_none());
        assert!(!Key::NONE.is_some());
        assert_eq!(Key::NONE.index(), SLOT_NONE);
    }

    #[test]
    fn key_valid_is_some() {
        let key = Key::new(42);
        assert!(key.is_some());
        assert!(!key.is_none());
    }

    #[test]
    fn key_raw_roundtrip() {
        let key = Key::new(999);
        let raw = key.into_raw();
        let restored = Key::from_raw(raw);
        assert_eq!(key, restored);
        assert_eq!(restored.index(), 999);
    }

    #[test]
    fn key_none_raw_roundtrip() {
        let raw = Key::NONE.into_raw();
        assert_eq!(raw, SLOT_NONE);
        let restored = Key::from_raw(raw);
        assert!(restored.is_none());
    }

    #[test]
    fn key_debug_format() {
        let key = Key::new(42);
        let debug = format!("{:?}", key);
        assert_eq!(debug, "Key(42)");

        let none_debug = format!("{:?}", Key::NONE);
        assert_eq!(none_debug, "Key::NONE");
    }

    #[test]
    fn key_equality() {
        let a = Key::new(100);
        let b = Key::new(100);
        let c = Key::new(200);

        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(Key::NONE, Key::NONE);
    }

    #[test]
    fn key_size() {
        assert_eq!(std::mem::size_of::<Key>(), 4);
    }

    // =========================================================================
    // Allocator tests
    // =========================================================================

    #[test]
    fn allocator_is_copy() {
        let alloc: Allocator<u64> = Allocator::builder().bounded(10).build();
        let alloc2 = alloc; // Copy
        let alloc3 = alloc; // Copy again
        assert_eq!(alloc2.capacity(), alloc3.capacity());
    }

    #[test]
    fn bounded_basic() {
        let alloc: Allocator<u64> = Allocator::builder().bounded(100).build();

        assert_eq!(alloc.len(), 0);
        assert_eq!(alloc.capacity(), 100);
        assert!(alloc.is_empty());

        let slot = alloc.new_slot(42);
        assert_eq!(*slot, 42);
        assert_eq!(alloc.len(), 1);

        drop(slot);
        assert_eq!(alloc.len(), 0);
    }

    #[test]
    fn unbounded_basic() {
        let alloc: Allocator<u64> = Allocator::builder()
            .unbounded()
            .chunk_capacity(16)
            .build();

        assert_eq!(alloc.len(), 0);

        let slot = alloc.new_slot(42);
        assert_eq!(*slot, 42);
        assert_eq!(alloc.len(), 1);

        drop(slot);
        assert_eq!(alloc.len(), 0);
    }

    #[test]
    fn slot_deref() {
        let alloc: Allocator<String> = Allocator::builder().bounded(10).build();

        let mut slot = alloc.new_slot("hello".to_string());
        assert_eq!(&*slot, "hello");

        slot.push_str(" world");
        assert_eq!(&*slot, "hello world");
    }

    #[test]
    fn slot_key_and_leak() {
        let alloc: Allocator<u64> = Allocator::builder().bounded(10).build();

        let slot = alloc.new_slot(42);
        let key = slot.key();
        assert!(key.is_some());

        let leaked_key = slot.leak();
        assert_eq!(key, leaked_key);
        assert_eq!(alloc.len(), 1); // Still occupied

        // Clean up
        unsafe { alloc.remove_by_key(leaked_key) };
        assert_eq!(alloc.len(), 0);
    }

    #[test]
    fn slot_into_inner() {
        let alloc: Allocator<String> = Allocator::builder().bounded(10).build();

        let slot = alloc.new_slot("hello".to_string());
        assert_eq!(alloc.len(), 1);

        let value = slot.into_inner();
        assert_eq!(value, "hello");
        assert_eq!(alloc.len(), 0);
    }

    #[test]
    fn slot_replace() {
        let alloc: Allocator<u64> = Allocator::builder().bounded(10).build();

        let mut slot = alloc.new_slot(42);
        let old = slot.replace(100);
        assert_eq!(old, 42);
        assert_eq!(*slot, 100);
    }

    #[test]
    fn key_based_access() {
        let alloc: Allocator<u64> = Allocator::builder().bounded(10).build();

        let slot = alloc.new_slot(42);
        let key = slot.leak();

        assert!(alloc.contains_key(key));
        assert_eq!(unsafe { alloc.get_by_key(key) }, Some(&42));
        assert_eq!(unsafe { *alloc.get_by_key_unchecked(key) }, 42);

        let value = unsafe { alloc.remove_by_key(key) };
        assert_eq!(value, 42);
        assert!(!alloc.contains_key(key));
    }

    #[test]
    fn slot_size() {
        assert_eq!(std::mem::size_of::<Slot<u64>>(), 16);
    }
}
