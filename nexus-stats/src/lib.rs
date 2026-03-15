#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! Fixed-memory, zero-allocation streaming statistics for real-time systems.
//!
//! Every primitive is O(1) per update, fixed memory, `no_std` with no `alloc`.
//!
//! # Algorithms
//!
//! - **CUSUM** — Cumulative sum change detection (Page, 1954). Detects persistent
//!   mean shifts in either direction.
//! - **EMA** — Exponential moving average. Float variant uses standard alpha
//!   smoothing; integer variant uses kernel-style fixed-point bit-shift arithmetic.
//! - **Welford** — Online mean, variance, and standard deviation with numerical
//!   stability. Supports Chan's merge for parallel aggregation.

mod cusum;
mod ema;
mod math;
mod welford;

pub use cusum::{
    CusumF32, CusumF32Builder, CusumF64, CusumF64Builder, CusumI32, CusumI32Builder, CusumI64,
    CusumI64Builder, Shift,
};
pub use ema::{
    EmaF32, EmaF32Builder, EmaF64, EmaF64Builder, EmaI32, EmaI32Builder, EmaI64, EmaI64Builder,
};
pub use welford::{WelfordF32, WelfordF64};
