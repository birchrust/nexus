//! Adaptive filters and online learning.

#[cfg(feature = "alloc")]
pub use crate::{
    LmsFilterF32, LmsFilterF32Builder, LmsFilterF64, LmsFilterF64Builder, NlmsFilterF32,
    NlmsFilterF32Builder, NlmsFilterF64, NlmsFilterF64Builder, OnlineKMeansF64,
    OnlineKMeansF64Builder, RlsFilterF32, RlsFilterF32Builder, RlsFilterF64,
    RlsFilterF64Builder,
};

#[cfg(all(feature = "alloc", any(feature = "std", feature = "libm")))]
pub use crate::{LogisticRegressionF64, LogisticRegressionF64Builder};
