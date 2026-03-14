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
//! - **8-byte handles** — half the size of `Box`
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
//! let slot = order_alloc::BoxSlot::try_new(Order { id: 1, price: 100.0 })?;
//! assert_eq!(slot.id, 1); // Deref to &Order
//!
//! // Leak for permanent storage
//! let leaked: LocalStatic<Order> = slot.leak();
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
//! fn process<A: nexus_slab::Alloc<Item = Order>>(slot: nexus_slab::alloc::BoxSlot<Order, A>) {
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
pub mod bounded;
pub mod byte;
#[doc(hidden)]
pub mod macros;
#[doc(hidden)]
pub mod shared;
pub mod unbounded;

// Re-export trait + markers + error + LocalStatic + BoxSlot + RcSlot + WeakSlot
pub use alloc::{
    Alloc, BoundedAlloc, BoxSlot, Full, LocalStatic, RcSlot, UnboundedAlloc, WeakSlot,
};

// Re-export byte slab types
pub use byte::{AlignedBytes, BoundedByteAlloc, UnboundedByteAlloc};

// Re-export raw slot handle from shared
pub use shared::RawSlot;

// Re-export SlotCell for direct slot access (used by nexus-collections and macros)
pub use shared::SlotCell;

// Re-export RcInner for macro expansion (bounded_rc_allocator! / unbounded_rc_allocator!)
pub use shared::RcInner;
