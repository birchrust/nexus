#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! Advanced control and frequency primitives for [`nexus-stats`](https://docs.rs/nexus-stats).
//!
//! This crate provides control and frequency types separated from the core
//! `nexus-stats` crate.
//!
//! Types are organized into submodules:
//! - [`control`] — PeakDetector, BoolWindow
//! - [`frequency`] — TopK, FlexProportion, DecayAccum

#[cfg(feature = "alloc")]
extern crate alloc;

/// Validates that a float value is finite (not NaN, not Inf).
macro_rules! check_finite {
    ($val:expr) => {
        if !$val.is_finite() {
            return Err(if $val.is_nan() {
                nexus_stats_core::DataError::NotANumber
            } else {
                nexus_stats_core::DataError::Infinite
            });
        }
    };
}

// Internal modules
mod peak_detector;

#[cfg(feature = "alloc")]
mod bool_window;

/// Advanced control types.
pub mod control {
    pub use super::peak_detector::*;

    #[cfg(feature = "alloc")]
    pub use super::bool_window::*;
}

/// Frequency counting and scoring.
pub mod frequency;
