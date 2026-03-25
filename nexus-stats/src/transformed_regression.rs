// Transformed Regression — Linearized Fits via ln
//
// Thin wrappers around linear regression that apply ln at the API boundary:
// - Exponential: y = a * e^(bx)  →  ln(y) = ln(a) + bx
// - Logarithmic: y = a * ln(x) + b  →  y = a * ln(x) + b  (already linear in ln(x))
// - Power law: y = a * x^b  →  ln(y) = ln(a) + b * ln(x)

use crate::linear_regression::{
    EwLinearRegressionF64, LinearRegressionF32, LinearRegressionF64,
};

// ============================================================================
// Exponential: y = a * e^(bx)
// ============================================================================

macro_rules! impl_exponential_regression {
    ($name:ident, $inner:ident, $ty:ty) => {
        /// Online exponential regression: `y = a · e^(bx)`.
        ///
        /// Linearized as `ln(y) = ln(a) + bx` and solved via linear regression.
        /// Observations with `y <= 0` are silently skipped (ln undefined).
        ///
        /// R² is measured in log-space (goodness of fit of `ln(y)` vs `x`).
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use nexus_stats::", stringify!($name), ";")]
        ///
        #[doc = concat!("let mut r = ", stringify!($name), "::new();")]
        /// for x in 0..100 {
        #[doc = concat!("    let y = 2.0 as ", stringify!($ty), " * (0.05 as ", stringify!($ty), " * x as ", stringify!($ty), ").exp();")]
        ///     r.update(x as _, y);
        /// }
        /// let rate = r.growth_rate().unwrap();
        /// assert!((rate - 0.05).abs() < 0.01);
        /// ```
        #[derive(Debug, Clone)]
        pub struct $name {
            inner: $inner,
        }

        impl $name {
            /// Creates a new empty exponential regression.
            #[inline]
            #[must_use]
            pub fn new() -> Self {
                Self { inner: $inner::new() }
            }

            /// Feeds (x, y). Silently skips if `y <= 0`.
            #[inline]
            pub fn update(&mut self, x: $ty, y: $ty) {
                if y > 0.0 as $ty {
                    #[allow(clippy::cast_possible_truncation)]
                    let ln_y = crate::math::ln(y as f64) as $ty;
                    self.inner.update(x, ln_y);
                }
            }

            /// Growth/decay rate (the exponent b), or `None` if not primed.
            #[must_use]
            pub fn growth_rate(&self) -> Option<$ty> {
                self.inner.slope()
            }

            /// Scale factor `a = e^(intercept)`, or `None` if not primed.
            #[must_use]
            pub fn scale(&self) -> Option<$ty> {
                self.inner.intercept_value().map(|v| {
                    #[allow(clippy::cast_possible_truncation)]
                    { crate::math::exp(v as f64) as $ty }
                })
            }

            /// R² in log-space.
            #[must_use]
            pub fn r_squared(&self) -> Option<$ty> {
                self.inner.r_squared()
            }

            /// Predict `y = a · e^(bx)`.
            #[must_use]
            pub fn predict(&self, x: $ty) -> Option<$ty> {
                self.inner.predict(x).map(|ln_y| {
                    #[allow(clippy::cast_possible_truncation)]
                    { crate::math::exp(ln_y as f64) as $ty }
                })
            }

            /// Number of accepted observations (y > 0).
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.inner.count()
            }

            /// Whether enough data for a fit (>= 2 observations with y > 0).
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.inner.is_primed()
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.inner.reset();
            }
        }

        impl Default for $name {
            #[inline]
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

impl_exponential_regression!(ExponentialRegressionF64, LinearRegressionF64, f64);
impl_exponential_regression!(ExponentialRegressionF32, LinearRegressionF32, f32);

// ============================================================================
// Logarithmic: y = a * ln(x) + b
// ============================================================================

macro_rules! impl_logarithmic_regression {
    ($name:ident, $inner:ident, $ty:ty) => {
        /// Online logarithmic regression: `y = a · ln(x) + b`.
        ///
        /// Linearized by substituting `u = ln(x)`, solving `y = a·u + b`.
        /// Observations with `x <= 0` are silently skipped (ln undefined).
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use nexus_stats::", stringify!($name), ";")]
        ///
        #[doc = concat!("let mut r = ", stringify!($name), "::new();")]
        /// for x in 1..200 {
        #[doc = concat!("    let y = 3.0 as ", stringify!($ty), " * (x as ", stringify!($ty), ").ln() + 1.0 as ", stringify!($ty), ";")]
        ///     r.update(x as _, y);
        /// }
        /// let slope = r.slope().unwrap();
        /// assert!((slope - 3.0).abs() < 0.01);
        /// ```
        #[derive(Debug, Clone)]
        pub struct $name {
            inner: $inner,
        }

        impl $name {
            /// Creates a new empty logarithmic regression.
            #[inline]
            #[must_use]
            pub fn new() -> Self {
                Self { inner: $inner::new() }
            }

            /// Feeds (x, y). Silently skips if `x <= 0`.
            #[inline]
            pub fn update(&mut self, x: $ty, y: $ty) {
                if x > 0.0 as $ty {
                    #[allow(clippy::cast_possible_truncation)]
                    let ln_x = crate::math::ln(x as f64) as $ty;
                    self.inner.update(ln_x, y);
                }
            }

            /// Slope (coefficient of ln(x)), or `None` if not primed.
            #[must_use]
            pub fn slope(&self) -> Option<$ty> {
                self.inner.slope()
            }

            /// Intercept (constant term b), or `None` if not primed.
            #[must_use]
            pub fn intercept_value(&self) -> Option<$ty> {
                self.inner.intercept_value()
            }

            /// R² goodness of fit.
            #[must_use]
            pub fn r_squared(&self) -> Option<$ty> {
                self.inner.r_squared()
            }

            /// Predict `y = a · ln(x) + b`. Returns `None` if not primed or `x <= 0`.
            #[must_use]
            pub fn predict(&self, x: $ty) -> Option<$ty> {
                if x <= 0.0 as $ty {
                    return Option::None;
                }
                #[allow(clippy::cast_possible_truncation)]
                let ln_x = crate::math::ln(x as f64) as $ty;
                self.inner.predict(ln_x)
            }

            /// Number of accepted observations (x > 0).
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.inner.count()
            }

            #[inline]
            #[must_use]
            /// Whether enough data for a fit.
            pub fn is_primed(&self) -> bool {
                self.inner.is_primed()
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.inner.reset();
            }
        }

        impl Default for $name {
            #[inline]
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

impl_logarithmic_regression!(LogarithmicRegressionF64, LinearRegressionF64, f64);
impl_logarithmic_regression!(LogarithmicRegressionF32, LinearRegressionF32, f32);

// ============================================================================
// Power Law: y = a * x^b
// ============================================================================

macro_rules! impl_power_regression {
    ($name:ident, $inner:ident, $ty:ty) => {
        /// Online power law regression: `y = a · x^b`.
        ///
        /// Linearized as `ln(y) = ln(a) + b · ln(x)`. Observations with
        /// `x <= 0` or `y <= 0` are silently skipped.
        ///
        /// R² is measured in log-log space.
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use nexus_stats::", stringify!($name), ";")]
        ///
        #[doc = concat!("let mut r = ", stringify!($name), "::new();")]
        /// for x in 1..200 {
        #[doc = concat!("    let y = 4.0 as ", stringify!($ty), " * (x as ", stringify!($ty), ").powf(2.5);")]
        ///     r.update(x as _, y);
        /// }
        /// let exp = r.exponent().unwrap();
        /// assert!((exp - 2.5).abs() < 0.01);
        /// ```
        #[derive(Debug, Clone)]
        pub struct $name {
            inner: $inner,
        }

        impl $name {
            /// Creates a new empty power law regression.
            #[inline]
            #[must_use]
            pub fn new() -> Self {
                Self { inner: $inner::new() }
            }

            /// Feeds (x, y). Silently skips if `x <= 0` or `y <= 0`.
            #[inline]
            pub fn update(&mut self, x: $ty, y: $ty) {
                if x > 0.0 as $ty && y > 0.0 as $ty {
                    #[allow(clippy::cast_possible_truncation)]
                    let ln_x = crate::math::ln(x as f64) as $ty;
                    #[allow(clippy::cast_possible_truncation)]
                    let ln_y = crate::math::ln(y as f64) as $ty;
                    self.inner.update(ln_x, ln_y);
                }
            }

            /// Exponent (the power b), or `None` if not primed.
            #[must_use]
            pub fn exponent(&self) -> Option<$ty> {
                self.inner.slope()
            }

            /// Scale factor `a = e^(intercept)`, or `None` if not primed.
            #[must_use]
            pub fn scale(&self) -> Option<$ty> {
                self.inner.intercept_value().map(|v| {
                    #[allow(clippy::cast_possible_truncation)]
                    { crate::math::exp(v as f64) as $ty }
                })
            }

            /// R² in log-log space.
            #[must_use]
            pub fn r_squared(&self) -> Option<$ty> {
                self.inner.r_squared()
            }

            /// Predict `y = a · x^b`. Returns `None` if not primed or `x <= 0`.
            #[must_use]
            pub fn predict(&self, x: $ty) -> Option<$ty> {
                if x <= 0.0 as $ty {
                    return Option::None;
                }
                let intercept = self.inner.intercept_value()?;
                let slope = self.inner.slope()?;
                #[allow(clippy::cast_possible_truncation)]
                {
                    let a = crate::math::exp(intercept as f64);
                    let b = slope as f64;
                    let result = a * (x as f64).powf(b);
                    Option::Some(result as $ty)
                }
            }

            /// Number of accepted observations.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.inner.count()
            }

            #[inline]
            #[must_use]
            /// Whether enough data for a fit.
            pub fn is_primed(&self) -> bool {
                self.inner.is_primed()
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.inner.reset();
            }
        }

        impl Default for $name {
            #[inline]
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

impl_power_regression!(PowerRegressionF64, LinearRegressionF64, f64);
impl_power_regression!(PowerRegressionF32, LinearRegressionF32, f32);

// ============================================================================
// EW Transformed Variants
// ============================================================================

macro_rules! impl_ew_exponential_regression {
    ($name:ident, $builder:ident, $inner:ident, $ty:ty) => {
        /// Exponentially-weighted exponential regression: `y = a · e^(bx)`.
        #[derive(Debug, Clone)]
        pub struct $name {
            inner: $inner,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: Option<$ty>,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder { alpha: Option::None }
            }

            /// Feeds (x, y). Silently skips if `y <= 0`.
            #[inline]
            pub fn update(&mut self, x: $ty, y: $ty) {
                if y > 0.0 as $ty {
                    #[allow(clippy::cast_possible_truncation)]
                    let ln_y = crate::math::ln(y as f64) as $ty;
                    self.inner.update(x, ln_y);
                }
            }

            /// Growth/decay rate.
            #[must_use]
            pub fn growth_rate(&self) -> Option<$ty> {
                self.inner.slope()
            }

            /// Scale factor `a = e^(intercept)`.
            #[must_use]
            pub fn scale(&self) -> Option<$ty> {
                self.inner.intercept_value().map(|v| {
                    #[allow(clippy::cast_possible_truncation)]
                    { crate::math::exp(v as f64) as $ty }
                })
            }

            /// R² in log-space.
            #[must_use]
            pub fn r_squared(&self) -> Option<$ty> { self.inner.r_squared() }

            /// Predict `y = a · e^(bx)`.
            #[must_use]
            pub fn predict(&self, x: $ty) -> Option<$ty> {
                self.inner.predict(x).map(|ln_y| {
                    #[allow(clippy::cast_possible_truncation)]
                    { crate::math::exp(ln_y as f64) as $ty }
                })
            }

            /// Number of accepted observations.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 { self.inner.count() }

            #[inline]
            #[must_use]
            /// Whether primed.
            pub fn is_primed(&self) -> bool { self.inner.is_primed() }

            /// Reset.
            #[inline]
            pub fn reset(&mut self) { self.inner.reset(); }
        }

        impl $builder {
            /// Weight on new observation, in (0, 1).
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = Option::Some(alpha);
                self
            }

            /// Builds the estimator.
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let alpha = self.alpha.ok_or(crate::ConfigError::Missing("alpha"))?;
                let inner = $inner::builder()
                    .alpha(alpha)
                    .build()?;
                Ok($name { inner })
            }
        }
    };
}

impl_ew_exponential_regression!(
    EwExponentialRegressionF64, EwExponentialRegressionF64Builder,
    EwLinearRegressionF64, f64
);

macro_rules! impl_ew_logarithmic_regression {
    ($name:ident, $builder:ident, $inner:ident, $ty:ty) => {
        /// Exponentially-weighted logarithmic regression: `y = a · ln(x) + b`.
        #[derive(Debug, Clone)]
        pub struct $name {
            inner: $inner,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: Option<$ty>,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder { alpha: Option::None }
            }

            /// Feeds (x, y). Silently skips if `x <= 0`.
            #[inline]
            pub fn update(&mut self, x: $ty, y: $ty) {
                if x > 0.0 as $ty {
                    #[allow(clippy::cast_possible_truncation)]
                    let ln_x = crate::math::ln(x as f64) as $ty;
                    self.inner.update(ln_x, y);
                }
            }

            /// Slope (coefficient of ln(x)).
            #[must_use]
            pub fn slope(&self) -> Option<$ty> {
                self.inner.slope()
            }

            /// Intercept.
            #[must_use]
            pub fn intercept_value(&self) -> Option<$ty> {
                self.inner.intercept_value()
            }

            /// R².
            #[must_use]
            pub fn r_squared(&self) -> Option<$ty> { self.inner.r_squared() }

            /// Predict `y = a · ln(x) + b`.
            #[must_use]
            pub fn predict(&self, x: $ty) -> Option<$ty> {
                if x <= 0.0 as $ty { return Option::None; }
                #[allow(clippy::cast_possible_truncation)]
                let ln_x = crate::math::ln(x as f64) as $ty;
                self.inner.predict(ln_x)
            }

            #[inline]
            #[must_use]
            /// Count.
            pub fn count(&self) -> u64 { self.inner.count() }
            #[inline]
            #[must_use]
            /// Primed.
            pub fn is_primed(&self) -> bool { self.inner.is_primed() }
            /// Reset.
            #[inline]
            pub fn reset(&mut self) { self.inner.reset(); }
        }

        impl $builder {
            /// Alpha.
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = Option::Some(alpha);
                self
            }

            /// Build.
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let alpha = self.alpha.ok_or(crate::ConfigError::Missing("alpha"))?;
                let inner = $inner::builder()
                    .alpha(alpha)
                    .build()?;
                Ok($name { inner })
            }
        }
    };
}

