#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! Fixed-memory, zero-allocation streaming statistics for real-time systems.
//!
//! Every primitive is O(1) per update, fixed memory, `no_std` with no `alloc`.
//!
//! # Algorithms
//!
//! **Change Detection:**
//! - [`CusumF64`] — Cumulative sum (Page, 1954). Persistent mean shifts.
//! - [`MosumF64`] — Moving sum. Transient spikes within a window.
//! - [`ShiryaevRobertsF64`] — Quasi-Bayesian. Optimal detection delay.
//!
//! **Smoothing:**
//! - [`EmaF64`] / [`EmaI64`] — Exponential moving average (float and integer).
//! - [`HoltF64`] — Double exponential. Level + trend tracking.
//!
//! **Variance & Correlation:**
//! - [`WelfordF64`] — Online mean, variance, std dev. Chan's merge.
//! - [`EwmaVarF64`] — Exponentially weighted variance (RiskMetrics).
//! - [`CovarianceF64`] — Online covariance and Pearson correlation.
//!
//! **Monitoring:**
//! - [`DrawdownF64`] — Peak-to-trough decline and max drawdown.
//! - [`WindowedMaxF64`] / [`WindowedMinF64`] — Nichols' algorithm (kernel `win_minmax.h`).
//! - [`RunningMinF64`] / [`RunningMaxF64`] — All-time min/max tracking.
//! - [`LivenessF64`] — EMA of inter-arrival times with deadline.
//! - [`EventRateF64`] — Smoothed event rate (events per unit time).
//! - [`QueueDelayI64`] — Queue sojourn time monitor (CoDel-inspired backpressure detection).
//!
//! **Frequency:**
//! - [`TopK`] — Space-Saving top-K frequent items.

mod adaptive_threshold;
mod bool_window;
mod covariance;
mod cusum;
mod dead_band;
mod debounce;
mod diff;
mod drawdown;
mod ema;
mod error_rate;
mod event_rate;
mod ewma_var;
mod harmonic_mean;
mod holt;
mod hysteresis;
mod jitter;
mod level_crossing;
mod liveness;
mod math;
mod max_gauge;
mod mosum;
mod peak_detector;
mod queue_delay;
mod running;
mod saturation;
mod shiryaev_roberts;
mod slew;
mod topk;
mod trend_alert;
mod welford;
mod windowed;

pub use bool_window::BoolWindow;
pub use adaptive_threshold::{
    AdaptiveThresholdF32, AdaptiveThresholdF32Builder, AdaptiveThresholdF64,
    AdaptiveThresholdF64Builder, Anomaly,
};
pub use covariance::{CovarianceF32, CovarianceF64};
pub use dead_band::{DeadBandF32, DeadBandF64, DeadBandI32, DeadBandI64};
pub use debounce::{DebounceU32, DebounceU64};
pub use diff::{
    FirstDiffF32, FirstDiffF64, FirstDiffI32, FirstDiffI64, SecondDiffF32, SecondDiffF64,
    SecondDiffI32, SecondDiffI64,
};
pub use cusum::{
    CusumF32, CusumF32Builder, CusumF64, CusumF64Builder, CusumI32, CusumI32Builder, CusumI64,
    CusumI64Builder, Shift,
};
pub use drawdown::{DrawdownF32, DrawdownF64, DrawdownI32, DrawdownI64};
pub use error_rate::{ErrorRateF32, ErrorRateF32Builder, ErrorRateF64, ErrorRateF64Builder, Health};
pub use event_rate::{
    EventRateF32, EventRateF32Builder, EventRateF64, EventRateF64Builder, EventRateI32,
    EventRateI32Builder, EventRateI64, EventRateI64Builder,
};
pub use ema::{
    EmaF32, EmaF32Builder, EmaF64, EmaF64Builder, EmaI32, EmaI32Builder, EmaI64, EmaI64Builder,
};
pub use ewma_var::{EwmaVarF32, EwmaVarF32Builder, EwmaVarF64, EwmaVarF64Builder};
pub use harmonic_mean::{HarmonicMeanF32, HarmonicMeanF64};
pub use holt::{HoltF32, HoltF32Builder, HoltF64, HoltF64Builder};
pub use hysteresis::{HysteresisF32, HysteresisF64, HysteresisI32, HysteresisI64};
pub use jitter::{
    JitterF32, JitterF32Builder, JitterF64, JitterF64Builder, JitterI32, JitterI32Builder,
    JitterI64, JitterI64Builder,
};
pub use level_crossing::{LevelCrossingF32, LevelCrossingF64, LevelCrossingI32, LevelCrossingI64};
pub use liveness::{
    LivenessF32, LivenessF32Builder, LivenessF64, LivenessF64Builder, LivenessI32,
    LivenessI32Builder, LivenessI64, LivenessI64Builder,
};
pub use max_gauge::{MaxGaugeF32, MaxGaugeF64, MaxGaugeI32, MaxGaugeI64};
pub use mosum::{MosumF32, MosumF32Builder, MosumF64, MosumF64Builder, MosumI32, MosumI32Builder, MosumI64, MosumI64Builder};
pub use peak_detector::{Peak, PeakDetectorF32, PeakDetectorF64, PeakDetectorI32, PeakDetectorI64};
pub use queue_delay::{QueueDelayI32, QueueDelayI32Builder, QueueDelayI64, QueueDelayI64Builder, QueuePressure};
pub use saturation::{SaturationF32, SaturationF32Builder, SaturationF64, SaturationF64Builder, Pressure};
pub use running::{
    RunningMaxF32, RunningMaxF64, RunningMaxI32, RunningMaxI64, RunningMinF32, RunningMinF64,
    RunningMinI32, RunningMinI64,
};
pub use shiryaev_roberts::{ShiryaevRobertsF64, ShiryaevRobertsF64Builder};
pub use slew::{SlewF32, SlewF64, SlewI32, SlewI64};
pub use topk::TopK;
pub use trend_alert::{Trend, TrendAlertF32, TrendAlertF32Builder, TrendAlertF64, TrendAlertF64Builder};
pub use welford::{WelfordF32, WelfordF64};
pub use windowed::{
    WindowedMaxF32, WindowedMaxF64, WindowedMaxI32, WindowedMaxI64, WindowedMinF32,
    WindowedMinF64, WindowedMinI32, WindowedMinI64,
};
