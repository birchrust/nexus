//! Information theory types.

#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
pub use crate::{EntropyF32, EntropyF32Builder, EntropyF64, EntropyF64Builder};

#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
pub use crate::{TransferEntropyF64, TransferEntropyF64Builder};
