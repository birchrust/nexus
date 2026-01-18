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
//! Benchmarked against the `slab` crate (the standard ecosystem choice):
//!
//! | Operation | BoundedSlab | slab crate | Notes |
//! |-----------|-------------|------------|-------|
//! | INSERT p50 | ~20 cycles | ~22 cycles | 2 cycles faster |
//! | GET p50 | ~24 cycles | ~26 cycles | 2 cycles faster |
//! | REMOVE p50 | ~24 cycles | ~30 cycles | 6 cycles faster |
//!
//! For growable `Slab`, steady-state performance matches `slab` crate, but
//! growth tail latency is **20-50x better** (p999: ~40 cycles vs ~2000+ cycles).
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
//! BoundedSlab (single contiguous allocation):
//! ┌─────────────────────────────────────────────┐
//! │ Slot 0: [tag: u32][value: T]                │
//! │ Slot 1: [tag: u32][value: T]                │
//! │ ...                                         │
//! │ Slot N: [tag: u32][value: T]                │
//! └─────────────────────────────────────────────┘
//!
//! Slab (multiple independent chunks):
//! ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
//! │ Chunk 0      │  │ Chunk 1      │  │ Chunk 2      │
//! │ (BoundedSlab)│  │ (BoundedSlab)│  │ (BoundedSlab)│
//! └──────────────┘  └──────────────┘  └──────────────┘
//!        ▲                                   ▲
//!        └─── head_with_space ───────────────┘
//!              (freelist of non-full chunks)
//! ```
//!
//! ## Slot Tag Encoding
//!
//! Each slot has a `tag: u32` that indicates state:
//!
//! - **Occupied**: `tag == 0` (value is valid)
//! - **Vacant**: `tag` has bit 31 set, bits 0-30 encode next free slot index
//!
//! This enables a single comparison (`tag == 0`) to check occupancy.
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
//! use nexus_slab::Slab;
//!
//! let mut slab = Slab::with_capacity(1000);
//!
//! let key = slab.insert(42);
//! assert_eq!(slab[key], 42);
//!
//! let value = slab.remove(key);
//! assert_eq!(value, 42);
//! ```
//!
//! # Choosing Between BoundedSlab and Slab
//!
//! - **[`BoundedSlab`]**: Fixed capacity, pre-allocated. Returns `Err(Full)` when
//!   exhausted. Use when capacity is known and you want zero allocation after init.
//!   This is the production choice for latency-critical systems.
//!
//! - **[`Slab`]**: Grows by adding new chunks. Use when capacity is unbounded
//!   or as an overflow safety net. Growth allocates one chunk at a time—no
//!   copying of existing data.

#![warn(missing_docs)]

pub mod bounded;
pub mod unbounded;

pub use bounded::BoundedSlab;
pub use unbounded::Slab;

// =============================================================================
// Constants
// =============================================================================

/// Bit 31 of tag: set indicates slot is vacant.
const VACANT_BIT: u32 = 1 << 31; // 0x8000_0000

/// Mask for next_free index (bits 0-30).
const INDEX_MASK: u32 = (1 << 31) - 1; // 0x7FFF_FFFF

/// Sentinel value indicating end of freelist chain or invalid key.
///
/// Max 31-bit value, limiting addressable slots to ~2 billion.
pub const SLOT_NONE: u32 = INDEX_MASK; // 0x7FFF_FFFF

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
    #[inline]
    pub(crate) const fn new(index: u32) -> Self {
        Key(index)
    }

    /// Returns the slot index.
    ///
    /// For [`BoundedSlab`](crate::BoundedSlab), this is the direct slot index.
    /// For [`Slab`](crate::Slab), this encodes slab and local index via
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
// Slot
// =============================================================================

/// Internal slot storage with vacant/occupied state.
///
/// # Tag Encoding (32-bit)
///
/// ```text
/// ┌───┬─────────────────────────────────┐
/// │ V │       next_free (31 bits)       │
/// └───┴─────────────────────────────────┘
/// bit 31              low
/// ```
///
/// - **Bit 31 (V)**: Vacant flag. Set = vacant, clear = occupied.
/// - **Bits 0-30**: When vacant, the next free slot index (or [`SLOT_NONE`]).
///
/// When occupied, the entire tag is zero. This enables a single comparison
/// to validate occupancy.
#[repr(C)]
pub(crate) struct Slot<T> {
    tag: u32,
    pub(crate) value: std::mem::MaybeUninit<T>,
}

