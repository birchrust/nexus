//! High-performance unique ID generators for low-latency systems.
//!
//! # Overview
//!
//! `nexus-id` provides time-ordered, extractable unique ID generation based on
//! the Snowflake algorithm, optimized for trading systems and other latency-sensitive
//! applications.
//!
//! | Generator | Speed | Time-ordered | Extractable | Hash Quality |
//! |-----------|-------|--------------|-------------|--------------|
//! | [`Snowflake64`] | ~26-28 cycles | Yes | Yes | Poor* |
//! | [`Snowflake32`] | ~26-28 cycles | Yes | Yes | Poor* |
//!
//! *Snowflake IDs have clustered bit patterns. Use [`FxHash`] or [`AHash`] for
//! HashMap keys, not identity hashers. A future `mixed()` variant will provide
//! good distribution for identity hasher compatibility.
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
//! fn main() {
//!     let epoch = Instant::now();
//!     let mut generator = ClOrdId::new(5, epoch);  // worker 5
//!
//!     let id: u64 = generator.next(Instant::now()).unwrap();
//!
//!     // Can extract components
//!     let (ts, worker, seq) = ClOrdId::unpack(id);
//!     assert_eq!(worker, 5);
//! }
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

mod snowflake;

pub use snowflake::{
    IdInt, SequenceExhausted, Snowflake, Snowflake32, Snowflake64, SnowflakeSigned32,
    SnowflakeSigned64,
};
