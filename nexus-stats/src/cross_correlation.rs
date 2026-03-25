// Checking var_product <= 0.0 is intentional: zero variance means
// correlation is undefined. This is exact, not approximate.
#![allow(clippy::float_cmp)]

// Online Cross-Correlation — Two-Stream, Multi-Lag
//
// Cross-correlation between stream A and stream B at lags 0..LAG-1.
// "Does A at time t-k correlate with B at time t?"
//
// Maintains a circular buffer for stream A's history, per-stream
// Welford accumulators, and per-lag cross-moment accumulators.
//
// r_AB(k) = C_AB(k) / sqrt(Var(A) * Var(B))

macro_rules! impl_cross_correlation {
    ($name:ident, $ty:ty) => {
        /// Online cross-correlation between two streams at multiple lags.
        ///
        /// Tracks the Pearson correlation between stream A (lagged by 0..LAG-1
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
        /// - O(LAG) per update, `16*LAG + 48` bytes state, zero allocation.
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use nexus_stats::", stringify!($name), ";")]
        ///
        /// // B = A shifted by 3 steps
        #[doc = concat!("let mut cc = ", stringify!($name), "::<10>::new();")]
        /// let a: Vec<f64> = (0..500).map(|i| (i as f64).sin()).collect();
        /// for i in 0..500 {
        ///     let b = if i >= 3 { a[i - 3] } else { 0.0 };
        #[doc = concat!("    cc.update(a[i] as ", stringify!($ty), ", b as ", stringify!($ty), ");")]
        /// }
        /// // Peak correlation should be near lag 3
        /// if let Some(peak) = cc.peak_lag() {
        ///     assert!((peak as i32 - 3).unsigned_abs() <= 2);
        /// }
        /// ```
        #[derive(Debug, Clone)]
        pub struct $name<const LAG: usize> {
            buffer_a: [$ty; LAG],
            head: usize,
            count: u64,
            mean_a: $ty,
            mean_b: $ty,
            m2_a: $ty,
            m2_b: $ty,
            cross_m: [$ty; LAG],
        }

        impl<const LAG: usize> $name<LAG> {
            const _ASSERT_LAG: () = assert!(LAG >= 1, "LAG must be at least 1");

            /// Creates a new empty cross-correlation tracker.
            #[inline]
            #[must_use]
            pub fn new() -> Self {
                #[allow(clippy::let_unit_value)]
                let () = Self::_ASSERT_LAG;
                Self {
                    buffer_a: [0.0 as $ty; LAG],
                    head: 0,
                    count: 0,
                    mean_a: 0.0 as $ty,
                    mean_b: 0.0 as $ty,
                    m2_a: 0.0 as $ty,
                    m2_b: 0.0 as $ty,
                    cross_m: [0.0 as $ty; LAG],
                }
            }

            /// Feeds paired observations from both streams.
            #[inline]
            pub fn update(&mut self, a: $ty, b: $ty) {
                self.count += 1;
                let n = self.count as $ty;

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

                // Lags 1..LAG-1: use buffered A values (approximate,
                // error is O(1/n) per step — fine for streaming)
                if self.count > 1 {
                    let filled = (self.count - 1).min(LAG as u64) as usize;
                    for k in 1..filled.min(LAG) {
                        let idx = (self.head + LAG - k) % LAG;
                        let a_lagged = self.buffer_a[idx];
                        self.cross_m[k] +=
                            (a_lagged - self.mean_a) * db_new;
                    }
                }

                // Store A in circular buffer
                self.buffer_a[self.head] = a;
                self.head = (self.head + 1) % LAG;
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
                if lag >= LAG {
                    return Option::None;
                }
                if self.count < (lag as u64 + 2) {
                    return Option::None;
                }
                let var_product = self.m2_a * self.m2_b;
                if var_product <= 0.0 as $ty {
                    return Option::None;
                }
                #[allow(clippy::cast_possible_truncation)]
                let denom = crate::math::sqrt(var_product as f64) as $ty;
                Option::Some(self.cross_m[lag] / denom)
            }

            /// The lag (0..LAG) with the strongest absolute correlation,
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
                let max_lag = (self.count - 1).min(LAG as u64) as usize;

                for k in 0..max_lag {
                    let abs_cm = if self.cross_m[k] < 0.0 as $ty {
                        -(self.cross_m[k])
                    } else {
                        self.cross_m[k]
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
                if lag >= LAG {
                    return Option::None;
                }
                if self.count < (lag as u64 + 2) {
                    return Option::None;
                }
                let n_pairs = (self.count - lag as u64) as $ty;
                Option::Some(self.cross_m[lag] / n_pairs)
            }

            /// Number of paired observations processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether enough data has been collected for all lags (>= LAG + 1).
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count > LAG as u64
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.buffer_a = [0.0 as $ty; LAG];
                self.head = 0;
                self.count = 0;
                self.mean_a = 0.0 as $ty;
                self.mean_b = 0.0 as $ty;
                self.m2_a = 0.0 as $ty;
                self.m2_b = 0.0 as $ty;
                self.cross_m = [0.0 as $ty; LAG];
            }
        }

        impl<const LAG: usize> Default for $name<LAG> {
            #[inline]
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

impl_cross_correlation!(CrossCorrelationF64, f64);
impl_cross_correlation!(CrossCorrelationF32, f32);

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Basic correctness
    // =========================================================================

    #[test]
    fn identical_streams_correlation_one() {
        let mut cc = CrossCorrelationF64::<1>::new();
        for i in 0..1000u64 {
            let x = i as f64;
            cc.update(x, x);
        }
        let r = cc.correlation(0).unwrap();
        assert!(
            (r - 1.0).abs() < 1e-6,
            "identical streams should correlate at 1.0, got {r}"
        );
    }

    #[test]
    fn opposite_streams_correlation_negative() {
        let mut cc = CrossCorrelationF64::<1>::new();
        for i in 0..1000u64 {
            let x = i as f64;
            cc.update(x, -x);
        }
        let r = cc.correlation(0).unwrap();
        assert!(
            (r - (-1.0)).abs() < 1e-6,
            "opposite streams should correlate at -1.0, got {r}"
        );
    }

    #[test]
    fn shifted_signal_peak_lag() {
        let mut cc = CrossCorrelationF64::<10>::new();
        let shift = 3;
        // A is a sine wave, B is A shifted by 3
        let a: Vec<f64> = (0..1000)
            .map(|i| ((i as f64) * 0.1).sin())
            .collect();
        for i in 0..1000 {
            let b = if i >= shift { a[i - shift] } else { 0.0 };
            cc.update(a[i], b);
        }
        let peak = cc.peak_lag().unwrap();
        assert!(
            (peak as i32 - shift as i32).unsigned_abs() <= 1,
            "peak lag should be near {shift}, got {peak}"
        );
    }

    #[test]
    fn lag0_matches_covariance_type() {
        // CrossCorrelation at lag 0 should match CovarianceF64's correlation
        let mut cc = CrossCorrelationF64::<1>::new();
        let mut cov = crate::CovarianceF64::new();

        for i in 0..500u64 {
            let x = i as f64;
            let y = x * 2.0 + 1.0;
            cc.update(x, y);
            cov.update(x, y);
        }

        let r_cc = cc.correlation(0).unwrap();
        let r_cov = cov.correlation().unwrap();
        assert!(
            (r_cc - r_cov).abs() < 0.01,
            "lag-0 cross-correlation ({r_cc}) should match covariance correlation ({r_cov})"
        );
    }

    // =========================================================================
    // Priming
    // =========================================================================

    #[test]
    fn not_primed_until_enough_samples() {
        let mut cc = CrossCorrelationF64::<5>::new();
        for i in 0..5 {
            cc.update(i as f64, i as f64);
            assert!(!cc.is_primed());
        }
        cc.update(5.0, 5.0);
        assert!(cc.is_primed());
    }

    #[test]
    fn lag_out_of_range_returns_none() {
        let mut cc = CrossCorrelationF64::<5>::new();
        for i in 0..20 {
            cc.update(i as f64, i as f64);
        }
        assert!(cc.correlation(5).is_none()); // LAG=5, max valid lag is 4
        assert!(cc.covariance(5).is_none());
    }

    // =========================================================================
    // Zero variance
    // =========================================================================

    #[test]
    fn zero_variance_returns_none() {
        let mut cc = CrossCorrelationF64::<1>::new();
        for _ in 0..100 {
            cc.update(42.0, 42.0);
        }
        assert!(cc.correlation(0).is_none());
    }

    // =========================================================================
    // Reset
    // =========================================================================

    #[test]
    fn reset_clears_state() {
        let mut cc = CrossCorrelationF64::<3>::new();
        for i in 0..100 {
            cc.update(i as f64, (i * 2) as f64);
        }
        cc.reset();
        assert_eq!(cc.count(), 0);
        assert!(!cc.is_primed());
    }

    // =========================================================================
    // f32 variant
    // =========================================================================

    #[test]
    fn f32_basic() {
        let mut cc = CrossCorrelationF32::<1>::new();
        for i in 0..200u32 {
            cc.update(i as f32, (i * 2) as f32);
        }
        let r = cc.correlation(0).unwrap();
        assert!(r > 0.9, "f32 perfect linear should be near 1.0, got {r}");
    }

    // =========================================================================
    // Default
    // =========================================================================

    #[test]
    fn default_is_empty() {
        let cc = CrossCorrelationF64::<5>::default();
        assert_eq!(cc.count(), 0);
    }
}