impl<T> Slot<T> {
    /// Creates a new vacant slot pointing to `next_free`.
    #[inline]
    pub(crate) const fn new_vacant(next_free: u32) -> Self {
        Self {
            tag: VACANT_BIT | (next_free & INDEX_MASK),
            value: std::mem::MaybeUninit::uninit(),
        }
    }

    /// Returns `true` if this slot contains a value.
    #[inline]
    pub(crate) const fn is_occupied(&self) -> bool {
        self.tag == 0
    }

    /// Returns the next free slot index.
    ///
    /// # Safety
    ///
    /// Only valid when slot is vacant. Debug-asserts this invariant.
    #[inline]
    pub(crate) const fn next_free(&self) -> u32 {
        debug_assert!(self.tag != 0, "next_free called on occupied slot");
        self.tag & INDEX_MASK
    }

    /// Marks this slot as occupied.
    #[inline]
    pub(crate) fn set_occupied(&mut self) {
        self.tag = 0;
    }

    /// Marks this slot as vacant, pointing to the next free slot.
    #[inline]
    pub(crate) fn set_vacant(&mut self, next_free: u32) {
        self.tag = VACANT_BIT | (next_free & INDEX_MASK);
    }
}

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
    // Slot tests
    // =========================================================================

    #[test]
    fn slot_new_vacant() {
        let slot: Slot<u64> = Slot::new_vacant(42);
        assert!(!slot.is_occupied());
        assert_eq!(slot.next_free(), 42);
    }

    #[test]
    fn slot_new_vacant_with_none() {
        let slot: Slot<u64> = Slot::new_vacant(SLOT_NONE);
        assert!(!slot.is_occupied());
        assert_eq!(slot.next_free(), SLOT_NONE);
    }

    #[test]
    fn slot_set_occupied() {
        let mut slot: Slot<u64> = Slot::new_vacant(42);
        assert!(!slot.is_occupied());

        slot.set_occupied();
        assert!(slot.is_occupied());
    }

    #[test]
    fn slot_set_vacant() {
        let mut slot: Slot<u64> = Slot::new_vacant(0);
        slot.set_occupied();
        assert!(slot.is_occupied());

        slot.set_vacant(123);
        assert!(!slot.is_occupied());
        assert_eq!(slot.next_free(), 123);
    }

    #[test]
    fn slot_occupied_tag_is_zero() {
        let mut slot: Slot<u64> = Slot::new_vacant(42);
        slot.set_occupied();
        // Occupied slots have tag == 0
        assert_eq!(slot.tag, 0);
    }

    #[test]
    fn slot_vacant_tag_has_bit_set() {
        let slot: Slot<u64> = Slot::new_vacant(42);
        // Vacant slots have VACANT_BIT set
        assert_ne!(slot.tag & VACANT_BIT, 0);
    }

    #[test]
    fn slot_max_next_free() {
        let slot: Slot<u64> = Slot::new_vacant(INDEX_MASK);
        assert!(!slot.is_occupied());
        assert_eq!(slot.next_free(), INDEX_MASK);
    }

    #[test]
    fn slot_none_sentinel_value() {
        // SLOT_NONE equals INDEX_MASK (max 31-bit value)
        assert_eq!(SLOT_NONE, INDEX_MASK);
        assert_eq!(SLOT_NONE, (1 << 31) - 1);
    }

    #[test]
    fn slot_size_u64() {
        // Slot<u64>: 4-byte tag + 8-byte value = 12 bytes, aligned to 16
        assert_eq!(std::mem::size_of::<Slot<u64>>(), 16);
    }

    #[test]
    fn slot_size_u32() {
        // Slot<u32>: 4-byte tag + 4-byte value = 8 bytes (no padding)
        assert_eq!(std::mem::size_of::<Slot<u32>>(), 8);
    }

    #[test]
    fn slot_size_u8() {
        // Slot<u8>: 4-byte tag + 1-byte value = 5 bytes, aligned to 8
        assert_eq!(std::mem::size_of::<Slot<u8>>(), 8);
    }

    #[test]
    fn slot_cycle_occupied_vacant() {
        let mut slot: Slot<u64> = Slot::new_vacant(10);

        // Cycle through states
        for next in [20, 30, 40, SLOT_NONE] {
            assert!(!slot.is_occupied());
            slot.set_occupied();
            assert!(slot.is_occupied());
            slot.set_vacant(next);
            assert!(!slot.is_occupied());
            assert_eq!(slot.next_free(), next);
        }
    }
}
