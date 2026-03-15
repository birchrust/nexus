use crate::math::MulAdd;
macro_rules! impl_holt {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Holt's Double Exponential Smoothing — level + trend tracking.
        ///
        /// Separates the signal into level (current smoothed value) and trend
        /// (rate of change). Detects not just "value is high" but "value is
        /// getting worse over time."
        ///
        /// # Use Cases
        /// - Trend detection ("latency is increasing linearly")
        /// - Capacity planning and degradation forecasting
        /// - Adaptive baselines that track drift
        #[derive(Debug, Clone)]
        pub struct $name {
            alpha: $ty,
            beta: $ty,
            level: $ty,
            trend: $ty,
            count: u64,
            min_samples: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: Option<$ty>,
            beta: Option<$ty>,
            min_samples: u64,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    alpha: Option::None,
                    beta: Option::None,
                    min_samples: 2,
                }
            }

            /// Feeds a sample. Returns `(level, trend)` once primed.
            ///
            /// First sample sets the level. Second sample initializes the trend.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<($ty, $ty)> {
                self.count += 1;

                if self.count == 1 {
                    self.level = sample;
                    self.trend = 0.0 as $ty;
                } else if self.count == 2 {
                    let prev_level = self.level;
                    self.level = sample;
                    self.trend = sample - prev_level;
                } else {
                    let prev_level = self.level;
                    // Level: alpha * sample + (1 - alpha) * (prev_level + prev_trend)
                    self.level = self.alpha.fma(sample, (1.0 as $ty - self.alpha) * (prev_level + self.trend));
                    // Trend: beta * (level - prev_level) + (1 - beta) * prev_trend
                    self.trend = self.beta.fma(self.level - prev_level, (1.0 as $ty - self.beta) * self.trend);
                }

                if self.count >= self.min_samples {
                    Option::Some((self.level, self.trend))
                } else {
                    Option::None
                }
            }

            /// Current smoothed level, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn level(&self) -> Option<$ty> {
                if self.count >= self.min_samples {
                    Option::Some(self.level)
                } else {
                    Option::None
                }
            }

            /// Current trend (rate of change), or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn trend(&self) -> Option<$ty> {
                if self.count >= self.min_samples {
                    Option::Some(self.trend)
                } else {
                    Option::None
                }
            }

            /// Forecast: `level + steps * trend`. Or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn forecast(&self, steps: u64) -> Option<$ty> {
                if self.count >= self.min_samples {
                    Option::Some(self.trend.fma(steps as $ty, self.level))
                } else {
                    Option::None
                }
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether Holt's has reached `min_samples`.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count >= self.min_samples
            }

            /// Resets to uninitialized state. Parameters unchanged.
            #[inline]
            pub fn reset(&mut self) {
                self.level = 0.0 as $ty;
                self.trend = 0.0 as $ty;
                self.count = 0;
            }
        }

        impl $builder {
            /// Level smoothing factor. Must be in (0, 1) exclusive.
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = Option::Some(alpha);
                self
            }

            /// Trend smoothing factor. Must be in (0, 1) exclusive.
            #[inline]
            #[must_use]
            pub fn beta(mut self, beta: $ty) -> Self {
                self.beta = Option::Some(beta);
                self
            }

            /// Minimum samples before values are valid. Default: 2.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the Holt's smoother.
            ///
            /// # Panics
            ///
            /// - Alpha and beta must have been set.
            /// - Both must be in (0, 1) exclusive.
            #[inline]
            #[must_use]
            pub fn build(self) -> $name {
                let alpha = self.alpha.expect("Holt alpha must be set");
                let beta = self.beta.expect("Holt beta must be set");
                assert!(alpha > 0.0 as $ty && alpha < 1.0 as $ty, "Holt alpha must be in (0, 1)");
                assert!(beta > 0.0 as $ty && beta < 1.0 as $ty, "Holt beta must be in (0, 1)");

                $name {
                    alpha,
                    beta,
                    level: 0.0 as $ty,
                    trend: 0.0 as $ty,
                    count: 0,
                    min_samples: self.min_samples,
                }
            }
        }
    };
}

impl_holt!(HoltF64, HoltF64Builder, f64);
impl_holt!(HoltF32, HoltF32Builder, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_input_zero_trend() {
        let mut h = HoltF64::builder().alpha(0.3).beta(0.1).build();

        for _ in 0..100 {
            let _ = h.update(50.0);
        }

        let trend = h.trend().unwrap();
        assert!(trend.abs() < 0.01, "constant input should have ~zero trend, got {trend}");
    }

    #[test]
    fn linear_input_correct_trend() {
        let mut h = HoltF64::builder().alpha(0.5).beta(0.5).build();

        // Feed linear data: 0, 10, 20, 30, ...
        for i in 0..100 {
            let _ = h.update(i as f64 * 10.0);
        }

        let trend = h.trend().unwrap();
        // Should converge to ~10.0 (the slope)
        assert!((trend - 10.0).abs() < 1.0, "linear trend should be ~10, got {trend}");
    }

    #[test]
    fn forecast_accuracy() {
        let mut h = HoltF64::builder().alpha(0.5).beta(0.5).build();

        for i in 0..50 {
            let _ = h.update(i as f64 * 10.0);
        }

        let forecast_5 = h.forecast(5).unwrap();
        let level = h.level().unwrap();
        let trend = h.trend().unwrap();

        // forecast(5) = level + 5 * trend
        let expected = 5.0f64.fma(trend, level);
        assert!((forecast_5 - expected).abs() < 1e-10);
    }

    #[test]
    fn priming_needs_two_samples() {
        let mut h = HoltF64::builder().alpha(0.3).beta(0.1).build();

        assert!(h.update(10.0).is_none()); // first sample — not primed
        assert!(h.update(20.0).is_some()); // second sample — primed
    }

    #[test]
    fn reset_clears() {
        let mut h = HoltF64::builder().alpha(0.3).beta(0.1).build();
        let _ = h.update(10.0);
        let _ = h.update(20.0);

        h.reset();
        assert_eq!(h.count(), 0);
        assert!(h.level().is_none());
        assert!(h.trend().is_none());
    }

    #[test]
    fn f32_basic() {
        let mut h = HoltF32::builder().alpha(0.3).beta(0.1).build();
        let _ = h.update(10.0);
        let result = h.update(20.0);
        assert!(result.is_some());
    }

    #[test]
    #[should_panic(expected = "alpha must be set")]
    fn panics_without_alpha() {
        let _ = HoltF64::builder().beta(0.1).build();
    }

    #[test]
    #[should_panic(expected = "beta must be set")]
    fn panics_without_beta() {
        let _ = HoltF64::builder().alpha(0.3).build();
    }
}
