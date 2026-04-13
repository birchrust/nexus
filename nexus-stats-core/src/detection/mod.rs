//! Change detection.

mod cusum;
#[cfg(any(feature = "std", feature = "libm"))]
mod distribution_shift;

pub use cusum::*;
#[cfg(any(feature = "std", feature = "libm"))]
pub use distribution_shift::*;
