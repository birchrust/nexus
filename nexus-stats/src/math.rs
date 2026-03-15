#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("nexus-stats requires either the `std` (default) or `libm` feature — add one to your Cargo.toml");

/// Square root.
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

/// Trait providing `mul_add` that works in both `std` and `no_std` + `libm`.
pub(crate) trait MulAdd {
    /// Fused multiply-add: `self * b + c`.
    fn fma(self, b: Self, c: Self) -> Self;
}

impl MulAdd for f64 {
    #[inline]
    fn fma(self, b: f64, c: f64) -> f64 {
        #[cfg(feature = "std")]
        { self.mul_add(b, c) }
        #[cfg(all(not(feature = "std"), feature = "libm"))]
        { libm::fma(self, b, c) }
    }
}

impl MulAdd for f32 {
    #[inline]
    fn fma(self, b: f32, c: f32) -> f32 {
        #[cfg(feature = "std")]
        { self.mul_add(b, c) }
        #[cfg(all(not(feature = "std"), feature = "libm"))]
        { libm::fmaf(self, b, c) }
    }
}
