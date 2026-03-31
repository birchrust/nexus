//! Regression and classification.

mod ew_polynomial_regression;
pub(crate) mod linear_regression;
pub(crate) mod polynomial_regression;

#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
mod logistic_regression;
#[cfg(any(feature = "std", feature = "libm"))]
mod transformed_regression;

pub use ew_polynomial_regression::*;
pub use linear_regression::*;
pub use polynomial_regression::*;

#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
pub use logistic_regression::*;
#[cfg(any(feature = "std", feature = "libm"))]
pub use transformed_regression::*;
