// Online Polynomial Regression — Sufficient Statistics + Normal Equations
//
// Accumulates sums of powers of x and cross-products with y.
// Solves normal equations via Gaussian elimination at query time.
// O(degree) per update, O(degree³) per query (bounded, max 9×9).
//
// degree and intercept are runtime-configured via builder.

// Normal equations with sums-of-powers can accumulate large values.
#![allow(clippy::suboptimal_flops)]

macro_rules! impl_coefficients {
    ($name:ident, $ty:ty) => {
        /// Polynomial coefficients returned by regression queries.
        ///
        /// With intercept: `values[0]` = constant (a₀), `values[1]` = x coefficient (a₁), etc.
        /// Without intercept: `values[0]` = x¹ coefficient, `values[1]` = x² coefficient, etc.
        #[derive(Debug, Clone, Copy)]
        pub struct $name {
            pub(crate) values: [$ty; 9],
            pub(crate) len: usize,
        }

        impl $name {
            /// Coefficients as a slice.
            #[inline]
            #[must_use]
            pub fn as_slice(&self) -> &[$ty] {
                &self.values[..self.len]
            }

            /// Number of coefficients.
            #[inline]
            #[must_use]
            pub fn len(&self) -> usize {
                self.len
            }

            /// Whether there are no coefficients (should never happen in practice).
            #[inline]
            #[must_use]
            pub fn is_empty(&self) -> bool {
                self.len == 0
            }

            /// Get the i-th coefficient, or `None` if out of range.
            #[inline]
            #[must_use]
            pub fn get(&self, i: usize) -> Option<$ty> {
                if i < self.len {
                    Option::Some(self.values[i])
                } else {
                    Option::None
                }
            }
        }

        impl core::ops::Index<usize> for $name {
            type Output = $ty;
            #[inline]
            fn index(&self, i: usize) -> &$ty {
                assert!(
                    i < self.len,
                    "coefficient index {i} out of range (len={})",
                    self.len
                );
                &self.values[i]
            }
        }
    };
}

impl_coefficients!(CoefficientsF64, f64);
impl_coefficients!(CoefficientsF32, f32);

/// Gaussian elimination with partial pivoting on a fixed-size system.
/// Returns `true` if the solve succeeded (non-singular).
/// On success, `rhs` contains the solution.
macro_rules! impl_gauss_solve {
    ($fn_name:ident, $ty:ty) => {
        pub(crate) fn $fn_name(dim: usize, a: &mut [[$ty; 9]; 9], b: &mut [$ty; 9]) -> bool {
            for col in 0..dim {
                let mut max_row = col;
                let mut max_val = if a[col][col] < 0.0 as $ty {
                    -(a[col][col])
                } else {
                    a[col][col]
                };
                for row in (col + 1)..dim {
                    let v = if a[row][col] < 0.0 as $ty {
                        -(a[row][col])
                    } else {
                        a[row][col]
                    };
                    if v > max_val {
                        max_val = v;
                        max_row = row;
                    }
                }
                if max_val < 1e-14 as $ty {
                    return false;
                }
                if max_row != col {
                    a.swap(col, max_row);
                    b.swap(col, max_row);
                }
                for row in (col + 1)..dim {
                    let factor = a[row][col] / a[col][col];
                    for j in col..dim {
                        a[row][j] -= factor * a[col][j];
                    }
                    b[row] -= factor * b[col];
                }
            }
            for i in (0..dim).rev() {
                for j in (i + 1)..dim {
                    b[i] -= a[i][j] * b[j];
                }
                b[i] /= a[i][i];
            }
            true
        }
    };
}

impl_gauss_solve!(gauss_solve_f64, f64);
impl_gauss_solve!(gauss_solve_f32, f32);