impl_ew_logarithmic_regression!(
    EwLogarithmicRegressionF64, EwLogarithmicRegressionF64Builder,
    EwLinearRegressionF64, f64
);

macro_rules! impl_ew_power_regression {
    ($name:ident, $builder:ident, $inner:ident, $ty:ty) => {
        /// Exponentially-weighted power law regression: `y = a · x^b`.
        #[derive(Debug, Clone)]
        pub struct $name {
            inner: $inner,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: Option<$ty>,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder { alpha: Option::None }
            }

            /// Feeds (x, y). Silently skips if `x <= 0` or `y <= 0`.
            #[inline]
            pub fn update(&mut self, x: $ty, y: $ty) {
                if x > 0.0 as $ty && y > 0.0 as $ty {
                    #[allow(clippy::cast_possible_truncation)]
                    let ln_x = crate::math::ln(x as f64) as $ty;
                    #[allow(clippy::cast_possible_truncation)]
                    let ln_y = crate::math::ln(y as f64) as $ty;
                    self.inner.update(ln_x, ln_y);
                }
            }

            /// Exponent b.
            #[must_use]
            pub fn exponent(&self) -> Option<$ty> {
                self.inner.slope()
            }

            /// Scale `a = e^(intercept)`.
            #[must_use]
            pub fn scale(&self) -> Option<$ty> {
                self.inner.intercept_value().map(|v| {
                    #[allow(clippy::cast_possible_truncation)]
                    { crate::math::exp(v as f64) as $ty }
                })
            }

            /// R² in log-log space.
            #[must_use]
            pub fn r_squared(&self) -> Option<$ty> { self.inner.r_squared() }

            /// Predict `y = a · x^b`.
            #[must_use]
            pub fn predict(&self, x: $ty) -> Option<$ty> {
                if x <= 0.0 as $ty { return Option::None; }
                let intercept = self.inner.intercept_value()?;
                let slope = self.inner.slope()?;
                #[allow(clippy::cast_possible_truncation)]
                {
                    let a = crate::math::exp(intercept as f64);
                    let b = slope as f64;
                    let result = a * (x as f64).powf(b);
                    Option::Some(result as $ty)
                }
            }

            #[inline]
            #[must_use]
            /// Count.
            pub fn count(&self) -> u64 { self.inner.count() }
            #[inline]
            #[must_use]
            /// Primed.
            pub fn is_primed(&self) -> bool { self.inner.is_primed() }
            /// Reset.
            #[inline]
            pub fn reset(&mut self) { self.inner.reset(); }
        }

        impl $builder {
            /// Alpha.
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = Option::Some(alpha);
                self
            }

            /// Build.
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let alpha = self.alpha.ok_or(crate::ConfigError::Missing("alpha"))?;
                let inner = $inner::builder()
                    .alpha(alpha)
                    .build()?;
                Ok($name { inner })
            }
        }
    };
}

