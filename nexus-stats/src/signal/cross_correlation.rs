// Checking var_product <= 0.0 is intentional: zero variance means
// correlation is undefined. This is exact, not approximate.
#![allow(clippy::float_cmp)]

// Online Cross-Correlation — Two-Stream, Multi-Lag
//
// Cross-correlation between stream A and stream B at lags 0..lag-1.
// "Does A at time t-k correlate with B at time t?"
//
// Maintains a circular buffer for stream A's history, per-stream
// Welford accumulators, and per-lag cross-moment accumulators.
//
// r_AB(k) = C_AB(k) / sqrt(Var(A) * Var(B))

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec;

macro_rules! impl_cross_correlation {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Online cross-correlation between two streams at multiple lags.
        ///
        /// Tracks the Pearson correlation between stream A (lagged by 0..lag-1
        /// steps) and stream B at the current time. Uses Welford-style
        /// running accumulators for numerical stability.
        ///
        /// "Does A at time t-k predict B at time t?"
        ///
        /// # Use Cases
        /// - Lead/lag detection between two price series
        /// - Identifying which signal predicts another
        /// - Measuring coupling strength between two metrics
        ///
        /// # Complexity
        /// - O(lag) per update, heap-allocated buffers.
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use nexus_stats::signal::", stringify!($name), ";")]
        ///
        /// // B = A shifted by 3 steps
        #[doc = concat!("let mut cc = ", stringify!($name), "::builder().lag(10).build().unwrap();")]
        /// let a: Vec<f64> = (0..500).map(|i| (i as f64).sin()).collect();
        /// for i in 0..500 {
        ///     let b = if i >= 3 { a[i - 3] } else { 0.0 };
        #[doc = concat!("    cc.update(a[i] as ", stringify!($ty), ", b as ", stringify!($ty), ").unwrap();")]
        /// }
        /// // Peak correlation should be near lag 3
        /// if let Some(peak) = cc.peak_lag() {
        ///     assert!((peak as i32 - 3).unsigned_abs() <= 2);
        /// }
        /// ```
        #[derive(Debug, Clone)]
        pub struct $name {
            buffer_a: Box<[$ty]>,
            cross_m: Box<[$ty]>,
            lag: usize,
            head: usize,
            count: u64,
            mean_a: $ty,
            mean_b: $ty,
            m2_a: $ty,
            m2_b: $ty,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            lag: Option<usize>,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    lag: Option::None,
                }
            }

            /// Feeds paired observations from both streams.
            ///
            /// # Errors
            ///
            /// Returns `DataError::NotANumber` if either value is NaN, or
            /// `DataError::Infinite` if either value is infinite.
            #[inline]
            pub fn update(&mut self, a: $ty, b: $ty) -> Result<(), crate::DataError> {
                check_finite!(a);
                check_finite!(b);
                self.count += 1;
                let n = self.count as $ty;
                let lag = self.lag;

                // Capture old means for Welford co-moment
                let da_old = a - self.mean_a;
                let db_old = b - self.mean_b;

                // Welford mean + variance updates
                self.mean_a += da_old / n;
                let da_new = a - self.mean_a;
                self.m2_a += da_old * da_new;

                self.mean_b += db_old / n;
                let db_new = b - self.mean_b;
                self.m2_b += db_old * db_new;

                // Lag 0: exact Welford co-moment (old_delta_a * new_residual_b)
                self.cross_m[0] += da_old * db_new;

                // Lags 1..lag-1: use buffered A values (approximate,
                // error is O(1/n) per step — fine for streaming)
                if self.count > 1 {
                    let filled = (self.count - 1).min(lag as u64) as usize;
                    for k in 1..filled.min(lag) {
                        let idx = (self.head + lag - k) % lag;
                        let a_lagged = self.buffer_a[idx];
                        self.cross_m[k] +=
                            (a_lagged - self.mean_a) * db_new;
                    }
                }

                // Store A in circular buffer
                self.buffer_a[self.head] = a;
                self.head = (self.head + 1) % lag;
                Ok(())
            }

            /// Cross-correlation at the given lag, or `None` if not primed.
            ///
            /// Returns the Pearson correlation between A(t-lag) and B(t).
            /// Values in \[-1, 1\]. Returns `None` if either stream has zero
            /// variance.
            #[cfg(any(feature = "std", feature = "libm"))]
            #[inline]
            #[must_use]
            pub fn correlation(&self, lag: usize) -> Option<$ty> {
                if lag >= self.lag {
                    return Option::None;
                }
                if self.count < (lag as u64 + 2) {
                    return Option::None;
                }
                let var_product = self.m2_a * self.m2_b;
                if var_product <= 0.0 as $ty {
                    return Option::None;
                }
                // Lag 0: cross_m and m2_a/m2_b have the same number of
                // contributing samples — no scaling needed.
                // Lags > 0: cross_m[lag] has (count - lag) pairs while
                // m2 has (count - 1) samples. Scale to normalize.
                let scale = if lag == 0 {
                    1.0 as $ty
                } else {
                    let n_pairs = (self.count - lag as u64) as $ty;
                    let n_samples = (self.count - 1) as $ty;
                    n_samples / n_pairs
                };
                #[allow(clippy::cast_possible_truncation)]
                let denom = crate::math::sqrt(var_product as f64) as $ty;
                Option::Some(self.cross_m[lag] * scale / denom)
            }

            /// The lag (0..max_lag) with the strongest absolute correlation,
            /// or `None` if not primed.
            #[cfg(any(feature = "std", feature = "libm"))]
            #[inline]
            #[must_use]
            pub fn peak_lag(&self) -> Option<usize> {
                if self.count < 2 {
                    return Option::None;
                }
                let var_product = self.m2_a * self.m2_b;
                if var_product <= 0.0 as $ty {
                    return Option::None;
                }

                let mut best_lag = 0;
                let mut best_abs = 0.0 as $ty;
                let max_lag = (self.count - 1).min(self.lag as u64) as usize;
                let n_samples = (self.count - 1) as $ty;

                for k in 0..max_lag {
                    let normalized = if k == 0 {
                        self.cross_m[k]
                    } else {
                        let n_pairs = (self.count - k as u64) as $ty;
                        self.cross_m[k] * n_samples / n_pairs
                    };
                    let abs_cm = if normalized < 0.0 as $ty {
                        -normalized
                    } else {
                        normalized
                    };
                    if abs_cm > best_abs {
                        best_abs = abs_cm;
                        best_lag = k;
                    }
                }

                Option::Some(best_lag)
            }

            /// Raw cross-covariance at the given lag, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn covariance(&self, lag: usize) -> Option<$ty> {
                if lag >= self.lag {
                    return Option::None;
                }
                if self.count < (lag as u64 + 2) {
                    return Option::None;
                }
                let n_pairs = (self.count - lag as u64) as $ty;
                Option::Some(self.cross_m[lag] / n_pairs)
            }

            /// The configured maximum lag.
            #[inline]
            #[must_use]
            pub fn lag(&self) -> usize {
                self.lag
            }

            /// Number of paired observations processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether enough data has been collected for all lags (> lag).
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count > self.lag as u64
            }

            /// Resets to empty state. Configuration and buffer allocations preserved.
            #[inline]
            pub fn reset(&mut self) {
                self.buffer_a.fill(0.0 as $ty);
                self.cross_m.fill(0.0 as $ty);
                self.head = 0;
                self.count = 0;
                self.mean_a = 0.0 as $ty;
                self.mean_b = 0.0 as $ty;
                self.m2_a = 0.0 as $ty;
                self.m2_b = 0.0 as $ty;
            }
        }

        impl $builder {
            /// Sets the maximum lag (required, >= 1).
            ///
            /// The tracker computes cross-correlation at lags 0..lag-1.
            #[inline]
            #[must_use]
            pub fn lag(mut self, lag: usize) -> Self {
                self.lag = Option::Some(lag);
                self
            }

            /// Builds the cross-correlation tracker.
            ///
            /// # Errors
            /// Returns `ConfigError` if lag is missing or < 1.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let lag = self.lag.ok_or(crate::ConfigError::Missing("lag"))?;
                if lag < 1 {
                    return Err(crate::ConfigError::Invalid("lag must be >= 1"));
                }
                Ok($name {
                    buffer_a: vec![0.0 as $ty; lag].into_boxed_slice(),
                    cross_m: vec![0.0 as $ty; lag].into_boxed_slice(),
                    lag,
                    head: 0,
                    count: 0,
                    mean_a: 0.0 as $ty,
                    mean_b: 0.0 as $ty,
                    m2_a: 0.0 as $ty,
                    m2_b: 0.0 as $ty,
                })
            }
        }
    };
}

