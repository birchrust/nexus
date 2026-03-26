//! Regression types — linear, polynomial, and transformed fits.

pub use crate::{
    CoefficientsF32, CoefficientsF64, EwLinearRegressionF32, EwLinearRegressionF32Builder,
    EwLinearRegressionF64, EwLinearRegressionF64Builder, EwPolynomialRegressionF32,
    EwPolynomialRegressionF32Builder, EwPolynomialRegressionF64,
    EwPolynomialRegressionF64Builder, LinearRegressionF32, LinearRegressionF32Builder,
    LinearRegressionF64, LinearRegressionF64Builder, PolynomialRegressionF32,
    PolynomialRegressionF32Builder, PolynomialRegressionF64, PolynomialRegressionF64Builder,
};

#[cfg(any(feature = "std", feature = "libm"))]
pub use crate::{
    EwExponentialRegressionF64, EwExponentialRegressionF64Builder,
    EwLogarithmicRegressionF64, EwLogarithmicRegressionF64Builder, EwPowerRegressionF64,
    EwPowerRegressionF64Builder, ExponentialRegressionF32, ExponentialRegressionF64,
    LogarithmicRegressionF32, LogarithmicRegressionF64, PowerRegressionF32, PowerRegressionF64,
};
