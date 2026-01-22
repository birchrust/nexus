//! High-performance unique ID generators for low-latency systems.
//!
//! # Overview
//!
//! `nexus-id` provides unique ID generation optimized for trading systems and
//! other latency-sensitive applications. All generators avoid syscalls on the
//! hot path and produce stack-allocated output.
//!
//! | Generator | Speed | Time-ordered | Output | Use Case |
//! |-----------|-------|--------------|--------|----------|
//! | [`Snowflake64`] | ~26 cycles | Yes | `u64` | Numeric IDs with extraction |
//! | [`Snowflake32`] | ~26 cycles | Yes | `u32` | Compact numeric IDs |
//! | [`UuidV4`] | ~35 cycles | No | [`Uuid`] | Random unique IDs |
//! | [`UuidV7`] | ~45 cycles | Yes | [`Uuid`] | Time-ordered UUIDs |
//! | [`UlidGenerator`] | ~45 cycles | Yes | [`Ulid`] | Sortable 26-char IDs |
//!
//! # ID Encoding Types
//!
//! These types encode integer values into various string formats:
//!
//! | Type | Method | Description |
//! |------|--------|-------------|
//! | [`HexId64`] | `HexId64::encode(u64)` | 16-char lowercase hex |
//! | [`Base62Id`] | `Base62Id::encode(u64)` | 11-char alphanumeric (0-9, A-Z, a-z) |
//! | [`Base36Id`] | `Base36Id::encode(u64)` | 13-char case-insensitive (0-9, a-z) |
//!
//! All types support `decode()` to recover the original value.
//!
//! # Why Snowflake?
//!
//! Snowflake embeds a timestamp in every ID, which provides critical guarantees:
//!
//! - **Burst safety:** Sequence exhaustion returns an error rather than silent collision
//! - **Restart safety:** New run = new timestamp = no collision with previous IDs
//! - **Flow control:** Natural backpressure when generating too fast
//!
//! These properties are load-bearing for trading systems where a duplicate ClOrdID
//! means rejected orders and potential position errors.
//!
//! # Usage
//!
//! ```rust
//! use std::time::Instant;
//! use nexus_id::Snowflake64;
//!
//! // Layout: 42 bits timestamp, 6 bits worker, 16 bits sequence
//! // Supports 65,536 IDs per millisecond per worker
//! type ClOrdId = Snowflake64<42, 6, 16>;
//!
//! let epoch = Instant::now();
//! let mut generator = ClOrdId::new(5, epoch);  // worker 5
//!
//! let id: u64 = generator.next(Instant::now()).unwrap();
//!
//! // Can extract components
//! let (ts, worker, seq) = ClOrdId::unpack(id);
//! assert_eq!(worker, 5);
//! ```
//!
//! # Bit Layout Selection
//!
//! Choose your `<TS, W, S>` parameters based on your requirements:
//!
//! | Layout | Timestamp | Workers | Seq/ms | Use Case |
//! |--------|-----------|---------|--------|----------|
//! | `<42, 6, 16>` | 139 years | 64 | 65,536 | High-throughput trading |
//! | `<41, 10, 12>` | 69 years | 1,024 | 4,096 | Twitter-style (many workers) |
//! | `<42, 10, 12>` | 139 years | 1,024 | 4,096 | Balanced |
//!
//! # HashMap Usage
//!
//! Snowflake IDs have poor bit distribution for power-of-2 hash tables.
//! Always use a real hasher:
//!
//! ```rust, ignore
//! use std::collections::HashMap;
//! use rustc_hash::FxHashMap;  // or ahash::AHashMap
//!
//! // ✗ Bad: identity hasher will have pathological collisions
//! // let map: HashMap<u64, Order, nohash::BuildNoHashHasher<u64>> = ...;
//!
//! // ✓ Good: FxHash mixes the bits properly
//! let map: FxHashMap<u64, Order> = FxHashMap::default();
//! ```
//!
//! # Error Handling
//!
//! [`Snowflake64::next`] returns `Err(SequenceExhausted)` if you exceed the
//! per-millisecond sequence limit. This is intentional flow control - it
//! prevents generating IDs faster than the timestamp can distinguish them.
//!
//! ```rust
//! use nexus_id::{Snowflake64, SequenceExhausted};
//! # use std::time::Instant;
//!
//! type Id = Snowflake64<42, 6, 16>;
//!
//! fn generate_id(generator: &mut Id, now: Instant) -> u64 {
//!     match generator.next(now) {
//!         Ok(id) => id,
//!         Err(SequenceExhausted { .. }) => {
//!             // Options: wait for next ms, use different worker, or propagate
//!             panic!("ID generation rate exceeded");
//!         }
//!     }
//! }
//! ```

pub(crate) mod encode;
mod prng;
mod snowflake;
mod types;
pub mod ulid;
pub mod uuid;

pub use snowflake::{
    IdInt, SequenceExhausted, Snowflake, Snowflake32, Snowflake64, SnowflakeSigned32,
    SnowflakeSigned64,
};
pub use types::{Base36Id, Base62Id, HexId64, Ulid, Uuid, UuidCompact};
pub use ulid::UlidGenerator;
pub use uuid::{UuidV4, UuidV7};
