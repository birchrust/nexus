//! Signal analysis types.

#[cfg(feature = "alloc")]
pub use crate::{
    AutocorrelationF32, AutocorrelationF32Builder, AutocorrelationF64, AutocorrelationF64Builder,
    AutocorrelationI32, AutocorrelationI32Builder, AutocorrelationI64, AutocorrelationI64Builder,
};

#[cfg(feature = "alloc")]
pub use crate::{
    CrossCorrelationF32, CrossCorrelationF32Builder, CrossCorrelationF64,
    CrossCorrelationF64Builder,
};
