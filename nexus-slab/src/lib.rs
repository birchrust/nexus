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
//! ## BoundedSlab (fixed capacity)
//!
//! | Operation | BoundedSlab | slab crate | Notes |
//! |-----------|-------------|------------|-------|
//! | INSERT p50 | ~20 cycles | ~22 cycles | 2 cycles faster |
//! | GET p50 | ~24 cycles | ~26 cycles | 2 cycles faster |
//! | REMOVE p50 | ~24 cycles | ~30 cycles | 6 cycles faster |
//!
//! ## Slab (growable)
//!
//! Steady-state p50 matches `slab` crate (~22-32 cycles depending on operation).
//! The win is tail latency during growth:
//!
//! | Metric | Slab | slab crate | Notes |
//! |--------|------|------------|-------|
//! | Growth p999 | ~40 cycles | ~2000+ cycles | 50x better |
//! | Growth max | ~70K cycles | ~1.5M cycles | 20x better |
//!
//! `Slab` adds chunks independently—no copying. `slab` crate uses `Vec`,
//! which copies all existing data on reallocation.
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
//! - **Vacant**: `tag` has bit 31 set, bits 0-29 encode next free slot index
//! - **Borrowed**: `tag` has bit 30 set (for runtime borrow checking)
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
//! let slab = Slab::with_capacity(1000);
//!
//! // Entry-based API (primary)
//! let entry = slab.insert(42);
//! assert_eq!(*entry.get(), 42);
//! let value = entry.remove();
//! assert_eq!(value, 42);
//!
//! // Key-based API (for collections)
//! let entry = slab.insert(100);
//! let key = entry.key();
//! assert_eq!(slab[key], 100);
//! let value = slab.remove_by_key(key);
//! assert_eq!(value, 100);
//! ```
//!
//! # Choosing Between BoundedSlab and Slab
//!
//! - **[`BoundedSlab`]**: Fixed capacity, pre-allocated. Returns `None` when
//!   exhausted. Use when capacity is known and you want zero allocation after init.
//!   This is the production choice for latency-critical systems.
//!
//! - **[`Slab`]**: Grows by adding new chunks. Use when capacity is unbounded
//!   or as an overflow safety net. Growth allocates one chunk at a time—no
//!   copying of existing data.

#![warn(missing_docs)]

pub mod bounded;
pub mod unbounded;

// Re-export primary types at root
pub use bounded::{BoundedSlab, Entry, Ref, RefMut};
pub use unbounded::{Slab, SlabBuilder};

// =============================================================================
// Constants
// =============================================================================

/// Mask for key index (bits 0-30).
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
}
