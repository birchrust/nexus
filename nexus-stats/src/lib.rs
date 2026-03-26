#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! Fixed-memory, zero-allocation streaming statistics for real-time systems.
//!
//! 65+ algorithms, all O(1) per update (or O(d) for d-dimensional filters), fixed memory.
//! Core types are `no_std` compatible; types marked *(std)* require the `std` feature,
//! *(alloc)* require `alloc`, and *(std|libm)* require either `std` or `libm`.
//!
//! # Usage
//!
//! Import from the category module you need:
//!
//! ```rust
//! use nexus_stats::smoothing::EmaF64;
//! use nexus_stats::detection::CusumF64;
//! use nexus_stats::statistics::WelfordF64;
//! use nexus_stats::regression::LinearRegressionF64;
//! use nexus_stats::learning::AdamF64;
//! use nexus_stats::{DataError, ConfigError, Direction};
//! ```
//!
//! # Data Quality & Error Policy
//!
//! nexus-stats distinguishes two failure categories:
//!
//! **Data errors** — NaN or Inf values reaching a streaming update.
//! These indicate upstream data quality problems (broken feeds, failed
//! computations, missing values). All update methods that accept float
//! inputs return `Result<_, DataError>`. The library rejects the input
//! and leaves internal state unchanged. The caller declares the policy:
//!
//! - `.unwrap()` to crash on bad data (testing, strict systems)
//! - Log and continue (monitoring, degraded-mode operation)
//! - Increment a counter and trigger a circuit breaker (production)
//!
//! **Programmer errors** — wrong dimensions, out-of-range indices,
//! type misuse. These are bugs in the calling code. The library panics
//! via `assert!`. Fix the code.
//!
//! ## Internal State Invariant
//!
//! Given only finite (non-NaN, non-Inf) inputs, all internal
//! accumulators remain finite for typical workloads. Extreme value
//! ranges (>1e150) or very long-running instances (billions of updates)
//! can cause internal accumulator overflow through summation. For
//! long-running systems: call `reset()` periodically, use
//! exponentially-weighted variants (EW*) which naturally bound growth,
//! or use `.max_covariance()` on RLS filters to auto-reset when the
//! covariance matrix diverges.
//!
//! # Categories
//!
//! | Module | Contents |
//! |--------|----------|
//! | [`smoothing`] | EMA, Holt, KAMA, Kalman1d, Spring, Slew, WindowedMedian |
//! | [`detection`] | CUSUM, MOSUM, Shiryaev-Roberts, AdaptiveThreshold, RobustZ, TrendAlert, MultiGate |
//! | [`statistics`] | Welford, Moments, EwmaVar, Covariance, HarmonicMean, Percentile |
//! | [`monitoring`] | Drawdown, Windowed Min/Max, CoDel, Liveness, EventRate, Jitter, ErrorRate, Saturation |
//! | [`signal`] | Autocorrelation, CrossCorrelation, Entropy, TransferEntropy |
//! | [`regression`] | Linear, Polynomial, EW variants, Transformed fits, LogisticRegression |
//! | [`estimation`] | Kalman 2d/3d, BetaBinomial, GammaPoisson, SPRT |
//! | [`learning`] | LMS, NLMS, RLS, OnlineKMeans, GD, AdaGrad, Adam |
//! | [`control`] | DeadBand, Hysteresis, Debounce, LevelCrossing, PeakDetector, BoolWindow, Diff |
//! | [`frequency`] | TopK, FlexProportion, DecayAccum |
//!
//! # Features
//!
//! | Feature | Default | Enables |
//! |---------|---------|---------|
//! | `std` | yes | `Instant`-based windowed/CoDel/liveness/event-rate types, `sqrt`/`exp` intrinsics |
//! | `alloc` | with `std` | MOSUM, KAMA, WindowedMedian, BoolWindow, adaptive filters, optimizers |
//! | `libm` | no | Pure Rust `sqrt`/`exp` fallback for `no_std` (enables Shiryaev-Roberts, etc.) |

#[cfg(feature = "alloc")]
extern crate alloc;

// Shared types at crate root
mod enums;
#[macro_use]
mod math;
mod feature_vector;

pub use enums::{Condition, ConfigError, DataError, Direction};

// Category modules
pub mod smoothing;
pub mod detection;
pub mod statistics;
pub mod monitoring;
pub mod signal;
pub mod regression;
pub mod estimation;
pub mod learning;
pub mod control;
pub mod frequency;
