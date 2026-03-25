#![allow(clippy::suboptimal_flops)]

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec;

macro_rules! impl_rls_filter {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Recursive Least Squares adaptive filter.
        ///
        /// Tracks linear relationships between feature vectors and a scalar
        /// target using a recursive update of the covariance matrix. Converges
        /// faster than LMS at the cost of O(d²) per update. The forgetting
        /// factor controls how quickly old observations are downweighted.
        ///
        /// # Use Cases
        /// - Online linear regression with fast convergence
        /// - System identification with non-stationary parameters
        /// - Adaptive noise cancellation
        ///
        /// # Complexity
        /// O(d²) per update, heap-allocated weight vector and covariance matrix.
        #[derive(Debug, Clone)]
        pub struct $name {
            weights: Box<[$ty]>,
            p_matrix: Box<[$ty]>,
            scratch_px: Box<[$ty]>,
            scratch_k: Box<[$ty]>,
            forgetting_factor: $ty,
            initial_covariance: $ty,
            dims: usize,
            count: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            dimensions: Option<usize>,
            forgetting_factor: $ty,
            initial_covariance: $ty,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    dimensions: Option::None,
                    forgetting_factor: 1.0 as $ty,
                    initial_covariance: 1000.0 as $ty,
                }
            }

            /// Computes the dot product w·x (predicted output).
            ///
            /// # Panics
            /// Panics if `features.len() != self.dimensions()`.
            #[inline]
            #[must_use]
            pub fn predict(&self, features: &[$ty]) -> $ty {
                assert_eq!(
                    features.len(),
                    self.dims,
                    "feature length {} != dimensions {}",
                    features.len(),
                    self.dims,
                );
                let mut sum = 0.0 as $ty;
                for i in 0..self.dims {
                    sum += self.weights[i] * features[i];
                }
                sum
            }

            /// Updates weights using the Sherman-Morrison rank-1 update.
            ///
            /// Computes the Kalman gain, updates weights by the prediction
            /// error, and updates the inverse covariance matrix.
            ///
            /// # Panics
            /// Panics if `features.len() != self.dimensions()`.
            #[inline]
            pub fn update(&mut self, features: &[$ty], target: $ty) {
                assert_eq!(
                    features.len(),
                    self.dims,
                    "feature length {} != dimensions {}",
                    features.len(),
                    self.dims,
                );
                let d = self.dims;
                let lambda = self.forgetting_factor;

                // px[i] = Σ_j P[i][j] * x[j]
                for i in 0..d {
                    let mut sum = 0.0 as $ty;
                    for j in 0..d {
                        sum += self.p_matrix[i * d + j] * features[j];
                    }
                    self.scratch_px[i] = sum;
                }

                // xpx = Σ x[i] * px[i]
                let mut xpx = 0.0 as $ty;
                for i in 0..d {
                    xpx += features[i] * self.scratch_px[i];
                }

                // k[i] = px[i] / (lambda + xpx)
                // Epsilon floor prevents NaN if P degrades numerically.
                let denom = (lambda + xpx).max(<$ty>::EPSILON);
                for i in 0..d {
                    self.scratch_k[i] = self.scratch_px[i] / denom;
                }

                // error = target - w·x
                let mut prediction = 0.0 as $ty;
                for i in 0..d {
                    prediction += self.weights[i] * features[i];
                }
                let error = target - prediction;

                // w += k * error
                for i in 0..d {
                    self.weights[i] += self.scratch_k[i] * error;
                }

                // P[i][j] = (P[i][j] - k[i] * px[j]) / lambda
                for i in 0..d {
                    for j in 0..d {
                        self.p_matrix[i * d + j] =
                            (self.p_matrix[i * d + j] - self.scratch_k[i] * self.scratch_px[j])
                                / lambda;
                    }
                }

                self.count += 1;
            }

            /// Returns the current weight vector.
            #[inline]
            #[must_use]
            pub fn weights(&self) -> &[$ty] {
                &self.weights
            }

            /// Returns the number of dimensions.
            #[inline]
            #[must_use]
            pub fn dimensions(&self) -> usize {
                self.dims
            }

            /// Returns the forgetting factor.
            #[inline]
            #[must_use]
            pub fn forgetting_factor(&self) -> $ty {
                self.forgetting_factor
            }

            /// Returns the number of updates performed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Zeros weights and resets covariance to initial state.
            #[inline]
            pub fn reset(&mut self) {
                self.weights.fill(0.0 as $ty);
                self.scratch_px.fill(0.0 as $ty);
                self.scratch_k.fill(0.0 as $ty);
                let d = self.dims;
                let delta = self.initial_covariance;
                for i in 0..d {
                    for j in 0..d {
                        self.p_matrix[i * d + j] = if i == j { delta } else { 0.0 as $ty };
                    }
                }
                self.count = 0;
            }
        }

        impl $builder {
            /// Sets the number of input dimensions (required, >= 1).
            #[inline]
            #[must_use]
            pub fn dimensions(mut self, dims: usize) -> Self {
                self.dimensions = Option::Some(dims);
                self
            }

            /// Sets the forgetting factor (default 1.0, must be in (0, 1]).
            ///
            /// Values less than 1.0 downweight older observations exponentially.
            /// A value of 1.0 gives equal weight to all observations (standard RLS).
            #[inline]
            #[must_use]
            pub fn forgetting_factor(mut self, lambda: $ty) -> Self {
                self.forgetting_factor = lambda;
                self
            }

            /// Sets the initial covariance diagonal (default 1000.0, must be > 0).
            ///
            /// The initial covariance matrix P is set to `delta * I`.
            /// Larger values mean less confidence in initial weights.
            #[inline]
            #[must_use]
            pub fn initial_covariance(mut self, delta: $ty) -> Self {
                self.initial_covariance = delta;
                self
            }

            /// Builds the filter. Returns an error if parameters are missing or invalid.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let dims = self
                    .dimensions
                    .ok_or(crate::ConfigError::Missing("dimensions"))?;
                let lambda = self.forgetting_factor;
                let delta = self.initial_covariance;

                if dims < 1 {
                    return Err(crate::ConfigError::Invalid(
                        "dimensions must be >= 1",
                    ));
                }
                if lambda <= 0.0 as $ty || lambda > 1.0 as $ty {
                    return Err(crate::ConfigError::Invalid(
                        "forgetting_factor must be in (0, 1]",
                    ));
                }
                if delta <= 0.0 as $ty {
                    return Err(crate::ConfigError::Invalid(
                        "initial_covariance must be positive",
                    ));
                }

                // Initialize P = delta * I
                let mut p = vec![0.0 as $ty; dims * dims].into_boxed_slice();
                for i in 0..dims {
                    p[i * dims + i] = delta;
                }

                Ok($name {
                    weights: vec![0.0 as $ty; dims].into_boxed_slice(),
                    p_matrix: p,
                    scratch_px: vec![0.0 as $ty; dims].into_boxed_slice(),
                    scratch_k: vec![0.0 as $ty; dims].into_boxed_slice(),
                    forgetting_factor: lambda,
                    initial_covariance: delta,
                    dims,
                    count: 0,
                })
            }
        }
    };
}

