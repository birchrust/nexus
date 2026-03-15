//! Single-threaded rate limiters.
//!
//! All types use `&mut self` — no atomic overhead.

mod gcra;
mod token_bucket;
#[cfg(feature = "alloc")]
mod sliding_window;

pub use gcra::{Gcra, GcraBuilder};
pub use token_bucket::{TokenBucket, TokenBucketBuilder};
#[cfg(feature = "alloc")]
pub use sliding_window::{SlidingWindow, SlidingWindowBuilder};
