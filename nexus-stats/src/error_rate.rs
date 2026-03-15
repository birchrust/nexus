use crate::math::MulAdd;
/// Health state from error rate monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Error rate below threshold.
    Healthy,
    /// Smoothed error rate exceeds threshold.
    Degraded,
}

macro_rules! impl_error_rate {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Smoothed error rate tracker.
        ///
        /// Internally uses an EMA where success = 0.0 and failure = 1.0 (or
        /// weighted). The smoothed value approximates the recent error rate.
        ///
        /// # Use Cases
        /// - API error rate monitoring
        /// - Exchange rejection rate tracking
        /// - Weighted severity tracking (critical failures count more)
        ///
        /// # Weighted outcomes
        ///
        /// `record_weighted(false, 2.0)` feeds 2.0 to the EMA (a failure that
        /// counts double). With weights > 1.0, the smoothed "rate" can exceed
        /// 1.0 — it becomes a severity-weighted signal rather than a pure rate.
        #[derive(Debug, Clone)]
        pub struct $name {
            alpha: $ty,
            one_minus_alpha: $ty,
            value: $ty,
            threshold: $ty,
            count: u64,
            min_samples: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: Option<$ty>,
            threshold: Option<$ty>,
            min_samples: u64,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    alpha: Option::None,
                    threshold: Option::None,
                    min_samples: 1,
                }
            }

            /// Records an outcome with default weight 1.0.
            #[inline]
            #[must_use]
            pub fn record(&mut self, success: bool) -> Option<Health> {
                self.record_weighted(success, 1.0 as $ty)
            }

            /// Records an outcome with a severity weight.
            ///
            /// Success feeds `0.0`, failure feeds `weight`. The EMA smooths
            /// this signal. With `weight=1.0`, the smoothed value is the
            /// recent error rate in [0, 1]. With `weight > 1.0`, it can exceed 1.0.
            #[inline]
            #[must_use]
            pub fn record_weighted(&mut self, success: bool, weight: $ty) -> Option<Health> {
                self.count += 1;

                let sample = if success { 0.0 as $ty } else { weight };

                if self.count == 1 {
                    self.value = sample;
                } else {
                    self.value = self.alpha.fma(sample, self.one_minus_alpha * self.value);
                }

                if self.count < self.min_samples {
                    return Option::None;
                }

                if self.value > self.threshold {
                    Option::Some(Health::Degraded)
                } else {
                    Option::Some(Health::Healthy)
                }
            }

            /// Current smoothed error rate, or `None` if not primed.
            ///
            /// With unweighted `record()`, this is in [0.0, 1.0].
            /// With weighted outcomes, it may exceed 1.0.
            #[inline]
            #[must_use]
            pub fn error_rate(&self) -> Option<$ty> {
                if self.count >= self.min_samples {
                    Option::Some(self.value)
                } else {
                    Option::None
                }
            }

            /// Number of outcomes recorded.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 { self.count }

            /// Whether enough data has been collected.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool { self.count >= self.min_samples }

            /// Resets to empty state. Parameters unchanged.
            #[inline]
            pub fn reset(&mut self) {
                self.value = 0.0 as $ty;
                self.count = 0;
            }
        }

        impl $builder {
            /// Smoothing factor.
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = Option::Some(alpha);
                self
            }

            /// Halflife for smoothing.
            #[inline]
            #[must_use]
            pub fn halflife(mut self, halflife: $ty) -> Self {
                let ln2 = core::f64::consts::LN_2 as $ty;
                self.alpha = Option::Some(1.0 as $ty - crate::math::exp((-ln2 / halflife) as f64) as $ty);
                self
            }

            /// Span for smoothing.
            #[inline]
            #[must_use]
            pub fn span(mut self, n: u64) -> Self {
                self.alpha = Option::Some(2.0 as $ty / (n as $ty + 1.0 as $ty));
                self
            }

            /// Error rate threshold. Degraded when smoothed rate exceeds this.
            #[inline]
            #[must_use]
            pub fn threshold(mut self, threshold: $ty) -> Self {
                self.threshold = Option::Some(threshold);
                self
            }

            /// Minimum outcomes before detection activates. Default: 1.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the error rate tracker.
            ///
            /// # Panics
            ///
            /// - Alpha and threshold must have been set.
            /// - Alpha must be in (0, 1) exclusive.
            /// - Threshold must be positive.
            #[inline]
            #[must_use]
            pub fn build(self) -> $name {
                let alpha = self.alpha.expect("ErrorRate alpha must be set");
                let threshold = self.threshold.expect("ErrorRate threshold must be set");
                assert!(alpha > 0.0 as $ty && alpha < 1.0 as $ty, "alpha must be in (0, 1)");
                assert!(threshold > 0.0 as $ty, "threshold must be positive");

                $name {
                    alpha,
                    one_minus_alpha: 1.0 as $ty - alpha,
                    value: 0.0 as $ty,
                    threshold,
                    count: 0,
                    min_samples: self.min_samples,
                }
            }
        }
    };
}