impl_ew_power_regression!(
    EwPowerRegressionF64, EwPowerRegressionF64Builder,
    EwLinearRegressionF64, f64
);

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Exponential: y = a * e^(bx)
    // =========================================================================

    #[test]
    fn exponential_exact_fit() {
        let mut r = ExponentialRegressionF64::new();
        for x in 0..100 {
            let xf = x as f64;
            let y = 2.0 * (0.05 * xf).exp();
            r.update(xf, y);
        }
        let rate = r.growth_rate().unwrap();
        assert!((rate - 0.05).abs() < 1e-8, "growth rate = {rate}");
        let scale = r.scale().unwrap();
        assert!((scale - 2.0).abs() < 1e-6, "scale = {scale}");
        assert!((r.r_squared().unwrap() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn exponential_predict() {
        let mut r = ExponentialRegressionF64::new();
        for x in 0..100 {
            let xf = x as f64;
            r.update(xf, 2.0 * (0.05 * xf).exp());
        }
        let y = r.predict(10.0).unwrap();
        let expected = 2.0 * (0.05 * 10.0_f64).exp();
        assert!((y - expected).abs() < 1e-4, "predict(10) = {y}");
    }

    #[test]
    fn exponential_skips_negative_y() {
        let mut r = ExponentialRegressionF64::new();
        r.update(1.0, -5.0); // skipped
        r.update(2.0, 0.0);  // skipped
        assert_eq!(r.count(), 0);
    }

    // =========================================================================
    // Logarithmic: y = a * ln(x) + b
    // =========================================================================

    #[test]
    fn logarithmic_exact_fit() {
        let mut r = LogarithmicRegressionF64::new();
        for x in 1..200 {
            let xf = x as f64;
            r.update(xf, 3.0 * xf.ln() + 1.0);
        }
        let slope = r.slope().unwrap();
        assert!((slope - 3.0).abs() < 1e-6, "slope = {slope}");
        let intercept = r.intercept_value().unwrap();
        assert!((intercept - 1.0).abs() < 1e-6, "intercept = {intercept}");
    }

    #[test]
    fn logarithmic_skips_negative_x() {
        let mut r = LogarithmicRegressionF64::new();
        r.update(-1.0, 5.0);
        r.update(0.0, 5.0);
        assert_eq!(r.count(), 0);
    }

    #[test]
    fn logarithmic_predict_negative_x_returns_none() {
        let mut r = LogarithmicRegressionF64::new();
        for x in 1..100 {
            r.update(x as f64, (x as f64).ln());
        }
        assert!(r.predict(-1.0).is_none());
    }

    // =========================================================================
    // Power law: y = a * x^b
    // =========================================================================

    #[test]
    fn power_exact_fit() {
        let mut r = PowerRegressionF64::new();
        for x in 1..200 {
            let xf = x as f64;
            r.update(xf, 4.0 * xf.powf(2.5));
        }
        let exp = r.exponent().unwrap();
        assert!((exp - 2.5).abs() < 1e-4, "exponent = {exp}");
        let scale = r.scale().unwrap();
        assert!((scale - 4.0).abs() < 0.1, "scale = {scale}");
    }

    #[test]
    fn power_skips_nonpositive() {
        let mut r = PowerRegressionF64::new();
        r.update(0.0, 5.0);
        r.update(1.0, -5.0);
        r.update(-1.0, 5.0);
        assert_eq!(r.count(), 0);
    }

    // =========================================================================
    // EW transformed variants
    // =========================================================================

    #[test]
    fn ew_exponential_basic() {
        let mut r = EwExponentialRegressionF64::builder()
            .alpha(0.05)
            .build()
            .unwrap();
        for x in 0..300 {
            let xf = x as f64;
            r.update(xf, 2.0 * (0.01 * xf).exp());
        }
        assert!(r.is_primed());
        assert!(r.growth_rate().is_some());
    }

    #[test]
    fn ew_logarithmic_basic() {
        let mut r = EwLogarithmicRegressionF64::builder()
            .alpha(0.05)
            .build()
            .unwrap();
        for x in 1..300 {
            r.update(x as f64, 2.0 * (x as f64).ln() + 5.0);
        }
        assert!(r.is_primed());
    }

    #[test]
    fn ew_power_basic() {
        let mut r = EwPowerRegressionF64::builder()
            .alpha(0.05)
            .build()
            .unwrap();
        for x in 1..300 {
            r.update(x as f64, 3.0 * (x as f64).powf(1.5));
        }
        assert!(r.is_primed());
    }

    // =========================================================================
    // f32 variants
    // =========================================================================

    #[test]
    fn f32_exponential() {
        let mut r = ExponentialRegressionF32::new();
        for x in 0..100u32 {
            let xf = x as f32;
            r.update(xf, 2.0 * (0.05 * xf).exp());
        }
        assert!(r.growth_rate().is_some());
    }

    #[test]
    fn f32_logarithmic() {
        let mut r = LogarithmicRegressionF32::new();
        for x in 1..100u32 {
            r.update(x as f32, 3.0 * (x as f32).ln() + 1.0);
        }
        assert!(r.slope().is_some());
    }

    #[test]
    fn f32_power() {
        let mut r = PowerRegressionF32::new();
        for x in 1..100u32 {
            r.update(x as f32, 4.0 * (x as f32).powf(2.0));
        }
        assert!(r.exponent().is_some());
    }

    // =========================================================================
    // Reset / Default
    // =========================================================================

    #[test]
    fn reset_all_transforms() {
        let mut exp = ExponentialRegressionF64::new();
        let mut log = LogarithmicRegressionF64::new();
        let mut pow = PowerRegressionF64::new();
        for x in 1..100 {
            exp.update(x as f64, (x as f64).exp());
            log.update(x as f64, (x as f64).ln());
            pow.update(x as f64, (x as f64).powi(2));
        }
        exp.reset();
        log.reset();
        pow.reset();
        assert_eq!(exp.count(), 0);
        assert_eq!(log.count(), 0);
        assert_eq!(pow.count(), 0);
    }

    #[test]
    fn defaults_are_empty() {
        assert_eq!(ExponentialRegressionF64::default().count(), 0);
        assert_eq!(LogarithmicRegressionF64::default().count(), 0);
        assert_eq!(PowerRegressionF64::default().count(), 0);
    }
}
