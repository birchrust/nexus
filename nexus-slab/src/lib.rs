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
//! Think of `Slot<T>` as analogous to `Box<T>`: an owning handle that provides access
//! to a value and deallocates on drop. The difference is that `Box` allocates from the
//! heap on every call, while `Slot` allocates from a pre-allocated slab—O(1) with no
//! syscalls.
//!
//! # Quick Start
//!
//! Use [`create_allocator!`] to create a type-safe allocator with 8-byte RAII slots:
//!
//! ```
//! use nexus_slab::create_allocator;
//!
//! // Define an allocator for your type
//! create_allocator!(order_alloc, u64);
//!
//! // Initialize at startup (bounded = fixed capacity)
//! order_alloc::init().bounded(1024).build();
//!
//! // Insert returns an RAII Slot (8 bytes)
//! let slot = order_alloc::insert(42);
//! assert_eq!(*slot, 42);
//!
//! // Slot auto-deallocates on drop
//! drop(slot);
//! assert_eq!(order_alloc::len(), 0);
//! ```
//!
//! # Performance
//!
//! All measurements in CPU cycles (see `BENCHMARKS.md` for methodology):
//!
//! | Operation | Slot API | slab crate | Notes |
//! |-----------|----------|------------|-------|
//! | GET p50 | **2** | 3 | Direct pointer, no lookup |
//! | GET_MUT p50 | **2** | 3 | Direct pointer |
//! | INSERT p50 | 8 | **4** | +4 cycles TLS overhead |
//! | REMOVE p50 | 4 | **3** | TLS overhead |
//! | REPLACE p50 | **2** | 4 | Direct pointer, no lookup |
//!
//! The TLS lookup adds ~4 cycles to INSERT/REMOVE, but access operations have zero
//! overhead because `Slot` caches the pointer. For access-heavy workloads, this is
//! a net win. Full lifecycle cost is +1 cycle vs direct API.
//!
//! # Bounded vs Unbounded
//!
//! ```
//! use nexus_slab::create_allocator;
//!
//! create_allocator!(bounded_alloc, u64);
//! create_allocator!(unbounded_alloc, u64);
//!
//! // Bounded: fixed capacity, returns None when full
//! bounded_alloc::init().bounded(100).build();
//!
//! // Unbounded: grows by adding chunks (no copying)
//! unbounded_alloc::init()
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
//! use nexus_slab::create_allocator;
//!
//! create_allocator!(my_alloc, String);
//! my_alloc::init().bounded(100).build();
//!
//! let slot = my_alloc::insert("hello".to_string());
//! let key = slot.leak();  // Slot forgotten, data stays alive
//!
//! assert!(my_alloc::contains_key(key));
//!
//! // Access via key (unsafe - caller ensures validity)
//! let value = unsafe { my_alloc::get_unchecked(key) };
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
//! Each slot is 8 bytes (single pointer). The VTable for slab operations
//! is stored in thread-local storage, enabling:
//!
//! - Minimal slot size (cache-friendly)
//! - RAII semantics (drop returns slot to freelist)
//! - Type safety via macro-generated modules
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

#[doc(hidden)]
pub mod bounded;
#[doc(hidden)]
pub mod shared;
#[doc(hidden)]
pub mod unbounded;

#[doc(hidden)]
pub mod macros;

// Note: create_allocator! is automatically exported at crate root via #[macro_export]

// Re-export sentinel for Key::NONE
pub use shared::SLOT_NONE;

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
    /// This is primarily for internal use by the allocator macro.
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
