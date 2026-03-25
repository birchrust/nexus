// Checking m2 == 0.0 is intentional: zero variance means all samples
// are identical, and correlation is undefined. This is exact, not approximate.
#![allow(clippy::float_cmp)]

// Online Autocorrelation at Fixed Lag
//
// Maintains a circular buffer of size LAG for delayed values, plus
// Welford-style accumulators for variance and the cross-moment
// between x(t) and x(t-LAG).
//
// r(k) = cross_m / m2 — the 1/N normalization cancels.

macro_rules! impl_autocorrelation_float {
    ($name:ident, $ty:ty) => {
        /// Online autocorrelation at a fixed lag.
        ///
        /// Maintains a circular buffer of `LAG` previous values and computes
        /// the autocorrelation coefficient between x(t) and x(t-LAG) using
        /// Welford-style running accumulators.
        ///
        /// # Use Cases
        /// - "Is this signal trending or mean-reverting?" (positive vs negative lag-1)
        /// - Detecting periodicity at a known lag
        /// - Stationarity monitoring
        ///
        /// # Complexity
        /// - O(1) per update, `8*LAG + 32` bytes state, zero allocation.
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use nexus_stats::", stringify!($name), ";")]
        ///
        /// // Strongly periodic signal: lag-1 autocorrelation of alternating values
        #[doc = concat!("let mut ac = ", stringify!($name), "::<1>::new();")]
        /// for i in 0..200u64 {
        #[doc = concat!("    ac.update(if i % 2 == 0 { 1.0 as ", stringify!($ty), " } else { -1.0 as ", stringify!($ty), " });")]
        /// }
        /// let r = ac.correlation().unwrap();
        /// assert!(r < -0.9, "alternating signal should have negative lag-1 autocorrelation");
        /// ```
        #[derive(Debug, Clone)]
        pub struct $name<const LAG: usize> {
            buffer: [$ty; LAG],
            head: usize,
            count: u64,
            mean: $ty,
            m2: $ty,
            cross_m: $ty,
        }

        impl<const LAG: usize> $name<LAG> {
            const _ASSERT_LAG: () = assert!(LAG >= 1, "LAG must be at least 1");

            /// Creates a new empty autocorrelation tracker.
            #[inline]
            #[must_use]
            pub fn new() -> Self {
                #[allow(clippy::let_unit_value)]
                let () = Self::_ASSERT_LAG;
                Self {
                    buffer: [0.0 as $ty; LAG],
                    head: 0,
                    count: 0,
                    mean: 0.0 as $ty,
                    m2: 0.0 as $ty,
                    cross_m: 0.0 as $ty,
                }
            }

            /// Feeds a sample.
            #[inline]
            pub fn update(&mut self, sample: $ty) {
                self.count += 1;

                // Welford update for running mean and variance
                let delta = sample - self.mean;
                self.mean += delta / self.count as $ty;
                let delta2 = sample - self.mean;
                self.m2 += delta * delta2;

                // Cross-moment: accumulate (x_new - mean)(x_lagged - mean)
                // once we have at least LAG+1 observations
                if self.count > LAG as u64 {
                    let x_lagged = self.buffer[self.head];
                    self.cross_m +=
                        (sample - self.mean) * (x_lagged - self.mean);
                }

                // Store in circular buffer
                self.buffer[self.head] = sample;
                self.head = (self.head + 1) % LAG;
            }

            /// Autocorrelation coefficient in \[-1, 1\], or `None` if fewer
            /// than `LAG + 2` samples.
            ///
            /// Defined as γ(k)/γ(0) where γ(k) is the autocovariance at lag k
            /// and γ(0) is the variance. Returns `None` if variance is zero.
            #[inline]
            #[must_use]
            pub fn correlation(&self) -> Option<$ty> {
                if self.count < (LAG as u64 + 2) {
                    return Option::None;
                }
                if self.m2 == 0.0 as $ty {
                    return Option::None;
                }
                // cross_m accumulated over (count - LAG) pairs,
                // m2 accumulated over (count - 1) samples.
                // Normalize both to get comparable per-observation values.
                let n_pairs = (self.count - LAG as u64) as $ty;
                let n_samples = (self.count - 1) as $ty;
                Option::Some(self.cross_m * n_samples / (self.m2 * n_pairs))
            }

            /// Raw autocovariance at the configured lag, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn covariance(&self) -> Option<$ty> {
                if self.count < (LAG as u64 + 2) {
                    return Option::None;
                }
                let n_pairs = (self.count - LAG as u64) as $ty;
                Option::Some(self.cross_m / n_pairs)
            }

            /// Number of observations processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether enough data has been collected (>= LAG + 2).
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count >= LAG as u64 + 2
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.buffer = [0.0 as $ty; LAG];
                self.head = 0;
                self.count = 0;
                self.mean = 0.0 as $ty;
                self.m2 = 0.0 as $ty;
                self.cross_m = 0.0 as $ty;
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

macro_rules! impl_autocorrelation_int {
    ($name:ident, $input:ty) => {
        /// Online autocorrelation at a fixed lag (integer input variant).
        ///
        /// Accepts integer samples, accumulates internally in f64.
        /// All query methods return f64.
        ///
        /// # Complexity
        /// - O(1) per update, `8*LAG + 32` bytes state, zero allocation.
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use nexus_stats::", stringify!($name), ";")]
        ///
        #[doc = concat!("let mut ac = ", stringify!($name), "::<1>::new();")]
        #[doc = concat!("for i in 0..200 { ac.update(if i % 2 == 0 { 1 as ", stringify!($input), " } else { -1 as ", stringify!($input), " }); }")]
        /// let r = ac.correlation().unwrap();
        /// assert!(r < -0.9);
        /// ```
        #[derive(Debug, Clone)]
        pub struct $name<const LAG: usize> {
            buffer: [f64; LAG],
            head: usize,
            count: u64,
            mean: f64,
            m2: f64,
            cross_m: f64,
        }

        impl<const LAG: usize> $name<LAG> {
            const _ASSERT_LAG: () = assert!(LAG >= 1, "LAG must be at least 1");

            /// Creates a new empty autocorrelation tracker.
            #[inline]
            #[must_use]
            pub fn new() -> Self {
                #[allow(clippy::let_unit_value)]
                let () = Self::_ASSERT_LAG;
                Self {
                    buffer: [0.0; LAG],
                    head: 0,
                    count: 0,
                    mean: 0.0,
                    m2: 0.0,
                    cross_m: 0.0,
                }
            }

            /// Feeds a sample.
            #[inline]
            pub fn update(&mut self, sample: $input) {
                #[allow(clippy::cast_lossless, clippy::cast_possible_truncation)]
                let x = sample as f64;
                self.count += 1;

                let delta = x - self.mean;
                self.mean += delta / self.count as f64;
                let delta2 = x - self.mean;
                self.m2 += delta * delta2;

                if self.count > LAG as u64 {
                    let x_lagged = self.buffer[self.head];
                    self.cross_m += (x - self.mean) * (x_lagged - self.mean);
                }

                self.buffer[self.head] = x;
                self.head = (self.head + 1) % LAG;
            }

            /// Autocorrelation coefficient in \[-1, 1\], or `None` if fewer
            /// than `LAG + 2` samples or variance is zero.
            #[inline]
            #[must_use]
            pub fn correlation(&self) -> Option<f64> {
                if self.count < (LAG as u64 + 2) {
                    return Option::None;
                }
                if self.m2 == 0.0 {
                    return Option::None;
                }
                Option::Some(self.cross_m / self.m2)
            }

            /// Raw autocovariance at the configured lag, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn covariance(&self) -> Option<f64> {
                if self.count < (LAG as u64 + 2) {
                    return Option::None;
                }
                let n_pairs = (self.count - LAG as u64) as f64;
                Option::Some(self.cross_m / n_pairs)
            }

            /// Number of observations processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether enough data has been collected (>= LAG + 2).
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count >= LAG as u64 + 2
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.buffer = [0.0; LAG];
                self.head = 0;
                self.count = 0;
                self.mean = 0.0;
                self.m2 = 0.0;
                self.cross_m = 0.0;
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

impl_autocorrelation_float!(AutocorrelationF64, f64);
impl_autocorrelation_float!(AutocorrelationF32, f32);
impl_autocorrelation_int!(AutocorrelationI64, i64);
impl_autocorrelation_int!(AutocorrelationI32, i32);

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Basic correctness
    // =========================================================================

    #[test]
    fn alternating_negative_lag1() {
        let mut ac = AutocorrelationF64::<1>::new();
        for i in 0..1000u64 {
            ac.update(if i % 2 == 0 { 1.0 } else { -1.0 });
        }
        let r = ac.correlation().unwrap();
        assert!(r < -0.9, "alternating should be strongly negative, got {r}");
    }

    #[test]
    fn trending_positive_lag1() {
        let mut ac = AutocorrelationF64::<1>::new();
        // Monotonically increasing — consecutive values are close
        for i in 0..1000u64 {
            ac.update(i as f64);
        }
        let r = ac.correlation().unwrap();
        assert!(r > 0.9, "monotone trend should have positive lag-1, got {r}");
    }

    #[test]
    fn lag10_periodic() {
        let mut ac = AutocorrelationF64::<10>::new();
        // Period-10 signal: strong autocorrelation at lag 10
        for i in 0..2000u64 {
            ac.update((i % 10) as f64);
        }
        let r = ac.correlation().unwrap();
        assert!(
            r > 0.8,
            "period-10 signal should correlate at lag 10, got {r}"
        );
    }

    #[test]
    fn constant_input_zero_variance() {
        let mut ac = AutocorrelationF64::<1>::new();
        for _ in 0..100 {
            ac.update(42.0);
        }
        // Zero variance → correlation undefined
        assert!(ac.correlation().is_none());
    }

    // =========================================================================
    // Priming
    // =========================================================================

    #[test]
    fn not_primed_until_lag_plus_2() {
        let mut ac = AutocorrelationF64::<5>::new();
        for i in 0..6 {
            ac.update(i as f64);
            assert!(!ac.is_primed(), "should not be primed at count {}", i + 1);
        }
        ac.update(6.0);
        assert!(ac.is_primed(), "should be primed at count 7 (LAG+2)");
    }

    // =========================================================================
    // Covariance query
    // =========================================================================

    #[test]
    fn covariance_sign_matches_correlation() {
        let mut ac = AutocorrelationF64::<1>::new();
        for i in 0..500u64 {
            ac.update(i as f64);
        }
        let corr = ac.correlation().unwrap();
        let cov = ac.covariance().unwrap();
        assert!(
            corr.signum() == cov.signum(),
            "corr={corr}, cov={cov} — signs should match"
        );
    }

    // =========================================================================
    // Reset
    // =========================================================================

    #[test]
    fn reset_clears_state() {
        let mut ac = AutocorrelationF64::<1>::new();
        for i in 0..100 {
            ac.update(i as f64);
        }
        ac.reset();
        assert_eq!(ac.count(), 0);
        assert!(!ac.is_primed());
        assert!(ac.correlation().is_none());
    }

    // =========================================================================
    // Integer variants
    // =========================================================================

    #[test]
    fn i64_alternating() {
        let mut ac = AutocorrelationI64::<1>::new();
        for i in 0..1000i64 {
            ac.update(if i % 2 == 0 { 100 } else { -100 });
        }
        let r = ac.correlation().unwrap();
        assert!(r < -0.9, "i64 alternating got {r}");
    }

    #[test]
    fn i32_trending() {
        let mut ac = AutocorrelationI32::<1>::new();
        for i in 0..500i32 {
            ac.update(i);
        }
        let r = ac.correlation().unwrap();
        assert!(r > 0.9, "i32 trending got {r}");
    }

    // =========================================================================
    // f32 variant
    // =========================================================================

    #[test]
    fn f32_basic() {
        let mut ac = AutocorrelationF32::<1>::new();
        for i in 0..200u32 {
            ac.update(i as f32);
        }
        assert!(ac.correlation().is_some());
    }

    // =========================================================================
    // Default
    // =========================================================================

    #[test]
    fn default_is_empty() {
        let ac = AutocorrelationF64::<1>::default();
        assert_eq!(ac.count(), 0);
    }
}
