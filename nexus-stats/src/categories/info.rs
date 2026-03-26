//! Information theory types.

#[cfg(any(feature = "std", feature = "libm"))]
pub use crate::{EntropyF32, EntropyF64};

#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
pub use crate::{TransferEntropyF64, TransferEntropyF64Builder};
