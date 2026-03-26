//! Adaptive filters, online learning, and optimization.

#[cfg(feature = "alloc")]
mod lms;
#[cfg(feature = "alloc")]
mod rls;
#[cfg(feature = "alloc")]
mod online_kmeans;
#[cfg(feature = "alloc")]
mod online_gd;
#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
mod adagrad;
#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
mod adam;

#[cfg(feature = "alloc")]
pub use lms::*;
#[cfg(feature = "alloc")]
pub use rls::*;
#[cfg(feature = "alloc")]
pub use online_kmeans::*;
#[cfg(feature = "alloc")]
pub use online_gd::*;
#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
pub use adagrad::*;
#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
pub use adam::*;
