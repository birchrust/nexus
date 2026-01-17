//! # nexus-slab
//!
//! A high-performance slab allocator optimized for **predictable tail latency**.
//!
//! # Use Case
//!
//! Designed for latency-critical systems (trading, real-time, game servers) where
//! worst-case performance matters more than average-case throughput. Typical slab
//! allocators using `Vec` exhibit bimodal p999 latency due to reallocation copying;
//! `nexus-slab` provides consistent p999 by using independently-allocated slabs that
//! grow without copying existing data.
//!
//! # Performance Characteristics
//!
//! | Metric | nexus-slab | Typical Vec-based |
//! |--------|------------|-------------------|
//! | p50    | ~24 cycles | ~22 cycles        |
//! | p99    | ~26 cycles | ~24 cycles        |
//! | p999   | ~38-46 cycles (consistent) | 32-3700 cycles (bimodal) |
//! | max    | ~500-800K cycles (growth) | ~1.5-2M cycles (realloc+copy) |
//!
//! Trade a few cycles on median for **predictable** tail latency.
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
//! ## Memory Layout
//!
//! ```text
//! Slab 0                        Slab 1
//! ┌─────────────────────┐       ┌─────────────────────┐
//! │ Slot 0              │       │ Slot 0              │
//! │ ┌─────┬───────────┐ │       │ ┌─────┬───────────┐ │
//! │ │ tag │   value   │ │       │ │ tag │   value   │ │
//! │ └─────┴───────────┘ │       │ └─────┴───────────┘ │
//! │ Slot 1              │       │ Slot 1              │
//! │ ┌─────┬───────────┐ │       │ ...                 │
//! │ │ tag │   value   │ │       └─────────────────────┘
//! │ └─────┴───────────┘ │
//! │ ...                 │       SlabMeta[]
//! └─────────────────────┘       ┌─────────────────────┐
//!                               │ bump_cursor: u32    │
//!                               │ occupied: u32       │
//!                               │ freelist_head: u32  │
//!                               │ next_free_slab: u32 │
//!                               └─────────────────────┘
//! ```
//!
//! ## Slot Tag Encoding
//!
//! Each slot has a `tag: u32` that serves double duty:
//!
//! - **Occupied**: `tag == SLOT_OCCUPIED` (0xFFFF_FFFE), value is valid
//! - **Vacant (end of chain)**: `tag == SLOT_NONE` (0xFFFF_FFFF)
//! - **Vacant (chained)**: `tag < slots_per_slab`, points to next free slot
//!
//! Freelists are **intra-slab only** - chains never cross slab boundaries.
//! This enables slabs to drain independently.
//!
//! ## Allocation Strategy
//!
//! 1. **Check slab freelist head** - O(1) access to a slab with space
//! 2. **Slot freelist first (LIFO)**: Reuse recently-freed slots for cache locality
//! 3. **Bump allocation**: Sequential allocation when slot freelist is empty
//! 4. **Pop exhausted slabs**: Remove from slab freelist when full
//! 5. **Growth**: Allocate new slab when all are full (dynamic mode only)
//!
//! ## Remove: LIFO Cache-Hot Behavior
//!
//! On remove, the freed slot is pushed onto the slab's freelist:
//!
//! ```text
//! Remove slot X from slab S:
//! ┌─────────────────────────────────────────────────────────┐
//! │ 1. Read value from X                                    │
//! │ 2. X.tag ← S.freelist_head (chain to old head)          │
//! │ 3. S.freelist_head ← X (freed slot becomes new head)    │
//! │ 4. If S was full: push S to front of slab freelist      │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! When a full slab gains a free slot, it's pushed to the **front** of the
//! slab freelist (LIFO), so the next insert uses cache-hot memory.
//!
//! ## Growth (Dynamic Mode)
//!
//! When the slab freelist is empty, a new slab is allocated and becomes
//! the freelist head. This cost is amortized over `slots_per_slab` allocations
//! (typically ~16K slots per 256KB slab for 16-byte values).
//!
//! # Example
//!
//! ```
//! use nexus_slab::DynamicSlab;
//!
//! let mut slab = DynamicSlab::with_capacity(1000).unwrap();
//!
//! let key = slab.insert(42);
//! assert_eq!(slab[key], 42);
//!
//! let value = slab.remove(key);
//! assert_eq!(value, 42);
//! ```
//!
//! # Fixed vs Dynamic Mode
//!
//! - **Fixed**: Pre-allocates all memory upfront. Returns `Full` when exhausted.
//!   Use when capacity is known and you want zero allocation after init.
//!
//! - **Dynamic**: Grows by adding new slabs. Use when capacity is unbounded
//!   but growth is infrequent.

