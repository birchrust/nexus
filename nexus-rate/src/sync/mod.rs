//! Thread-safe rate limiters using atomics.
//!
//! All types use `&self` — safe to share across threads via `Arc` or
//! static references. Uses CAS loops for lock-free operation.

mod gcra;
mod token_bucket;

pub use gcra::{Gcra, GcraBuilder};
pub use token_bucket::{TokenBucket, TokenBucketBuilder};
