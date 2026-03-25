#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! Fixed-memory, zero-allocation streaming statistics for real-time systems.
//!
//! 50+ algorithms, all O(1) per update, fixed memory. Core types are `no_std`
//! compatible; types marked *(std)* require the `std` feature, *(alloc)* require
//! `alloc`, and *(std|libm)* require either `std` or `libm`.
//!
//! # Algorithms
//!
//! **Change Detection:**
//! - [`CusumF64`] — Cumulative sum (Page, 1954). Persistent mean shifts.
//! - [`MosumF64`] — Moving sum. Transient spikes within a window. *(alloc)*
//! - [`ShiryaevRobertsF64`] — Quasi-Bayesian. Optimal detection delay. *(std|libm)*
//!
//! **Anomaly Detection:**
//! - [`AdaptiveThresholdF64`] — EMA-based dynamic threshold. *(std|libm)*
//! - [`RobustZScoreF64`] — Median-based z-score (resistant to outliers).
//! - [`MultiGateF64`] — Cascaded gate checks with severity levels.
//! - [`TrendAlertF64`] — EMA trend detection with directional alerts.
//! - [`SaturationF64`] — Utilization monitor with threshold detection.
//! - [`ErrorRateF64`] — EMA-smoothed error rate with weighted severity.
//!
//! **Smoothing & Filtering:**
//! - [`EmaF64`] / [`EmaI64`] — Exponential moving average (float and integer).
//! - [`AsymEmaF64`] — Asymmetric EMA (separate rise/fall smoothing).
//! - [`HoltF64`] — Double exponential. Level + trend tracking.
//! - [`KamaF64`] — Kaufman adaptive moving average. *(alloc)*
//! - [`Kalman1dF64`] — Scalar Kalman filter (fixed dt=1).
//! - [`SpringF64`] — Critically damped spring follower.
//! - [`SlewF64`] — Slew rate limiter (max change per update).
//! - [`WindowedMedianF64`] — Streaming median over a sliding window. *(alloc)*
//!
//! **Statistics:**
//! - [`WelfordF64`] — Online mean, variance, std dev. Chan's merge.
//! - [`MomentsF64`] — Online skewness & kurtosis (Pébay, 2008).
//! - [`EwmaVarF64`] — Exponentially weighted variance (RiskMetrics).
//! - [`CovarianceF64`] — Online covariance and Pearson correlation.
//! - [`HarmonicMeanF64`] — Online harmonic mean.
//! - [`PercentileF64`] — P² streaming percentile (Jain & Chlamtac, 1985).
//!
//! **Signal Analysis:**
//! - [`AutocorrelationF64`] — Self-correlation at configurable lag.
//! - [`CrossCorrelationF64`] — Two-stream correlation with lead/lag detection. *(std|libm)*
//!
//! **Regression:**
//! - [`LinearRegressionF64`] — Online linear fit with closed-form solve (`y = ax + b`).
//! - [`EwLinearRegressionF64`] — Exponentially-weighted linear fit.
//! - [`PolynomialRegressionF64`] — Online polynomial fit, degree 2-8 (quadratic, cubic, etc.).
//! - [`EwPolynomialRegressionF64`] — Exponentially-weighted polynomial fit.
//! - [`ExponentialRegressionF64`] — Exponential fit (`y = ae^(bx)`). *(std|libm)*
//! - [`LogarithmicRegressionF64`] — Logarithmic fit (`y = a·ln(x) + b`). *(std|libm)*
//! - [`PowerRegressionF64`] — Power law fit (`y = ax^b`). *(std|libm)*
//!
//! **Information Theory:** *(std|libm)*
//! - [`EntropyF64`] — Shannon entropy over categorical distributions.
//! - [`TransferEntropyF64`] — Directed information flow (Granger causality). *(alloc, std|libm)*
//!
//! **Monitoring:**
//! - [`DrawdownF64`] — Peak-to-trough decline and max drawdown.
//! - [`WindowedMaxF64`] / [`WindowedMinF64`] — Nichols' algorithm (kernel `win_minmax.h`). *(std)*
//! - [`WindowedMaxF64Raw`] / [`WindowedMinF64Raw`] — Same algorithm, raw `u64` timestamps.
//! - [`RunningMinF64`] / [`RunningMaxF64`] — All-time min/max tracking.
//! - [`PeakHoldF64`] — Hold peak value with configurable decay.
//! - [`MaxGaugeF64`] — Track running maximum (reset on read).
//! - [`LivenessF64`] — EMA of inter-arrival times with deadline.
//! - [`LivenessInstant`] — Liveness with `Instant` timestamps. *(std)*
//! - [`EventRateF64`] — Smoothed event rate (events per unit time).
//! - [`EventRateInstant`] — Event rate with `Instant` timestamps. *(std)*
//! - [`CoDelI64`] — Controlled Delay queue monitor (Nichols & Jacobson, 2012). *(std)*
//! - [`CoDelI64Raw`] — CoDel with raw `u64` timestamps.
//! - [`JitterF64`] — EMA-smoothed inter-sample jitter.
//!
//! **Frequency & Scoring:**
//! - [`TopK`] — Space-Saving top-K frequent items.
//! - [`FlexProportionEntity`] / [`FlexProportionGlobal`] — Flexible fair-share proportioning.
//! - [`DecayAccumF64`] — Exponentially decaying accumulator. *(std|libm)*
//!
//! **Control & Thresholding:**
//! - [`DeadBandF64`] — Suppress changes below a threshold.
//! - [`HysteresisF64`] — Schmitt trigger with upper/lower thresholds.
//! - [`DebounceU32`] — Require N consecutive activations.
//! - [`LevelCrossingF64`] — Detect threshold crossings.
//! - [`PeakDetectorF64`] — Detect local peaks and troughs.
//! - [`BoolWindow`] — Count true/false over a sliding window. *(alloc)*
//!
//! **Differencing:**
//! - [`FirstDiffF64`] — First-order difference (Δx).
//! - [`SecondDiffF64`] — Second-order difference (Δ²x).
//!
//! # Features
//!
//! | Feature | Default | Enables |
//! |---------|---------|---------|
//! | `std` | yes | `Instant`-based windowed/CoDel/liveness/event-rate types, `sqrt`/`exp` intrinsics |
//! | `alloc` | with `std` | MOSUM, KAMA, WindowedMedian, BoolWindow (runtime-sized windows) |
//! | `libm` | no | Pure Rust `sqrt`/`exp` fallback for `no_std` (enables Shiryaev-Roberts, etc.) |
//!
//! For `no_std` without `alloc`: all core types work. Use `Raw` variants
//! (e.g., [`WindowedMaxF64Raw`]) for windowed tracking with raw integer timestamps.

