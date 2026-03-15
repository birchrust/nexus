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