impl_cross_correlation!(CrossCorrelationF64, CrossCorrelationF64Builder, f64);
impl_cross_correlation!(CrossCorrelationF32, CrossCorrelationF32Builder, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_streams_correlation_one() {
        let mut cc = CrossCorrelationF64::builder().lag(1).build().unwrap();
        for i in 0..1000u64 {
            let x = i as f64;
            cc.update(x, x).unwrap();
        }
        let r = cc.correlation(0).unwrap();
        assert!(
            (r - 1.0).abs() < 1e-6,
            "identical streams should correlate at 1.0, got {r}"
        );
    }

    #[test]
    fn opposite_streams_correlation_negative() {
        let mut cc = CrossCorrelationF64::builder().lag(1).build().unwrap();
        for i in 0..1000u64 {
            let x = i as f64;
            cc.update(x, -x).unwrap();
        }
        let r = cc.correlation(0).unwrap();
        assert!(
            (r - (-1.0)).abs() < 1e-6,
            "opposite streams should correlate at -1.0, got {r}"
        );
    }

    #[test]
    fn shifted_signal_peak_lag() {
        let mut cc = CrossCorrelationF64::builder().lag(10).build().unwrap();
        let shift = 3;
        let a: Vec<f64> = (0..1000).map(|i| ((i as f64) * 0.1).sin()).collect();
        for i in 0..1000 {
            let b = if i >= shift { a[i - shift] } else { 0.0 };
            cc.update(a[i], b).unwrap();
        }
        let peak = cc.peak_lag().unwrap();
        assert!(
            (peak as i32 - shift as i32).unsigned_abs() <= 1,
            "peak lag should be near {shift}, got {peak}"
        );
    }

    #[test]
    fn lag0_matches_covariance_type() {
        let mut cc = CrossCorrelationF64::builder().lag(1).build().unwrap();
        let mut cov = crate::statistics::CovarianceF64::new();

        for i in 0..500u64 {
            let x = i as f64;
            let y = x * 2.0 + 1.0;
            cc.update(x, y).unwrap();
            cov.update(x, y);
        }

        let r_cc = cc.correlation(0).unwrap();
        let r_cov = cov.correlation().unwrap();
        assert!(
            (r_cc - r_cov).abs() < 0.01,
            "lag-0 cross-correlation ({r_cc}) should match covariance correlation ({r_cov})"
        );
    }

    #[test]
    fn not_primed_until_enough_samples() {
        let mut cc = CrossCorrelationF64::builder().lag(5).build().unwrap();
        for i in 0..5 {
            cc.update(i as f64, i as f64).unwrap();
            assert!(!cc.is_primed());
        }
        cc.update(5.0, 5.0).unwrap();
        assert!(cc.is_primed());
    }

    #[test]
    fn lag_out_of_range_returns_none() {
        let mut cc = CrossCorrelationF64::builder().lag(5).build().unwrap();
        for i in 0..20 {
            cc.update(i as f64, i as f64).unwrap();
        }
        assert!(cc.correlation(5).is_none()); // lag=5, max valid lag index is 4
        assert!(cc.covariance(5).is_none());
    }

    #[test]
    fn zero_variance_returns_none() {
        let mut cc = CrossCorrelationF64::builder().lag(1).build().unwrap();
        for _ in 0..100 {
            cc.update(42.0, 42.0).unwrap();
        }
        assert!(cc.correlation(0).is_none());
    }

    #[test]
    fn reset_clears_state() {
        let mut cc = CrossCorrelationF64::builder().lag(3).build().unwrap();
        for i in 0..100 {
            cc.update(i as f64, (i * 2) as f64).unwrap();
        }
        cc.reset();
        assert_eq!(cc.count(), 0);
        assert!(!cc.is_primed());
    }

    #[test]
    fn lag_accessor() {
        let cc = CrossCorrelationF64::builder().lag(7).build().unwrap();
        assert_eq!(cc.lag(), 7);
    }

    #[test]
    fn f32_basic() {
        let mut cc = CrossCorrelationF32::builder().lag(1).build().unwrap();
        for i in 0..200u32 {
            cc.update(i as f32, (i * 2) as f32).unwrap();
        }
        let r = cc.correlation(0).unwrap();
        assert!(r > 0.9, "f32 perfect linear should be near 1.0, got {r}");
    }

    #[test]
    fn rejects_nan_and_inf() {
        let mut cc = CrossCorrelationF64::builder().lag(1).build().unwrap();
        assert_eq!(cc.update(f64::NAN, 1.0), Err(crate::DataError::NotANumber));
        assert_eq!(
            cc.update(1.0, f64::INFINITY),
            Err(crate::DataError::Infinite)
        );
        assert_eq!(cc.count(), 0);
    }

    #[test]
    fn builder_requires_lag() {
        let result = CrossCorrelationF64::builder().build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("lag"))));
    }

    #[test]
    fn builder_rejects_zero_lag() {
        let result = CrossCorrelationF64::builder().lag(0).build();
        assert!(result.is_err());
    }
}