#![warn(missing_docs)]

pub mod bounded;
mod sys;
pub mod unbounded;

pub use bounded::BoundedSlab;
pub use unbounded::{
    DynamicSlab, FixedSlab, FixedSlabBuilder, OldKey, Slab, SlabBuilder, SlabError,
};

// =============================================================================
// Constants
// =============================================================================

/// Sentinel value indicating end of freelist chain (31-bit max).
pub(crate) const SLOT_NONE: u32 = (1 << 31) - 1;

/// Bit 31 of tag: set indicates slot is vacant.
const VACANT_BIT: u64 = 1 << 31;

/// Mask for next_free index (bits 0-30).
const NEXT_FREE_MASK: u64 = (1 << 31) - 1;

/// Shift for generation (stored in upper 32 bits).
const GEN_SHIFT: u32 = 32;

// =============================================================================
// Errors
// =============================================================================

/// Returned when inserting into a full fixed-capacity slab.
#[derive(Debug)]
pub struct Full<T>(pub T);

// =============================================================================
// Key
// =============================================================================

/// Opaque handle to an allocated slot.
///
/// Keys are generational: each key encodes both a slot index and a generation
/// counter. When a slot is freed and reallocated, the generation increments,
/// invalidating old keys that point to the same index.
///
/// # Layout
///
/// ```text
/// ┌──────────────────────────┬──────────────────────────┐
/// │   generation (32 bits)   │      index (32 bits)     │
/// └──────────────────────────┴──────────────────────────┘
///           high                        low
/// ```
///
/// # Sentinel
///
/// [`Key::NONE`] (`u64::MAX`) represents an invalid/absent key, useful for
/// optional key fields without the `Option` overhead.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Key(u64);

impl Key {
    const INDEX_BITS: u32 = 32;
    const INDEX_MASK: u64 = (1 << Self::INDEX_BITS) - 1;
    const GEN_SHIFT: u32 = Self::INDEX_BITS;

    /// Sentinel value representing no key.
    ///
    /// Useful for struct fields where `Option<Key>` would add overhead.
    /// Check with [`is_none`](Self::is_none) or [`is_some`](Self::is_some).
    pub const NONE: Self = Key(u64::MAX);

    /// Creates a new key from an index and generation.
    #[inline]
    pub(crate) fn new(index: u32, generation: u32) -> Self {
        Key(((generation as u64) << Self::GEN_SHIFT) | (index as u64))
    }

    /// Returns the slot index component.
    ///
    /// For [`BoundedSlab`](crate::BoundedSlab), this is the slot directly.
    /// For [`Slab`](crate::Slab), this encodes slab and slot via power-of-2 math.
    #[inline]
    pub fn index(self) -> u32 {
        (self.0 & Self::INDEX_MASK) as u32
    }

    /// Returns the generation component.
    ///
    /// Generation increments each time a slot is reused, providing ABA protection.
    #[inline]
    pub fn generation(self) -> u32 {
        (self.0 >> Self::GEN_SHIFT) as u32
    }

    /// Returns `true` if this is the [`Key::NONE`] sentinel.
    #[inline]
    pub fn is_none(self) -> bool {
        self.0 == u64::MAX
    }

