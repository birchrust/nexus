use crate::math::MulAdd;
macro_rules! impl_covariance {
    ($name:ident, $ty:ty) => {
        /// Online covariance and Pearson correlation between two signals.
        ///
        /// Uses Welford-style numerically stable single-pass computation.
        /// Supports Chan's merge for parallel aggregation.
        ///
        /// # Use Cases
        /// - "Are these two signals moving together?"
        /// - Latency correlation across venues
        /// - Co-movement detection
        #[derive(Debug, Clone)]
        pub struct $name {
            count: u64,
            mean_x: $ty,
            mean_y: $ty,
            m2_x: $ty,
            m2_y: $ty,
            co_moment: $ty,
        }

        impl $name {
            /// Creates a new empty accumulator.
            #[inline]
            #[must_use]
            pub const fn new() -> Self {
                Self {
                    count: 0,
                    mean_x: 0.0 as $ty,
                    mean_y: 0.0 as $ty,
                    m2_x: 0.0 as $ty,
                    m2_y: 0.0 as $ty,
                    co_moment: 0.0 as $ty,
                }
            }

            /// Feeds a paired sample.
            #[inline]
            pub fn update(&mut self, x: $ty, y: $ty) {
                self.count += 1;
                let n = self.count as $ty;

                let dx = x - self.mean_x;
                let dy = y - self.mean_y;

                self.mean_x += dx / n;
                self.mean_y += dy / n;

                // Use the NEW mean_x but OLD mean_y for the co-moment update
                let dx2 = x - self.mean_x;
                self.co_moment += dx * (y - self.mean_y);

                // Welford M2 updates
                self.m2_x += dx * dx2;
                let dy2 = y - self.mean_y;
                self.m2_y += dy * dy2;
            }

            /// Number of paired samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Mean of X, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn mean_x(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.mean_x)
                }
            }

            /// Mean of Y, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn mean_y(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.mean_y)
                }
            }

            /// Sample covariance (N-1 denominator), or `None` if < 2 samples.
            #[inline]
            #[must_use]
            pub fn covariance(&self) -> Option<$ty> {
                if self.count < 2 {
                    Option::None
                } else {
                    Option::Some(self.co_moment / (self.count - 1) as $ty)
                }
            }

            /// Pearson correlation coefficient, or `None` if < 2 samples.
            ///
            /// Returns a value in [-1, 1]. Returns `None` if either variable
            /// has zero variance (undefined correlation).
            #[cfg(any(feature = "std", feature = "libm"))]
            #[inline]
            #[must_use]
            pub fn correlation(&self) -> Option<$ty> {
                if self.count < 2 {
                    return Option::None;
                }
                let var_product = self.m2_x * self.m2_y;
                if var_product <= 0.0 as $ty {
                    return Option::None;
                }
                let r = self.co_moment / crate::math::sqrt(var_product as f64) as $ty;
                Option::Some(r)
            }

            /// Merges another accumulator into this one (Chan's algorithm).
            #[inline]
            pub fn merge(&mut self, other: &Self) {
                if other.count == 0 {
                    return;
                }
                if self.count == 0 {
                    *self = other.clone();
                    return;
                }

                let combined = self.count + other.count;
                let dx = other.mean_x - self.mean_x;
                let dy = other.mean_y - self.mean_y;
                let weight = self.count as $ty * other.count as $ty / combined as $ty;

                let new_mean_x =
                    (dx * other.count as $ty).fma(1.0 as $ty / combined as $ty, self.mean_x);
                let new_mean_y =
                    (dy * other.count as $ty).fma(1.0 as $ty / combined as $ty, self.mean_y);

                self.co_moment += (dx * dy).fma(weight, other.co_moment);
                self.m2_x += (dx * dx).fma(weight, other.m2_x);
                self.m2_y += (dy * dy).fma(weight, other.m2_y);
                self.mean_x = new_mean_x;
                self.mean_y = new_mean_y;
                self.count = combined;
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                *self = Self::new();
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

impl_covariance!(CovarianceF64, f64);
impl_covariance!(CovarianceF32, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let c = CovarianceF64::new();
        assert_eq!(c.count(), 0);
        assert!(c.covariance().is_none());
        assert!(c.correlation().is_none());
    }

    #[test]
    fn perfect_positive_correlation() {
        let mut c = CovarianceF64::new();
        for i in 0..100 {
            c.update(i as f64, i as f64 * 2.0);
        }
        let r = c.correlation().unwrap();
        assert!(
            (r - 1.0).abs() < 1e-10,
            "perfect positive should be 1.0, got {r}"
        );
    }

    #[test]
    fn perfect_negative_correlation() {
        let mut c = CovarianceF64::new();
        for i in 0..100 {
            c.update(i as f64, -(i as f64));
        }
        let r = c.correlation().unwrap();
        assert!(
            (r + 1.0).abs() < 1e-10,
            "perfect negative should be -1.0, got {r}"
        );
    }

    #[test]
    fn known_covariance() {
        let mut c = CovarianceF64::new();
        // Simple dataset: (1,2), (2,4), (3,6)
        c.update(1.0, 2.0);
        c.update(2.0, 4.0);
        c.update(3.0, 6.0);

        let cov = c.covariance().unwrap();
        // Cov(X, 2X) = 2 * Var(X). Var([1,2,3]) = 1.0. So cov = 2.0.
        assert!(
            (cov - 2.0).abs() < 1e-10,
            "covariance should be 2.0, got {cov}"
        );
    }

    #[test]
    fn merge_matches_single() {
        let data_x: [f64; 8] = [1.0, 3.0, 5.0, 7.0, 2.0, 4.0, 6.0, 8.0];
        let data_y: [f64; 8] = [2.0, 6.0, 10.0, 14.0, 4.0, 8.0, 12.0, 16.0];

        let mut single = CovarianceF64::new();
        for i in 0..8 {
            single.update(data_x[i], data_y[i]);
        }

        let mut a = CovarianceF64::new();
        let mut b = CovarianceF64::new();
        for i in 0..4 {
            a.update(data_x[i], data_y[i]);
        }
        for i in 4..8 {
            b.update(data_x[i], data_y[i]);
        }
        a.merge(&b);

        assert_eq!(a.count(), single.count());
        assert!((a.covariance().unwrap() - single.covariance().unwrap()).abs() < 1e-10);
        assert!((a.correlation().unwrap() - single.correlation().unwrap()).abs() < 1e-10);
    }

    #[test]
    fn reset_clears() {
        let mut c = CovarianceF64::new();
        c.update(1.0, 2.0);
        c.update(3.0, 4.0);
        c.reset();
        assert_eq!(c.count(), 0);
    }

    #[test]
    fn f32_basic() {
        let mut c = CovarianceF32::new();
        c.update(1.0, 2.0);
        c.update(2.0, 4.0);
        assert!(c.covariance().is_some());
    }

    #[test]
    fn default_is_empty() {
        let c = CovarianceF64::default();
        assert_eq!(c.count(), 0);
    }

    #[test]
    fn zero_variance_returns_none_correlation() {
        let mut c = CovarianceF64::new();
        c.update(5.0, 1.0);
        c.update(5.0, 2.0); // X has zero variance
        assert!(c.correlation().is_none());
    }
}