#[cfg(feature = "alloc")]
extern crate alloc;

mod enums;

#[cfg(any(feature = "std", feature = "libm"))]
mod adaptive_threshold;
mod asym_ema;
mod autocorrelation;
#[cfg(feature = "alloc")]
mod bool_window;
mod codel;
mod covariance;
#[cfg(any(feature = "std", feature = "libm"))]
mod cross_correlation;
mod cusum;
mod dead_band;
mod debounce;
#[cfg(any(feature = "std", feature = "libm"))]
mod decay_accum;
mod diff;
mod drawdown;
mod ema;
#[cfg(any(feature = "std", feature = "libm"))]
mod entropy;
mod error_rate;
mod event_rate;
mod ew_polynomial_regression;
mod ewma_var;
mod flex_proportion;
mod harmonic_mean;
mod holt;
mod hysteresis;
mod jitter;
mod kalman1d;
#[cfg(feature = "alloc")]
mod kama;
mod level_crossing;
mod linear_regression;
mod liveness;
mod math;
mod max_gauge;
mod moments;
#[cfg(feature = "alloc")]
mod mosum;
mod multi_gate;
mod peak_detector;
mod peak_hold;
mod percentile;
mod polynomial_regression;
mod robust_z;
mod running;
mod saturation;
#[cfg(any(feature = "std", feature = "libm"))]
mod shiryaev_roberts;
mod slew;
mod spring;
mod topk;
#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
mod transfer_entropy;
#[cfg(any(feature = "std", feature = "libm"))]
mod transformed_regression;
mod trend_alert;
mod welford;
mod windowed;
#[cfg(feature = "alloc")]
mod windowed_median;