    /// Returns `true` if this is a valid key (not [`Key::NONE`]).
    #[inline]
    pub fn is_some(self) -> bool {
        self.0 != u64::MAX
    }

    /// Returns the raw `u64` representation.
    ///
    /// Useful for serialization or FFI.
    #[inline]
    pub const fn into_raw(self) -> u64 {
        self.0
    }

    /// Constructs a key from a raw `u64` value.
    ///
    /// # Safety
    ///
    /// The caller must ensure `value` was produced by [`into_raw`](Self::into_raw)
    /// on a valid key from the same slab, or is [`Key::NONE`]'s raw value.
    #[inline]
    pub const unsafe fn from_raw(value: u64) -> Self {
        Key(value)
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_none() {
            f.write_str("Key::NONE")
        } else {
            f.debug_struct("Key")
                .field("index", &self.index())
                .field("generation", &self.generation())
                .finish()
        }
    }
}

// =============================================================================
// Slot
// =============================================================================

/// Internal slot storage with generation-tagged state.
///
/// # Tag Encoding (64-bit)
///
/// ```text
/// ┌────────────────────────────────┬───────────────────────────────┬───┐
/// │      generation (32 bits)      │    next_free (31 bits)        │ V │
/// └────────────────────────────────┴───────────────────────────────┴───┘
///              high                              low                 bit 31
/// ```
///
/// - **Bit 31 (V)**: Vacant flag. Set = vacant, clear = occupied.
/// - **Bits 0-30**: When vacant, the next free slot index (or [`SLOT_NONE`]).
/// - **Bits 32-63**: Generation counter, preserved across vacant/occupied transitions.
///
/// This encoding stores generation separately from the freelist pointer,
/// so generation survives the vacant state and can be incremented on reallocation.
#[repr(C)]
pub(crate) struct Slot<T> {
    tag: u64,
    pub(crate) value: std::mem::MaybeUninit<T>,
}

impl<T> Slot<T> {
    /// Creates a new vacant slot pointing to `next_free` with generation 0.
    #[inline]
    pub(crate) fn new_vacant(next_free: u32) -> Self {
        Self {
            tag: VACANT_BIT | ((next_free as u64) & NEXT_FREE_MASK),
            value: std::mem::MaybeUninit::uninit(),
        }
    }

    /// Returns `true` if this slot contains a value.
    #[inline]
    pub(crate) fn is_occupied(&self) -> bool {
        self.tag & VACANT_BIT == 0
    }

    /// Returns `true` if this slot is vacant (in freelist).
    #[inline]
    pub(crate) fn is_vacant(&self) -> bool {
        self.tag & VACANT_BIT != 0
    }

    /// Returns the generation counter.
    ///
    /// Valid for both occupied and vacant slots.
    #[inline]
    pub(crate) fn generation(&self) -> u32 {
        (self.tag >> GEN_SHIFT) as u32
    }

    /// Returns the next free slot index.
    ///
    /// # Panics
    ///
    /// Debug-asserts that the slot is vacant.
    #[inline]
    pub(crate) fn next_free(&self) -> u32 {
        debug_assert!(self.is_vacant(), "next_free called on occupied slot");
        (self.tag & NEXT_FREE_MASK) as u32
    }

    /// Marks this slot as occupied with the given generation.
    #[inline]
    pub(crate) fn set_occupied(&mut self, generation: u32) {
        // Clear vacant bit, set generation, clear next_free bits
        self.tag = (generation as u64) << GEN_SHIFT;
    }

