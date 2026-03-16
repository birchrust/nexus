use crate::math::MulAdd;
macro_rules! impl_welford {
    ($name:ident, $ty:ty) => {
        /// Welford — Online mean, variance, and standard deviation.
        ///
        /// Numerically stable single-pass computation using Welford's algorithm.
        /// No catastrophic cancellation. Supports merging partial results via
        /// Chan's algorithm for parallel aggregation.
        ///
        /// # Use Cases
        /// - Running statistics on latency, throughput, PnL
        /// - Z-score computation (combine with EMA for baseline)
        /// - Input to adaptive thresholds
        #[derive(Debug, Clone)]
        pub struct $name {
            count: u64,
            mean: $ty,
            m2: $ty,
        }

        impl $name {
            /// Creates a new empty accumulator.
            #[inline]
            #[must_use]
            pub const fn new() -> Self {
                Self {
                    count: 0,
                    mean: 0.0 as $ty,
                    m2: 0.0 as $ty,
                }
            }

            /// Creates an accumulator pre-loaded from known statistics.
            ///
            /// `m2` is the sum of squared deviations from the mean
            /// (`variance * (count - 1)` for sample variance).
            #[inline]
            #[must_use]
            pub const fn from_parts(count: u64, mean: $ty, m2: $ty) -> Self {
                Self { count, mean, m2 }
            }

            /// Feeds a sample.
            #[inline]
            pub fn update(&mut self, sample: $ty) {
                self.count += 1;
                let delta = sample - self.mean;
                self.mean += delta / self.count as $ty;
                let delta2 = sample - self.mean;
                self.m2 += delta * delta2;
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Running mean, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn mean(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.mean)
                }
            }

            /// Sample variance (N-1 denominator), or `None` if < 2 samples.
            #[inline]
            #[must_use]
            pub fn variance(&self) -> Option<$ty> {
                if self.count < 2 {
                    Option::None
                } else {
                    Option::Some(self.m2 / (self.count - 1) as $ty)
                }
            }

            /// Population variance (N denominator), or `None` if empty.
            #[inline]
            #[must_use]
            pub fn population_variance(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.m2 / self.count as $ty)
                }
            }

            /// Sample standard deviation, or `None` if < 2 samples.
            #[inline]
            #[must_use]
            #[cfg(any(feature = "std", feature = "libm"))]
            pub fn std_dev(&self) -> Option<$ty> {
                self.variance().map(|v| {
                    #[allow(clippy::cast_possible_truncation)]
                    { crate::math::sqrt(v as f64) as $ty }
                })
            }

            /// Merges another accumulator into this one (Chan's algorithm).
            ///
            /// After merging, `self` contains the statistics of the combined
            /// dataset. The other accumulator is unchanged.
            #[inline]
            pub fn merge(&mut self, other: &Self) {
                if other.count == 0 {
                    return;
                }
                if self.count == 0 {
                    self.count = other.count;
                    self.mean = other.mean;
                    self.m2 = other.m2;
                    return;
                }

                let combined_count = self.count + other.count;
                let delta = other.mean - self.mean;
                let weight = other.count as $ty / combined_count as $ty;
                let new_mean = delta.fma(weight, self.mean);
                let cross = self.count as $ty * other.count as $ty / combined_count as $ty;
                let new_m2 = delta.fma(delta * cross, self.m2 + other.m2);

                self.count = combined_count;
                self.mean = new_mean;
                self.m2 = new_m2;
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.count = 0;
                self.mean = 0.0 as $ty;
                self.m2 = 0.0 as $ty;
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

impl_welford!(WelfordF64, f64);
impl_welford!(WelfordF32, f32);

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Basic correctness
    // =========================================================================

    #[test]
    fn empty_returns_none() {
        let w = WelfordF64::new();
        assert_eq!(w.count(), 0);
        assert!(w.mean().is_none());
        assert!(w.variance().is_none());
        assert!(w.population_variance().is_none());
        assert!(w.std_dev().is_none());
    }

    #[test]
    fn single_sample() {
        let mut w = WelfordF64::new();
        w.update(42.0);

        assert_eq!(w.count(), 1);
        assert_eq!(w.mean(), Some(42.0));
        // Variance needs at least 2 samples
        assert!(w.variance().is_none());
        // Population variance with 1 sample is 0
        assert_eq!(w.population_variance(), Some(0.0));
    }

    #[test]
    fn known_values() {
        let mut w = WelfordF64::new();

        // Dataset: [2, 4, 4, 4, 5, 5, 7, 9]
        for &x in &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            w.update(x);
        }

        assert_eq!(w.count(), 8);

        // Mean = 40/8 = 5.0
        let mean = w.mean().unwrap();
        assert!((mean - 5.0).abs() < 1e-10, "mean should be 5.0, got {mean}");

        // Population variance = 4.0
        let pop_var = w.population_variance().unwrap();
        assert!((pop_var - 4.0).abs() < 1e-10, "pop variance should be 4.0, got {pop_var}");

        // Sample variance = 32/7 ≈ 4.571428
        let var = w.variance().unwrap();
        assert!((var - 32.0 / 7.0).abs() < 1e-10, "sample variance should be 32/7, got {var}");

        // Std dev = sqrt(32/7) ≈ 2.138
        let sd = w.std_dev().unwrap();
        assert!((sd - (32.0_f64 / 7.0).sqrt()).abs() < 1e-6, "std dev got {sd}");
    }

    #[test]
    fn two_samples() {
        let mut w = WelfordF64::new();
        w.update(10.0);
        w.update(20.0);

        assert_eq!(w.count(), 2);
        assert!((w.mean().unwrap() - 15.0).abs() < 1e-10);
        // Sample variance of [10, 20] = 50.0
        assert!((w.variance().unwrap() - 50.0).abs() < 1e-10);
        // Pop variance = 25.0
        assert!((w.population_variance().unwrap() - 25.0).abs() < 1e-10);
    }

    // =========================================================================
    // Numerical stability
    // =========================================================================

    #[test]
    fn numerical_stability_large_offset() {
        // Classic failure case for naive variance: large mean, small deltas
        let mut w = WelfordF64::new();
        let base = 1e8;

        for i in 0..1000 {
            w.update((i as f64).fma(0.001, base));
        }

        let var = w.variance().unwrap();
        // Variance of 0, 0.001, 0.002, ..., 0.999 ≈ 0.08341...
        // (doesn't matter exactly — just check it's positive and reasonable)
        assert!(var > 0.0, "variance should be positive, got {var}");
        assert!(var < 1.0, "variance should be small, got {var}");
    }

    // =========================================================================
    // Merge (Chan's algorithm)
    // =========================================================================

    #[test]
    fn merge_empty_into_empty() {
        let mut a = WelfordF64::new();
        let b = WelfordF64::new();
        a.merge(&b);
        assert_eq!(a.count(), 0);
        assert!(a.mean().is_none());
    }

    #[test]
    fn merge_into_empty() {
        let mut a = WelfordF64::new();
        let mut b = WelfordF64::new();
        b.update(10.0);
        b.update(20.0);

        a.merge(&b);
        assert_eq!(a.count(), 2);
        assert!((a.mean().unwrap() - 15.0).abs() < 1e-10);
    }

    #[test]
    fn merge_empty_into_existing() {
        let mut a = WelfordF64::new();
        a.update(10.0);
        a.update(20.0);
        let b = WelfordF64::new();

        a.merge(&b);
        assert_eq!(a.count(), 2);
        assert!((a.mean().unwrap() - 15.0).abs() < 1e-10);
    }

    #[test]
    fn merge_matches_single_accumulator() {
        let data = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];

        // Single accumulator
        let mut single = WelfordF64::new();
        for &x in &data {
            single.update(x);
        }

        // Split into two halves and merge
        let mut first = WelfordF64::new();
        let mut second = WelfordF64::new();
        for &x in &data[..4] {
            first.update(x);
        }
        for &x in &data[4..] {
            second.update(x);
        }
        first.merge(&second);

        assert_eq!(first.count(), single.count());
        assert!((first.mean().unwrap() - single.mean().unwrap()).abs() < 1e-10);
        assert!((first.variance().unwrap() - single.variance().unwrap()).abs() < 1e-10);
    }

    #[test]
    fn merge_uneven_split() {
        let mut single = WelfordF64::new();
        let mut a = WelfordF64::new();
        let mut b = WelfordF64::new();

        for i in 0..100 {
            let x = i as f64;
            single.update(x);
            if i < 7 {
                a.update(x);
            } else {
                b.update(x);
            }
        }
        a.merge(&b);

        assert_eq!(a.count(), 100);
        assert!((a.mean().unwrap() - single.mean().unwrap()).abs() < 1e-10);
        assert!((a.variance().unwrap() - single.variance().unwrap()).abs() < 1e-6);
    }

    // =========================================================================
    // Reset
    // =========================================================================

    #[test]
    fn reset_clears_state() {
        let mut w = WelfordF64::new();
        for i in 0..100 {
            w.update(i as f64);
        }

        w.reset();
        assert_eq!(w.count(), 0);
        assert!(w.mean().is_none());
        assert!(w.variance().is_none());
    }

    // =========================================================================
    // Default
    // =========================================================================

    #[test]
    fn default_is_empty() {
        let w = WelfordF64::default();
        assert_eq!(w.count(), 0);
    }

    // =========================================================================
    // f32 variant
    // =========================================================================

    // =========================================================================
    // from_parts
    // =========================================================================

    #[test]
    fn from_parts_round_trip() {
        let mut w = WelfordF64::new();
        for &x in &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            w.update(x);
        }

        let count = w.count();
        let mean = w.mean().unwrap();
        // m2 = variance * (count - 1)
        let m2 = w.variance().unwrap() * (count - 1) as f64;

        let w2 = WelfordF64::from_parts(count, mean, m2);
        assert_eq!(w2.count(), count);
        assert!((w2.mean().unwrap() - mean).abs() < 1e-10);
        assert!((w2.variance().unwrap() - w.variance().unwrap()).abs() < 1e-10);
    }

    #[test]
    fn f32_basic() {
        let mut w = WelfordF32::new();
        w.update(10.0);
        w.update(20.0);

        assert_eq!(w.count(), 2);
        assert!((w.mean().unwrap() - 15.0).abs() < 1e-5);
    }

    #[test]
    fn f32_default() {
        let w = WelfordF32::default();
        assert_eq!(w.count(), 0);
    }
}
