//! Control, thresholding, and differencing.

mod dead_band;
mod hysteresis;
mod debounce;
mod level_crossing;
mod peak_detector;
mod diff;

#[cfg(feature = "alloc")]
mod bool_window;

pub use dead_band::*;
pub use hysteresis::*;
pub use debounce::*;
pub use level_crossing::*;
pub use peak_detector::*;
pub use diff::*;

#[cfg(feature = "alloc")]
pub use bool_window::*;