impl_rls_filter!(RlsFilterF64, RlsFilterF64Builder, f64);
impl_rls_filter!(RlsFilterF32, RlsFilterF32Builder, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_linear_relationship() {
        // y = 2*x1 + 3*x2
        let mut filter = RlsFilterF64::builder()
            .dimensions(2)
            .build()
            .unwrap();

        for i in 0..200 {
            let x1 = (i as f64 * 0.7).sin();
            let x2 = (i as f64 * 1.3).cos();
            let target = 2.0 * x1 + 3.0 * x2;
            filter.update(&[x1, x2], target);
        }

        let w = filter.weights();
        assert!(
            (w[0] - 2.0).abs() < 0.01,
            "w[0] = {}, expected ~2.0",
            w[0]
        );
        assert!(
            (w[1] - 3.0).abs() < 0.01,
            "w[1] = {}, expected ~3.0",
            w[1]
        );
    }

    #[test]
    fn forgetting_adapts_to_change() {
        let mut filter = RlsFilterF64::builder()
            .dimensions(1)
            .forgetting_factor(0.95)
            .build()
            .unwrap();

        // Learn y = 2*x
        for i in 0..200 {
            let x = (i as f64 * 0.5).sin();
            filter.update(&[x], 2.0 * x);
        }

        let w_before = filter.weights()[0];
        assert!(
            (w_before - 2.0).abs() < 0.1,
            "w = {w_before}, expected ~2.0"
        );

        // Switch to y = 5*x
        for i in 0..200 {
            let x = (i as f64 * 0.5).sin();
            filter.update(&[x], 5.0 * x);
        }

        let w_after = filter.weights()[0];
        assert!(
            (w_after - 5.0).abs() < 0.5,
            "w = {w_after}, expected ~5.0 after adaptation"
        );
    }

    #[test]
    fn covariance_decreases() {
        let mut filter = RlsFilterF64::builder()
            .dimensions(1)
            .initial_covariance(1000.0)
            .build()
            .unwrap();

        filter.update(&[1.0], 2.0);
        filter.update(&[2.0], 4.0);
        filter.update(&[3.0], 6.0);

        // The diagonal of P should be much smaller than initial 1000.0
        // We can't access P directly, but we can verify convergence speed
        // is much faster than LMS (RLS converges in ~d samples for noiseless data).
        let w = filter.weights();
        assert!(
            (w[0] - 2.0).abs() < 0.01,
            "RLS should converge quickly, w = {}",
            w[0]
        );
    }

    #[test]
    fn predict_matches_dot_product() {
        let mut filter = RlsFilterF64::builder()
            .dimensions(3)
            .build()
            .unwrap();

        filter.update(&[1.0, 0.0, 0.0], 5.0);
        filter.update(&[0.0, 1.0, 0.0], 3.0);
        filter.update(&[0.0, 0.0, 1.0], 7.0);

        let features = [2.0, 4.0, 6.0];
        let w = filter.weights();
        let expected = w[0] * 2.0 + w[1] * 4.0 + w[2] * 6.0;
        let prediction = filter.predict(&features);
        assert!(
            (prediction - expected).abs() < 1e-12,
            "predict={prediction}, expected={expected}"
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut filter = RlsFilterF64::builder()
            .dimensions(2)
            .build()
            .unwrap();

        filter.update(&[1.0, 2.0], 5.0);
        assert!(filter.count() > 0);
        assert!(filter.weights().iter().any(|&w| w != 0.0));

        filter.reset();
        assert_eq!(filter.count(), 0);
        assert!(filter.weights().iter().all(|&w| w == 0.0));
    }

    #[test]
    #[should_panic(expected = "feature length")]
    fn dimension_mismatch_predict() {
        let filter = RlsFilterF64::builder()
            .dimensions(3)
            .build()
            .unwrap();

        filter.predict(&[1.0, 2.0]);
    }

    #[test]
    #[should_panic(expected = "feature length")]
    fn dimension_mismatch_update() {
        let mut filter = RlsFilterF64::builder()
            .dimensions(3)
            .build()
            .unwrap();

        filter.update(&[1.0], 5.0);
    }

    #[test]
    fn builder_rejects_zero_dimensions() {
        let result = RlsFilterF64::builder()
            .dimensions(0)
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn builder_rejects_invalid_forgetting_factor() {
        assert!(RlsFilterF64::builder()
            .dimensions(2)
            .forgetting_factor(0.0)
            .build()
            .is_err());

        assert!(RlsFilterF64::builder()
            .dimensions(2)
            .forgetting_factor(1.5)
            .build()
            .is_err());

        assert!(RlsFilterF64::builder()
            .dimensions(2)
            .forgetting_factor(-0.1)
            .build()
            .is_err());
    }

    #[test]
    fn builder_rejects_negative_covariance() {
        let result = RlsFilterF64::builder()
            .dimensions(2)
            .initial_covariance(-1.0)
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn f32_basic() {
        let mut filter = RlsFilterF32::builder()
            .dimensions(2)
            .build()
            .unwrap();

        for i in 0..200 {
            let x1 = (i as f32 * 0.7).sin();
            let x2 = (i as f32 * 1.3).cos();
            let target = 2.0 * x1 + 3.0 * x2;
            filter.update(&[x1, x2], target);
        }

        let w = filter.weights();
        assert!(
            (w[0] - 2.0).abs() < 0.1,
            "w[0] = {}, expected ~2.0",
            w[0]
        );
        assert!(
            (w[1] - 3.0).abs() < 0.1,
            "w[1] = {}, expected ~3.0",
            w[1]
        );
    }
}
