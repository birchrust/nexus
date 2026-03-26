//! Smoothing and filtering primitives.

pub(crate) mod ema;
mod asym_ema;
mod holt;
mod spring;
mod slew;

#[cfg(feature = "alloc")]
mod kama;
#[cfg(feature = "alloc")]
mod windowed_median;

pub use ema::*;
pub use asym_ema::*;
pub use holt::*;
pub use spring::*;
pub use slew::*;

#[cfg(feature = "alloc")]
pub use kama::*;
#[cfg(feature = "alloc")]
pub use windowed_median::*;