macro_rules! impl_polynomial_regression {
    ($name:ident, $builder:ident, $coeff:ident, $solve_fn:ident, $ty:ty) => {
        /// Online polynomial regression via sufficient statistics.
        ///
        /// Accumulates sums of powers of x and cross-products with y.
        /// Solves the normal equations at query time via Gaussian elimination
        /// with partial pivoting.
        ///
        /// Degree and intercept are configured at construction via the builder.
        /// Supports degree 1 (linear) through 8 (octic).
        ///
        /// For best numerical stability with high-degree fits or large x ranges,
        /// center and scale your x values: `x_scaled = (x - x_mean) / x_std`.
        ///
        /// # Complexity
        /// - O(degree) per update, O(degree³) per coefficient query.
        /// - ~216 bytes state (f64), ~120 bytes (f32). Zero allocation.
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use nexus_stats::regression::", stringify!($name), ";")]
        ///
        /// // Fit y = x² - 3x + 2 (quadratic)
        #[doc = concat!("let mut r = ", stringify!($name), "::builder().degree(2).build().unwrap();")]
        /// for x in -50..50i64 {
        #[doc = concat!("    let xf = x as ", stringify!($ty), ";")]
        #[doc = concat!("    r.update(xf, xf * xf - 3.0 as ", stringify!($ty), " * xf + 2.0 as ", stringify!($ty), ");")]
        /// }
        /// let c = r.coefficients().unwrap();
        /// assert_eq!(c.len(), 3); // [constant, x, x²]
        /// ```
        #[derive(Debug, Clone)]
        pub struct $name {
            sum_x: [$ty; 17],
            sum_xy: [$ty; 9],
            sum_y2: $ty,
            count: u64,
            degree: usize,
            intercept: bool,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            degree: Option<usize>,
            intercept: bool,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    degree: Option::None,
                    intercept: true,
                }
            }

            /// System dimension.
            #[inline]
            fn dim(&self) -> usize {
                self.degree + self.intercept as usize
            }

            /// Configured polynomial degree.
            #[inline]
            #[must_use]
            pub fn degree(&self) -> usize {
                self.degree
            }

            /// Whether the intercept (constant term) is included.
            #[inline]
            #[must_use]
            pub fn has_intercept(&self) -> bool {
                self.intercept
            }

            /// Feeds an (x, y) observation.
            ///
            /// # Errors
            ///
            /// Returns `DataError::NotANumber` if either value is NaN, or
            /// `DataError::Infinite` if either value is infinite.
            #[inline]
            pub fn update(&mut self, x: $ty, y: $ty) -> Result<(), crate::DataError> {
                check_finite!(x);
                check_finite!(y);
                self.count += 1;
                self.sum_y2 += y * y;
                let mut x_pow = 1.0 as $ty;
                let max_pow = 2 * self.degree;
                for j in 0..=max_pow {
                    self.sum_x[j] += x_pow;
                    if j <= self.degree {
                        self.sum_xy[j] += x_pow * y;
                    }
                    x_pow *= x;
                }
                Ok(())
            }

            /// Solve for polynomial coefficients, or `None` if underdetermined.
            ///
            /// Returns coefficients indexed from constant term (a₀) to highest
            /// power (aₖ). Without intercept, the first coefficient corresponds
            /// to x¹.
            #[must_use]
            pub fn coefficients(&self) -> Option<$coeff> {
                let dim = self.dim();
                if (self.count as usize) < dim {
                    return Option::None;
                }

                let mut a = [[0.0 as $ty; 9]; 9];
                let mut b = [0.0 as $ty; 9];
                let offset: usize = if self.intercept { 0 } else { 1 };

                for i in 0..dim {
                    for j in 0..dim {
                        a[i][j] = self.sum_x[i + j + 2 * offset];
                    }
                    b[i] = self.sum_xy[i + offset];
                }

                if !$solve_fn(dim, &mut a, &mut b) {
                    return Option::None;
                }

                let mut coeffs = $coeff {
                    values: [0.0 as $ty; 9],
                    len: dim,
                };
                for i in 0..dim {
                    coeffs.values[i] = b[i];
                }
                Option::Some(coeffs)
            }

            /// R² goodness of fit, or `None` if not enough data.
            ///
            /// With intercept: centered R² = 1 - SS_res/SS_tot.
            /// Without intercept: uncentered R² = 1 - SS_res/Σy².
            #[must_use]
            pub fn r_squared(&self) -> Option<$ty> {
                let coeffs = self.coefficients()?;
                let n = self.count as $ty;
                let dim = self.dim();
                let offset: usize = if self.intercept { 0 } else { 1 };

                let mut beta_dot_rhs = 0.0 as $ty;
                for i in 0..dim {
                    beta_dot_rhs += coeffs.values[i] * self.sum_xy[i + offset];
                }
                let ss_res = self.sum_y2 - beta_dot_rhs;

                let ss_tot = if self.intercept {
                    let sum_y = self.sum_xy[0];
                    self.sum_y2 - sum_y * sum_y / n
                } else {
                    self.sum_y2
                };

                if ss_tot <= 0.0 as $ty {
                    return Option::None;
                }

                Option::Some(1.0 as $ty - ss_res / ss_tot)
            }

            /// Predict y for a given x using current coefficients.
            #[must_use]
            pub fn predict(&self, x: $ty) -> Option<$ty> {
                let coeffs = self.coefficients()?;
                let dim = self.dim();

                let mut y = 0.0 as $ty;
                let mut x_pow = if self.intercept { 1.0 as $ty } else { x };
                for i in 0..dim {
                    y += coeffs.values[i] * x_pow;
                    x_pow *= x;
                }
                Option::Some(y)
            }

            /// Number of observations processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether enough data has been collected to solve (count >= dim).
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                (self.count as usize) >= self.dim()
            }

            /// Resets to empty state. Degree and intercept unchanged.
            #[inline]
            pub fn reset(&mut self) {
                self.sum_x = [0.0 as $ty; 17];
                self.sum_xy = [0.0 as $ty; 9];
                self.sum_y2 = 0.0 as $ty;
                self.count = 0;
            }
        }

        impl $builder {
            /// Polynomial degree (1..=8). Required.
            #[inline]
            #[must_use]
            pub fn degree(mut self, degree: usize) -> Self {
                self.degree = Option::Some(degree);
                self
            }

            /// Whether to include the constant term. Default: `true`.
            ///
            /// `false` forces the fit through the origin.
            #[inline]
            #[must_use]
            pub fn intercept(mut self, intercept: bool) -> Self {
                self.intercept = intercept;
                self
            }

            /// Builds the regression estimator.
            ///
            /// # Errors
            ///
            /// Returns `ConfigError::Missing` if degree not set.
            /// Returns `ConfigError::Invalid` if degree not in 1..=8.
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let degree = self.degree
                    .ok_or(crate::ConfigError::Missing("degree"))?;
                if degree < 1 || degree > 8 {
                    return Err(crate::ConfigError::Invalid(
                        "degree must be in 1..=8",
                    ));
                }

                Ok($name {
                    sum_x: [0.0 as $ty; 17],
                    sum_xy: [0.0 as $ty; 9],
                    sum_y2: 0.0 as $ty,
                    count: 0,
                    degree,
                    intercept: self.intercept,
                })
            }
        }
    };
}

