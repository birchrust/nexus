use crate::Condition;
use crate::math::MulAdd;

macro_rules! impl_saturation {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Saturation detector — smoothed utilization with threshold.
        ///
        /// Internally uses an EMA to smooth the utilization signal and
        /// compares against a configured threshold.
        ///
        /// # Use Cases
        /// - CPU/memory utilization monitoring
        /// - Queue fill level monitoring
        /// - Bandwidth saturation detection
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

            /// Feeds a utilization sample. Returns pressure state once primed.
            #[inline]
            #[must_use]
            pub fn update(&mut self, utilization: $ty) -> Option<Condition> {
                self.count += 1;

                if self.count == 1 {
                    self.value = utilization;
                } else {
                    self.value = self.alpha.fma(utilization, self.one_minus_alpha * self.value);
                }

                if self.count < self.min_samples {
                    return Option::None;
                }

                if self.value > self.threshold {
                    Option::Some(Condition::Degraded)
                } else {
                    Option::Some(Condition::Normal)
                }
            }

            /// Current smoothed utilization, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn utilization(&self) -> Option<$ty> {
                if self.count >= self.min_samples {
                    Option::Some(self.value)
                } else {
                    Option::None
                }
            }

            /// Number of samples processed.
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

            /// Updates the saturation threshold without resetting state.
            #[inline]
            pub fn reconfigure_threshold(&mut self, threshold: $ty) {
                self.threshold = threshold;
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
            #[cfg(any(feature = "std", feature = "libm"))]
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

            /// Saturation threshold. Default must be set.
            #[inline]
            #[must_use]
            pub fn threshold(mut self, threshold: $ty) -> Self {
                self.threshold = Option::Some(threshold);
                self
            }

            /// Minimum samples before detection activates. Default: 1.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the saturation detector.
            ///
            /// # Errors
            ///
            /// - Alpha and threshold must have been set.
            /// - Alpha must be in (0, 1) exclusive.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let alpha = self.alpha.ok_or(crate::ConfigError::Missing("alpha"))?;
                let threshold = self.threshold.ok_or(crate::ConfigError::Missing("threshold"))?;
                if !(alpha > 0.0 as $ty && alpha < 1.0 as $ty) {
                    return Err(crate::ConfigError::Invalid("alpha must be in (0, 1)"));
                }

                Ok($name {
                    alpha,
                    one_minus_alpha: 1.0 as $ty - alpha,
                    value: 0.0 as $ty,
                    threshold,
                    count: 0,
                    min_samples: self.min_samples,
                })
            }
        }
    };
}

impl_saturation!(SaturationF64, SaturationF64Builder, f64);
impl_saturation!(SaturationF32, SaturationF32Builder, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_threshold_is_normal() {
        let mut s = SaturationF64::builder()
            .alpha(0.3)
            .threshold(0.8)
            .build().unwrap();

        for _ in 0..50 {
            assert_eq!(s.update(0.5), Some(Condition::Normal));
        }
    }

    #[test]
    fn above_threshold_is_saturated() {
        let mut s = SaturationF64::builder()
            .alpha(0.3)
            .threshold(0.8)
            .build().unwrap();

        for _ in 0..50 {
            let _ = s.update(0.95);
        }
        assert_eq!(s.update(0.95), Some(Condition::Degraded));
    }

    #[test]
    fn crosses_back() {
        let mut s = SaturationF64::builder()
            .alpha(0.5)
            .threshold(0.8)
            .build().unwrap();

        // Drive up
        for _ in 0..50 {
            let _ = s.update(0.95);
        }
        assert_eq!(s.update(0.95), Some(Condition::Degraded));

        // Drive down
        for _ in 0..50 {
            let _ = s.update(0.3);
        }
        assert_eq!(s.update(0.3), Some(Condition::Normal));
    }

    #[test]
    fn priming() {
        let mut s = SaturationF64::builder()
            .alpha(0.3)
            .threshold(0.8)
            .min_samples(5)
            .build().unwrap();

        for _ in 0..4 {
            assert!(s.update(0.95).is_none());
        }
        assert!(s.update(0.95).is_some());
    }

    #[test]
    fn reset() {
        let mut s = SaturationF64::builder()
            .alpha(0.3)
            .threshold(0.8)
            .build().unwrap();

        for _ in 0..10 {
            let _ = s.update(0.95);
        }
        s.reset();
        assert_eq!(s.count(), 0);
        assert!(s.utilization().is_none());
    }

    #[test]
    fn f32_basic() {
        let mut s = SaturationF32::builder()
            .alpha(0.3)
            .threshold(0.8)
            .build().unwrap();

        assert_eq!(s.update(0.5), Some(Condition::Normal));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn reconfigure_threshold_changes_behavior() {
        let mut s = SaturationF64::builder()
            .alpha(0.3)
            .threshold(0.8)
            .build().unwrap();

        for _ in 0..50 {
            let _ = s.update(0.75);
        }
        assert_eq!(s.update(0.75), Some(Condition::Normal));

        // Lower the threshold — same value should now be degraded
        s.reconfigure_threshold(0.7);
        assert_eq!(s.update(0.75), Some(Condition::Degraded));
    }

    #[test]
    fn errors_without_threshold() {
        let result = SaturationF64::builder().alpha(0.3).build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("threshold"))));
    }
}
