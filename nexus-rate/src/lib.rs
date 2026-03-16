#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! Rate limiting and flow control primitives for real-time systems.
//!
//! Three algorithms, two threading models:
//!
//! | Algorithm | `local` (`&mut self`) | `sync` (`&self`, atomic) |
//! |-----------|----------------------|--------------------------|
//! | GCRA | [`local::Gcra`] | [`sync::Gcra`] |
//! | Token Bucket | [`local::TokenBucket`] | [`sync::TokenBucket`] |
//! | Sliding Window | [`local::SlidingWindow`] | — |
//!
//! All types share a uniform `try_acquire(cost, now) -> bool` API with
//! weighted request support.
//!
//! `no_std` compatible. GCRA and TokenBucket require no allocation.
//! SlidingWindow requires the `alloc` feature.

#[cfg(feature = "alloc")]
extern crate alloc;

mod error;

pub use error::ConfigError;

/// Single-threaded rate limiters (`&mut self`).
pub mod local;

/// Thread-safe rate limiters (`&self`, using atomics).
pub mod sync;