#[cfg(any(feature = "std", feature = "libm"))]
pub use adaptive_threshold::{
    AdaptiveThresholdF32, AdaptiveThresholdF32Builder, AdaptiveThresholdF64,
    AdaptiveThresholdF64Builder,
};
pub use asym_ema::{
    AsymEmaF32, AsymEmaF32Builder, AsymEmaF64, AsymEmaF64Builder, AsymEmaI32, AsymEmaI32Builder,
    AsymEmaI64, AsymEmaI64Builder,
};
pub use autocorrelation::{
    AutocorrelationF32, AutocorrelationF64, AutocorrelationI32, AutocorrelationI64,
};
#[cfg(feature = "alloc")]
pub use bool_window::BoolWindow;
#[cfg(feature = "std")]
pub use codel::{
    CoDelF32, CoDelF32Builder, CoDelF64, CoDelF64Builder, CoDelI32, CoDelI32Builder, CoDelI64,
    CoDelI64Builder, CoDelI128, CoDelI128Builder,
};
pub use codel::{
    CoDelF32Raw, CoDelF32RawBuilder, CoDelF64Raw, CoDelF64RawBuilder, CoDelI32Raw,
    CoDelI32RawBuilder, CoDelI64Raw, CoDelI64RawBuilder, CoDelI128Raw, CoDelI128RawBuilder,
};
pub use covariance::{CovarianceF32, CovarianceF64};
#[cfg(any(feature = "std", feature = "libm"))]
pub use cross_correlation::{CrossCorrelationF32, CrossCorrelationF64};
pub use cusum::{
    CusumF32, CusumF32Builder, CusumF64, CusumF64Builder, CusumI32, CusumI32Builder, CusumI64,
    CusumI64Builder, CusumI128, CusumI128Builder,
};
pub use dead_band::{DeadBandF32, DeadBandF64, DeadBandI32, DeadBandI64, DeadBandI128};
pub use debounce::{DebounceU32, DebounceU64};
#[cfg(any(feature = "std", feature = "libm"))]
pub use decay_accum::DecayAccumF64;
pub use diff::{
    FirstDiffF32, FirstDiffF64, FirstDiffI32, FirstDiffI64, FirstDiffI128, SecondDiffF32,
    SecondDiffF64, SecondDiffI32, SecondDiffI64, SecondDiffI128,
};
pub use drawdown::{DrawdownF32, DrawdownF64, DrawdownI32, DrawdownI64, DrawdownI128};
pub use ema::{
    EmaF32, EmaF32Builder, EmaF64, EmaF64Builder, EmaI32, EmaI32Builder, EmaI64, EmaI64Builder,
};
pub use enums::{Condition, ConfigError, Direction};
#[cfg(any(feature = "std", feature = "libm"))]
pub use entropy::{EntropyF32, EntropyF64};
pub use error_rate::{ErrorRateF32, ErrorRateF32Builder, ErrorRateF64, ErrorRateF64Builder};
pub use event_rate::{
    EventRateF32, EventRateF32Builder, EventRateF64, EventRateF64Builder, EventRateI32,
    EventRateI32Builder, EventRateI64, EventRateI64Builder,
};
#[cfg(feature = "std")]
pub use event_rate::{EventRateInstant, EventRateInstantBuilder};
pub use ew_polynomial_regression::{
    EwPolynomialRegressionF32, EwPolynomialRegressionF32Builder, EwPolynomialRegressionF64,
    EwPolynomialRegressionF64Builder,
};
pub use ewma_var::{EwmaVarF32, EwmaVarF32Builder, EwmaVarF64, EwmaVarF64Builder};
pub use flex_proportion::{FlexProportionEntity, FlexProportionGlobal};
pub use harmonic_mean::{HarmonicMeanF32, HarmonicMeanF64};
pub use holt::{HoltF32, HoltF32Builder, HoltF64, HoltF64Builder};
pub use hysteresis::{HysteresisF32, HysteresisF64, HysteresisI32, HysteresisI64, HysteresisI128};
pub use jitter::{
    JitterF32, JitterF32Builder, JitterF64, JitterF64Builder, JitterI32, JitterI32Builder,
    JitterI64, JitterI64Builder,
};
pub use kalman1d::{Kalman1dF32, Kalman1dF32Builder, Kalman1dF64, Kalman1dF64Builder};
#[cfg(feature = "alloc")]
pub use kama::{KamaF32, KamaF32Builder, KamaF64, KamaF64Builder};
pub use linear_regression::{
    EwLinearRegressionF32, EwLinearRegressionF32Builder, EwLinearRegressionF64,
    EwLinearRegressionF64Builder, LinearRegressionF32, LinearRegressionF32Builder,
    LinearRegressionF64, LinearRegressionF64Builder,
};
pub use level_crossing::{
    LevelCrossingF32, LevelCrossingF64, LevelCrossingI32, LevelCrossingI64, LevelCrossingI128,
};
pub use liveness::{
    LivenessF32, LivenessF32Builder, LivenessF64, LivenessF64Builder, LivenessI32,
    LivenessI32Builder, LivenessI64, LivenessI64Builder,
};
#[cfg(feature = "std")]
pub use liveness::{LivenessInstant, LivenessInstantBuilder};
pub use max_gauge::{MaxGaugeF32, MaxGaugeF64, MaxGaugeI32, MaxGaugeI64, MaxGaugeI128};
pub use moments::{MomentsF32, MomentsF64, MomentsI32, MomentsI64};
#[cfg(feature = "alloc")]
pub use mosum::{
    MosumF32, MosumF32Builder, MosumF64, MosumF64Builder, MosumI32, MosumI32Builder, MosumI64,
    MosumI64Builder, MosumI128, MosumI128Builder,
};
pub use multi_gate::{
    MultiGateF32, MultiGateF32Builder, MultiGateF64, MultiGateF64Builder, Verdict,
};
pub use peak_detector::{
    Peak, PeakDetectorF32, PeakDetectorF64, PeakDetectorI32, PeakDetectorI64, PeakDetectorI128,
};
pub use peak_hold::{
    PeakHoldF32, PeakHoldF32Builder, PeakHoldF64, PeakHoldF64Builder, PeakHoldI32,
    PeakHoldI32Builder, PeakHoldI64, PeakHoldI64Builder, PeakHoldI128, PeakHoldI128Builder,
};
pub use percentile::{PercentileF32, PercentileF32Builder, PercentileF64, PercentileF64Builder};
pub use polynomial_regression::{
    CoefficientsF32, CoefficientsF64, PolynomialRegressionF32, PolynomialRegressionF32Builder,
    PolynomialRegressionF64, PolynomialRegressionF64Builder,
};
pub use robust_z::{
    RobustZScoreF32, RobustZScoreF32Builder, RobustZScoreF64, RobustZScoreF64Builder,
};
pub use running::{
    RunningMaxF32, RunningMaxF64, RunningMaxI32, RunningMaxI64, RunningMaxI128, RunningMinF32,
    RunningMinF64, RunningMinI32, RunningMinI64, RunningMinI128,
};
pub use saturation::{SaturationF32, SaturationF32Builder, SaturationF64, SaturationF64Builder};
#[cfg(any(feature = "std", feature = "libm"))]
pub use shiryaev_roberts::{ShiryaevRobertsF64, ShiryaevRobertsF64Builder};
pub use slew::{SlewF32, SlewF64, SlewI32, SlewI64, SlewI128};
pub use spring::{SpringF32, SpringF64};
pub use topk::TopK;
#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
pub use transfer_entropy::{TransferEntropyF64, TransferEntropyF64Builder};
#[cfg(any(feature = "std", feature = "libm"))]
pub use transformed_regression::{
    EwExponentialRegressionF64, EwExponentialRegressionF64Builder,
    EwLogarithmicRegressionF64, EwLogarithmicRegressionF64Builder,
    EwPowerRegressionF64, EwPowerRegressionF64Builder,
    ExponentialRegressionF32, ExponentialRegressionF64,
    LogarithmicRegressionF32, LogarithmicRegressionF64,
    PowerRegressionF32, PowerRegressionF64,
};
pub use trend_alert::{TrendAlertF32, TrendAlertF32Builder, TrendAlertF64, TrendAlertF64Builder};
pub use welford::{WelfordF32, WelfordF64};
#[cfg(feature = "std")]
pub use windowed::{
    WindowedMaxF32, WindowedMaxF64, WindowedMaxI32, WindowedMaxI64, WindowedMaxI128,
    WindowedMinF32, WindowedMinF64, WindowedMinI32, WindowedMinI64, WindowedMinI128,
};
pub use windowed::{
    WindowedMaxF32Raw, WindowedMaxF64Raw, WindowedMaxI32Raw, WindowedMaxI64Raw, WindowedMaxI128Raw,
    WindowedMinF32Raw, WindowedMinF64Raw, WindowedMinI32Raw, WindowedMinI64Raw, WindowedMinI128Raw,
};
#[cfg(feature = "alloc")]
pub use windowed_median::{
    WindowedMedianF32, WindowedMedianF64, WindowedMedianI32, WindowedMedianI64,
};
