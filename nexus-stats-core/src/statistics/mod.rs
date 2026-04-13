//! Core streaming statistics.

mod amihud;
mod bucket;
mod covariance;
#[cfg(feature = "alloc")]
mod covariance_matrix;
mod ewma_var;
#[cfg(any(feature = "std", feature = "libm"))]
mod half_life;
mod harmonic_mean;
mod hit_rate;
#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
mod hurst;
mod moments;
mod percentile;
#[cfg(feature = "alloc")]
mod variance_ratio;
mod welford;

pub use amihud::*;
pub use bucket::*;
pub use covariance::*;
#[cfg(feature = "alloc")]
pub use covariance_matrix::*;
pub use ewma_var::*;
#[cfg(any(feature = "std", feature = "libm"))]
pub use half_life::*;
pub use harmonic_mean::*;
pub use hit_rate::*;
#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
pub use hurst::*;
pub use moments::*;
pub use percentile::*;
#[cfg(feature = "alloc")]
pub use variance_ratio::*;
pub use welford::*;