impl_error_rate!(ErrorRateF64, ErrorRateF64Builder, f64);
impl_error_rate!(ErrorRateF32, ErrorRateF32Builder, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_success_is_healthy() {
        let mut er = ErrorRateF64::builder()
            .alpha(0.3)
            .threshold(0.05)
            .build();

        for _ in 0..100 {
            assert_eq!(er.record(true), Some(Health::Healthy));
        }
    }

    #[test]
    fn all_failure_is_degraded() {
        let mut er = ErrorRateF64::builder()
            .alpha(0.3)
            .threshold(0.05)
            .build();

        for _ in 0..50 {
            let _ = er.record(false);
        }
        assert_eq!(er.record(false), Some(Health::Degraded));
    }

    #[test]
    fn threshold_crossing() {
        let mut er = ErrorRateF64::builder()
            .alpha(0.3)
            .threshold(0.5)
            .build();

        // All success — should be healthy
        for _ in 0..20 {
            let _ = er.record(true);
        }
        assert_eq!(er.record(true), Some(Health::Healthy));

        // All failure — should become degraded
        for _ in 0..50 {
            let _ = er.record(false);
        }
        assert_eq!(er.record(false), Some(Health::Degraded));
    }

    #[test]
    fn weighted_failure_triggers_faster() {
        let mut light = ErrorRateF64::builder()
            .alpha(0.5)
            .threshold(0.5)
            .build();
        let mut heavy = ErrorRateF64::builder()
            .alpha(0.5)
            .threshold(0.5)
            .build();

        // Both start healthy
        for _ in 0..10 {
            let _ = light.record(true);
            let _ = heavy.record(true);
        }

        // One weighted failure
        let _ = light.record_weighted(false, 1.0);
        let _ = heavy.record_weighted(false, 5.0);

        let light_rate = light.error_rate().unwrap();
        let heavy_rate = heavy.error_rate().unwrap();

        assert!(heavy_rate > light_rate,
            "heavy ({heavy_rate}) should exceed light ({light_rate})");
    }

    #[test]
    fn priming() {
        let mut er = ErrorRateF64::builder()
            .alpha(0.3)
            .threshold(0.05)
            .min_samples(5)
            .build();

        for _ in 0..4 {
            assert!(er.record(false).is_none());
        }
        assert!(er.record(false).is_some());
    }

    #[test]
    fn reset() {
        let mut er = ErrorRateF64::builder()
            .alpha(0.3)
            .threshold(0.05)
            .build();

        for _ in 0..10 {
            let _ = er.record(false);
        }
        er.reset();
        assert_eq!(er.count(), 0);
        assert!(er.error_rate().is_none());
    }

    #[test]
    fn f32_basic() {
        let mut er = ErrorRateF32::builder()
            .alpha(0.3)
            .threshold(0.05)
            .build();

        assert_eq!(er.record(true), Some(Health::Healthy));
    }

    #[test]
    #[should_panic(expected = "threshold must be set")]
    fn panics_without_threshold() {
        let _ = ErrorRateF64::builder().alpha(0.3).build();
    }
}
