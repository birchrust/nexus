//! Smoothing and filtering types.

pub use crate::{
    AsymEmaF32, AsymEmaF32Builder, AsymEmaF64, AsymEmaF64Builder, AsymEmaI32,
    AsymEmaI32Builder, AsymEmaI64, AsymEmaI64Builder, EmaF32, EmaF32Builder, EmaF64,
    EmaF64Builder, EmaI32, EmaI32Builder, EmaI64, EmaI64Builder, HoltF32, HoltF32Builder,
    HoltF64, HoltF64Builder, SlewF32, SlewF64, SlewI32, SlewI64, SlewI128, SpringF32,
    SpringF64,
};

#[cfg(feature = "alloc")]
pub use crate::{
    KamaF32, KamaF32Builder, KamaF64, KamaF64Builder, WindowedMedianF32, WindowedMedianF64,
    WindowedMedianI32, WindowedMedianI64,
};
