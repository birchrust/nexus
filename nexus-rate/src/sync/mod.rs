//! Thread-safe rate limiters using atomics.
//!
//! Hot-path methods (`try_acquire`, `available`) use `&self`. Control-plane
//! methods (`reconfigure`) may require `&mut self`. Uses CAS loops for
//! lock-free operation on hot paths.

mod gcra;
mod token_bucket;

pub use gcra::{Gcra, GcraBuilder};
pub use token_bucket::{TokenBucket, TokenBucketBuilder};
