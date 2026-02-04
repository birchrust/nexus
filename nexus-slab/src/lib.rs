//! # nexus-slab
//!
//! Thread-local slab allocators for **stable memory addresses** without heap
//! allocation overhead.
//!
//! # What Is This?
//!
//! `nexus-slab` provides **macro-generated, thread-local slab allocators** with
//! 8-byte RAII slot handles. Each allocator is a ZST backed by `thread_local!`
//! storage — no runtime dispatch, no heap allocation on the hot path.
//!
//! Use this when you need:
//! - **Stable memory addresses** — pointers remain valid until explicitly freed
//! - **Box-like semantics without Box** — RAII ownership with pre-allocated storage
//! - **Predictable tail latency** — no reallocation spikes, no allocator contention
//! - **8-byte handles** — half the size of `Box`, key-based access for external storage
//!
//! If you need a general-purpose slab data structure (insert, get by key, iterate),
//! use the [`slab`](https://crates.io/crates/slab) crate instead.
//!
//! # Quick Start
//!
//! ```ignore
//! mod order_alloc {
//!     nexus_slab::bounded_allocator!(super::Order);
//! }
//!
//! // Initialize once per thread
//! order_alloc::Allocator::builder().capacity(10_000).build()?;
//!
//! // 8-byte RAII slot — drops automatically, returns to freelist
//! let slot = order_alloc::Slot::new(Order { id: 1, price: 100.0 });
//! assert_eq!(slot.id, 1); // Deref to &Order
//!
//! // Leak for key-based access
//! let key = slot.leak();
//! let order = unsafe { order_alloc::Slot::from_key(key) };
//! ```
//!
//! # Bounded vs Unbounded
//!
//! - **[`bounded_allocator!`]**: Fixed capacity, returns `Err(Full)` when full.
//!   ~20-24 cycle operations, zero allocation after init.
//! - **[`unbounded_allocator!`]**: Grows via independent chunks (no copying).
//!   ~40 cycle p999 during growth.
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
//! # The [`Alloc`] Trait
//!
//! All macro-generated allocators implement [`Alloc`], enabling generic code
//! over any slab allocator:
//!
//! ```ignore
//! fn process<A: nexus_slab::Alloc<Item = Order>>(slot: nexus_slab::alloc::Slot<A>) {
//!     // Works with any bounded or unbounded allocator for Order
//! }
//! ```
//!
//! # Architecture
//!
//! ## Two-Level Freelist (Unbounded)
//!
//! ```text
//! slabs_head ─► Slab 2 ─► Slab 0 ─► NONE
//!                 │         │
//!                 ▼         ▼
//!              [slots]   [slots]     Slab 1 (full, not on freelist)
//! ```
//!
//! ## Slot State (SLUB-style union)
//!
//! Each slot is a `repr(C)` union — either a freelist pointer or a value:
//!
//! - **Occupied**: `value` field is active — contains the user's `T`
//! - **Vacant**: `next_free` field is active — points to next free slot (or null)
//!
//! Writing a value implicitly transitions the slot from vacant to occupied
//! (overwrites the freelist pointer). Writing a freelist link transitions it
//! back. There is no tag, no sentinel — the Slot RAII handle is the proof
//! of occupancy. Zero bookkeeping on the hot path.
//!
//! Freelists are **intra-slab only** — chains never cross slab boundaries.

#![warn(missing_docs)]

pub mod alloc;
#[doc(hidden)]
pub mod bounded;
#[doc(hidden)]
pub mod macros;
pub(crate) mod shared;
#[doc(hidden)]
pub mod unbounded;

// Re-export trait + markers + error
pub use alloc::{Alloc, BoundedAlloc, Full, UnboundedAlloc};

// Re-export sentinel for Key::NONE
pub use shared::SLOT_NONE;

// Re-export SlotCell for direct slot access (used by nexus-collections and macros)
pub use shared::SlotCell;

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
}
