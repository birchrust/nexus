//! Thread-safe rate limiters using atomics.
//!
//! All methods take `&self`. Hot-path methods (`try_acquire`, `available`,
//! `release`) use CAS loops. Control-plane methods (`reconfigure`, `reset`)
//! use atomic stores.

mod gcra;
mod token_bucket;

pub use gcra::{Gcra, GcraBuilder};
pub use token_bucket::{TokenBucket, TokenBucketBuilder};
