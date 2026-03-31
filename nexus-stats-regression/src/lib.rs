#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

//! Online regression, learning, and estimation for [`nexus-stats`](https://docs.rs/nexus-stats).
//!
//! This crate provides regression, adaptive learning, and Bayesian estimation
//! types separated from the core `nexus-stats` crate.
//!
//! # Regression Types
//!
//! | Type | Description | Feature |
//! |------|-------------|---------|
//! | `LinearRegressionF64` | Online linear regression | — |
//! | `PolynomialRegressionF64` | Online polynomial regression | — |
//! | `EwPolynomialRegressionF64` | Exponentially weighted polynomial regression | — |
//! | `TransformedRegressionF64` | Log/exp/power/reciprocal transformed fits | `std` or `libm` |
//! | `LogisticRegressionF64` | Online logistic regression | `alloc` + (`std` or `libm`) |
//!
//! # Learning Types
//!
//! | Type | Description | Feature |
//! |------|-------------|---------|
//! | `LmsFilterF64` | Least Mean Squares adaptive filter | `alloc` |
//! | `RlsFilterF64` | Recursive Least Squares adaptive filter | `alloc` |
//! | `OnlineKMeansF64` | Online K-Means clustering | `alloc` |
//! | `OnlineGdF64` | Online gradient descent optimizer | `alloc` |
//! | `AdaGradF64` | AdaGrad optimizer | `alloc` + (`std` or `libm`) |
//! | `AdamF64` | Adam optimizer | `alloc` + (`std` or `libm`) |
//!
//! # Estimation Types
//!
//! | Type | Description | Feature |
//! |------|-------------|---------|
//! | `Kalman2dF64` | 2D Kalman filter | — |
//! | `Kalman3dF64` | 3D Kalman filter | — |
//! | `BetaBinomialF64` | Beta-Binomial conjugate estimator | `std` or `libm` |
//! | `GammaPoissonF64` | Gamma-Poisson conjugate estimator | `std` or `libm` |
//!
//! # Re-export
//!
//! When the `regression` feature is enabled on `nexus-stats`, these types are
//! re-exported under `nexus_stats_regression::regression::*`, `nexus_stats_regression::learning::*`,
//! and `nexus_stats_regression::estimation::*`.

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

/// State estimation and Bayesian inference.
pub mod estimation;
/// Adaptive filters, online learning, and optimization.
pub mod learning;
/// Regression and classification.
pub mod regression;
