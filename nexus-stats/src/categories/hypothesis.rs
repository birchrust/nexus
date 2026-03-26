//! Sequential hypothesis testing.

#[cfg(any(feature = "std", feature = "libm"))]
pub use crate::{
    Decision, SprtBernoulli, SprtBernoulliBuilder, SprtGaussian, SprtGaussianBuilder,
};
