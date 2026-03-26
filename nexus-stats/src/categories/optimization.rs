//! Online parameter optimization.

#[cfg(feature = "alloc")]
pub use crate::{OnlineGdF64, OnlineGdF64Builder};

#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
pub use crate::{AdaGradF64, AdaGradF64Builder, AdamF64, AdamF64Builder};