    /// Marks this slot as vacant, pointing to the next free slot.
    ///
    /// Preserves the current generation for the next allocation to read and increment.
    #[inline]
    pub(crate) fn set_vacant(&mut self, next_free: u32) {
        let generation = self.generation();
        self.tag =
            ((generation as u64) << GEN_SHIFT) | VACANT_BIT | ((next_free as u64) & NEXT_FREE_MASK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Key tests
    // =========================================================================

    #[test]
    fn key_roundtrip() {
        let key = Key::new(12345, 67890);
        assert_eq!(key.index(), 12345);
        assert_eq!(key.generation(), 67890);
    }

    #[test]
    fn key_max_values() {
        // Max index (but not u32::MAX, that's reserved for NONE)
        let key = Key::new(u32::MAX - 1, u32::MAX);
        assert_eq!(key.index(), u32::MAX - 1);
        assert_eq!(key.generation(), u32::MAX);
        assert!(key.is_some());
    }

    #[test]
    fn key_none_sentinel() {
        assert!(Key::NONE.is_none());
        assert!(!Key::NONE.is_some());

        let valid = Key::new(0, 0);
        assert!(!valid.is_none());
        assert!(valid.is_some());
    }

    #[test]
    fn key_raw_roundtrip() {
        let key = Key::new(999, 888);
        let raw = key.into_raw();
        let restored = unsafe { Key::from_raw(raw) };
        assert_eq!(key, restored);
    }

    #[test]
    fn key_none_raw_roundtrip() {
        let raw = Key::NONE.into_raw();
        let restored = unsafe { Key::from_raw(raw) };
        assert!(restored.is_none());
    }

    #[test]
    fn key_debug_format() {
        let key = Key::new(42, 7);
        let debug = format!("{:?}", key);
        assert!(debug.contains("42"));
        assert!(debug.contains("7"));

        let none_debug = format!("{:?}", Key::NONE);
        assert_eq!(none_debug, "Key::NONE");
    }

    // =========================================================================
    // Slot tests
    // =========================================================================

    #[test]
    fn slot_new_vacant() {
        let slot: Slot<u64> = Slot::new_vacant(42);
        assert!(slot.is_vacant());
        assert!(!slot.is_occupied());
        assert_eq!(slot.next_free(), 42);
        assert_eq!(slot.generation(), 0);
    }

    #[test]
    fn slot_occupied_state() {
        let mut slot: Slot<u64> = Slot::new_vacant(0);

        slot.set_occupied(42);
        assert!(slot.is_occupied());
        assert!(!slot.is_vacant());
        assert_eq!(slot.generation(), 42);
    }

    #[test]
    fn slot_vacant_state() {
        let mut slot: Slot<u64> = Slot::new_vacant(0);
        slot.set_occupied(99); // Set some generation first

        slot.set_vacant(123);
        assert!(slot.is_vacant());
        assert!(!slot.is_occupied());
        assert_eq!(slot.next_free(), 123);
        assert_eq!(slot.generation(), 99); // Generation preserved
    }

    #[test]
    fn slot_generation_preserved_across_vacant() {
        let mut slot: Slot<u64> = Slot::new_vacant(0);

        // Allocate with generation 5
        slot.set_occupied(5);
        assert_eq!(slot.generation(), 5);

        // Free - generation should be preserved
        slot.set_vacant(42);
        assert_eq!(slot.generation(), 5);
        assert_eq!(slot.next_free(), 42);

        // Reallocate with incremented generation
        let new_gen = slot.generation().wrapping_add(1);
        slot.set_occupied(new_gen);
        assert_eq!(slot.generation(), 6);
    }

    #[test]
    fn slot_max_generation() {
        let mut slot: Slot<u64> = Slot::new_vacant(0);

        slot.set_occupied(u32::MAX);
        assert!(slot.is_occupied());
        assert_eq!(slot.generation(), u32::MAX);
    }

    #[test]
    fn slot_max_next_free() {
        let mut slot: Slot<u64> = Slot::new_vacant(SLOT_NONE);
        assert!(slot.is_vacant());
        assert_eq!(slot.next_free(), SLOT_NONE);
    }

    #[test]
    fn slot_none_sentinel() {
        // SLOT_NONE should be max 31-bit value
        assert_eq!(SLOT_NONE, (1 << 31) - 1);

        let slot: Slot<u64> = Slot::new_vacant(SLOT_NONE);
        assert_eq!(slot.next_free(), SLOT_NONE);
    }
}
