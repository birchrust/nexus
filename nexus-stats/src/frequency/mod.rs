//! Frequency counting and scoring.

mod topk;
mod flex_proportion;

#[cfg(any(feature = "std", feature = "libm"))]
mod decay_accum;

pub use topk::*;
pub use flex_proportion::*;

#[cfg(any(feature = "std", feature = "libm"))]
pub use decay_accum::*;
