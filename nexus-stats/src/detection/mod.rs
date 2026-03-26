//! Change detection and anomaly detection.

mod cusum;
mod multi_gate;
mod robust_z;
mod trend_alert;

#[cfg(feature = "alloc")]
mod mosum;
#[cfg(any(feature = "std", feature = "libm"))]
mod adaptive_threshold;
#[cfg(any(feature = "std", feature = "libm"))]
mod shiryaev_roberts;

pub use cusum::*;
pub use multi_gate::*;
pub use robust_z::*;
pub use trend_alert::*;

#[cfg(feature = "alloc")]
pub use mosum::*;
#[cfg(any(feature = "std", feature = "libm"))]
pub use adaptive_threshold::*;
#[cfg(any(feature = "std", feature = "libm"))]
pub use shiryaev_roberts::*;