impl_polynomial_regression!(
    PolynomialRegressionF64,
    PolynomialRegressionF64Builder,
    CoefficientsF64,
    gauss_solve_f64,
    f64
);
impl_polynomial_regression!(
    PolynomialRegressionF32,
    PolynomialRegressionF32Builder,
    CoefficientsF32,
    gauss_solve_f32,
    f32
);

#[cfg(test)]
mod tests {
    use super::*;

    fn quadratic() -> PolynomialRegressionF64 {
        PolynomialRegressionF64::builder()
            .degree(2)
            .build()
            .unwrap()
    }

    fn cubic() -> PolynomialRegressionF64 {
        PolynomialRegressionF64::builder()
            .degree(3)
            .build()
            .unwrap()
    }

    // =========================================================================
    // Quadratic regression (y = ax² + bx + c)
    // =========================================================================

    #[test]
    fn quadratic_exact_fit() {
        let mut r = quadratic();
        for x in -50..50 {
            let xf = x as f64;
            r.update(xf, xf * xf - 3.0 * xf + 2.0).unwrap();
        }
        let c = r.coefficients().unwrap();
        assert!((c[0] - 2.0).abs() < 1e-6, "c0 = {}", c[0]);
        assert!((c[1] - (-3.0)).abs() < 1e-6, "c1 = {}", c[1]);
        assert!((c[2] - 1.0).abs() < 1e-6, "c2 = {}", c[2]);
    }

    #[test]
    fn quadratic_predict() {
        let mut r = quadratic();
        for x in -50..50 {
            let xf = x as f64;
            r.update(xf, xf * xf).unwrap();
        }
        let y = r.predict(10.0).unwrap();
        assert!((y - 100.0).abs() < 1e-4, "predict(10) = {y}");
    }

    // =========================================================================
    // Cubic
    // =========================================================================

    #[test]
    fn cubic_exact_fit() {
        let mut r = cubic();
        for x in -20..20 {
            let xf = x as f64;
            let y = 0.5 * xf * xf * xf - 2.0 * xf * xf + xf - 1.0;
            r.update(xf, y).unwrap();
        }
        let c = r.coefficients().unwrap();
        assert!((c[0] - (-1.0)).abs() < 1e-4, "c0 = {}", c[0]);
        assert!((c[1] - 1.0).abs() < 1e-4, "c1 = {}", c[1]);
        assert!((c[2] - (-2.0)).abs() < 1e-4, "c2 = {}", c[2]);
        assert!((c[3] - 0.5).abs() < 1e-4, "c3 = {}", c[3]);
    }

