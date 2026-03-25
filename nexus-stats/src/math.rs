/// Square root.
///
/// Requires `std` or `libm` feature. Types using this (`std_dev()`,
/// `ShiryaevRoberts`) won't compile without one of these features.
#[cfg(any(feature = "std", feature = "libm"))]
#[inline]
pub(crate) fn sqrt(x: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        x.sqrt()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        libm::sqrt(x)
    }
}

/// Exponential function.
///
/// Requires `std` or `libm` feature. Types using this (`ShiryaevRoberts`,
/// `halflife()` constructors) won't compile without one of these features.
#[cfg(any(feature = "std", feature = "libm"))]
#[inline]
pub(crate) fn exp(x: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        x.exp()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        libm::exp(x)
    }
}

/// Natural logarithm.
///
/// Requires `std` or `libm` feature. Types using this (`EntropyF64`,
/// `TransferEntropyF64`) won't compile without one of these features.
#[cfg(any(feature = "std", feature = "libm"))]
#[inline]
pub(crate) fn ln(x: f64) -> f64 {
    #[cfg(feature = "std")]
    {
        x.ln()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        libm::log(x)
    }
}

/// Trait providing `fma` (fused multiply-add) across all feature configurations.
///
/// With `std`: uses hardware FMA intrinsic.
/// With `libm`: uses `libm::fma` / `libm::fmaf`.
/// Without either: falls back to `a * b + c` (no fusion, but correct).
pub(crate) trait MulAdd {
    /// Fused multiply-add: `self * b + c`.
    fn fma(self, b: Self, c: Self) -> Self;
}

impl MulAdd for f64 {
    #[inline]
    fn fma(self, b: f64, c: f64) -> f64 {
        #[cfg(feature = "std")]
        {
            self.mul_add(b, c)
        }
        #[cfg(all(not(feature = "std"), feature = "libm"))]
        {
            libm::fma(self, b, c)
        }
        #[cfg(not(any(feature = "std", feature = "libm")))]
        {
            self * b + c
        }
    }
}

impl MulAdd for f32 {
    #[inline]
    fn fma(self, b: f32, c: f32) -> f32 {
        #[cfg(feature = "std")]
        {
            self.mul_add(b, c)
        }
        #[cfg(all(not(feature = "std"), feature = "libm"))]
        {
            libm::fmaf(self, b, c)
        }
        #[cfg(not(any(feature = "std", feature = "libm")))]
        {
            self * b + c
        }
    }
}
