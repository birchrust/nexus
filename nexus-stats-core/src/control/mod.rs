//! Control, thresholding, and differencing.

mod dead_band;
mod debounce;
mod diff;
mod hysteresis;
mod level_crossing;

pub use dead_band::*;
pub use debounce::*;
pub use diff::*;
pub use hysteresis::*;
pub use level_crossing::*;
