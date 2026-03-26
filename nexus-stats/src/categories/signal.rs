//! Signal analysis types.

pub use crate::{AutocorrelationF32, AutocorrelationF64, AutocorrelationI32, AutocorrelationI64};

#[cfg(any(feature = "std", feature = "libm"))]
pub use crate::{CrossCorrelationF32, CrossCorrelationF64};