    // =========================================================================
    // Builder
    // =========================================================================

    #[test]
    fn builder_degree_4() {
        let mut r = PolynomialRegressionF64::builder()
            .degree(4)
            .build()
            .unwrap();
        for x in -20..20 {
            let xf = x as f64;
            r.update(xf, xf * xf * xf * xf).unwrap();
        }
        assert!(r.is_primed());
        assert_eq!(r.degree(), 4);
        assert!(r.has_intercept());
    }

    #[test]
    fn builder_no_intercept() {
        let mut r = PolynomialRegressionF64::builder()
            .degree(1)
            .intercept(false)
            .build()
            .unwrap();
        for x in 1..100 {
            r.update(x as f64, 5.0 * x as f64).unwrap();
        }
        let c = r.coefficients().unwrap();
        assert_eq!(c.len(), 1);
        assert!((c[0] - 5.0).abs() < 1e-8, "slope = {}", c[0]);
    }

    #[test]
    fn builder_rejects_degree_0() {
        assert!(
            PolynomialRegressionF64::builder()
                .degree(0)
                .build()
                .is_err()
        );
    }

    #[test]
    fn builder_rejects_degree_9() {
        assert!(
            PolynomialRegressionF64::builder()
                .degree(9)
                .build()
                .is_err()
        );
    }

    #[test]
    fn builder_rejects_missing_degree() {
        assert!(PolynomialRegressionF64::builder().build().is_err());
    }

    // =========================================================================
    // R²
    // =========================================================================

    #[test]
    fn r_squared_perfect() {
        let mut r = quadratic();
        for x in -50..50 {
            let xf = x as f64;
            r.update(xf, xf * xf - 3.0 * xf + 2.0).unwrap();
        }
        let r2 = r.r_squared().unwrap();
        assert!((r2 - 1.0).abs() < 1e-10, "R² = {r2}");
    }

    // =========================================================================
    // Priming / edge cases
    // =========================================================================

    #[test]
    fn quadratic_needs_3_points() {
        let mut r = quadratic();
        r.update(1.0, 1.0).unwrap();
        r.update(2.0, 4.0).unwrap();
        assert!(!r.is_primed());
        r.update(3.0, 9.0).unwrap();
        assert!(r.is_primed());
    }

    // =========================================================================
    // Heterogeneous storage
    // =========================================================================

    #[test]
    fn different_degrees_same_type() {
        let models: [PolynomialRegressionF64; 2] = [
            PolynomialRegressionF64::builder()
                .degree(2)
                .build()
                .unwrap(),
            PolynomialRegressionF64::builder()
                .degree(3)
                .build()
                .unwrap(),
        ];
        assert_eq!(models[0].degree(), 2);
        assert_eq!(models[1].degree(), 3);
    }

    // =========================================================================
    // Reset
    // =========================================================================

    #[test]
    fn reset_clears_state() {
        let mut r = quadratic();
        for x in 0..100 {
            r.update(x as f64, x as f64).unwrap();
        }
        r.reset();
        assert_eq!(r.count(), 0);
        assert!(r.coefficients().is_none());
        assert_eq!(r.degree(), 2);
    }

    // =========================================================================
    // f32 variant
    // =========================================================================

    #[test]
    fn f32_quadratic() {
        let mut r = PolynomialRegressionF32::builder()
            .degree(2)
            .build()
            .unwrap();
        for x in -20..20i32 {
            let xf = x as f32;
            r.update(xf, xf * xf).unwrap();
        }
        assert!(r.coefficients().is_some());
    }

    // =========================================================================
    // Coefficients struct
    // =========================================================================

    #[test]
    fn coefficients_indexing() {
        let mut r = quadratic();
        for x in -10..10 {
            let xf = x as f64;
            r.update(xf, xf * xf).unwrap();
        }
        let c = r.coefficients().unwrap();
        assert_eq!(c.len(), 3);
        assert!(!c.is_empty());
        assert!(c.get(0).is_some());
        assert!(c.get(3).is_none());
        assert_eq!(c.as_slice().len(), 3);
    }

    #[test]
    fn rejects_nan_and_inf() {
        let mut r = quadratic();
        assert_eq!(r.update(f64::NAN, 1.0), Err(crate::DataError::NotANumber));
        assert_eq!(
            r.update(1.0, f64::INFINITY),
            Err(crate::DataError::Infinite)
        );
        assert_eq!(r.count(), 0);
    }
}
