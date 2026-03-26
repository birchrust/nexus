//! State estimation, Bayesian inference, and hypothesis testing.

mod kalman1d;
mod kalman2d;
mod kalman3d;
mod beta_binomial;
mod gamma_poisson;

#[cfg(any(feature = "std", feature = "libm"))]
mod sprt;

pub use kalman1d::*;
pub use kalman2d::*;
pub use kalman3d::*;
pub use beta_binomial::*;
pub use gamma_poisson::*;

#[cfg(any(feature = "std", feature = "libm"))]
pub use sprt::*;
