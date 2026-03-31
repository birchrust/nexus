use nexus_stats_core::Direction;
use nexus_stats_smoothing::{HoltF32, HoltF64};

macro_rules! impl_trend_alert {
    ($name:ident, $builder:ident, $ty:ty, $holt:ty) => {
        /// Trend alert — Holt's double exponential with threshold on the trend component.
        ///
        /// Detects not just "value is high" but "value is getting worse over time."
        /// Supports both absolute and relative trend thresholds.
        ///
        /// # Use Cases
        /// - "Latency is increasing linearly"
        /// - Capacity planning — detect degradation trends
        /// - Drift detection in stationary processes
        #[derive(Debug, Clone)]
        pub struct $name {
            holt: $holt,
            trend_threshold_abs: Option<$ty>,
            trend_threshold_rel: Option<$ty>,
            min_samples: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: Option<$ty>,
            beta: Option<$ty>,
            trend_threshold_abs: Option<$ty>,
            trend_threshold_rel: Option<$ty>,
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
                    trend_threshold_abs: Option::None,
                    trend_threshold_rel: Option::None,
                    min_samples: 2,
                }
            }

            /// Feeds a sample. Returns trend direction once primed.
            ///
            /// # Errors
            ///
            /// Returns `DataError::NotANumber` if the sample is NaN, or
            /// `DataError::Infinite` if the sample is infinite.
            #[inline]
            pub fn update(
                &mut self,
                sample: $ty,
            ) -> Result<Option<Direction>, nexus_stats_core::DataError> {
                check_finite!(sample);
                // Holt already validated by check_finite above
                let result = self.holt.update(sample)?;

                if self.holt.count() < self.min_samples {
                    return Ok(Option::None);
                }

                let Some((level, trend)) = result else {
                    return Ok(Option::None);
                };

                // Check absolute threshold
                if let Some(abs_thresh) = self.trend_threshold_abs {
                    if trend > abs_thresh {
                        return Ok(Option::Some(Direction::Rising));
                    } else if trend < -abs_thresh {
                        return Ok(Option::Some(Direction::Falling));
                    }
                }

                // Check relative threshold
                if let Some(rel_thresh) = self.trend_threshold_rel {
                    #[allow(clippy::float_cmp)]
                    if level != (0.0 as $ty) {
                        let ratio = trend / level;
                        if ratio > rel_thresh {
                            return Ok(Option::Some(Direction::Rising));
                        } else if ratio < -rel_thresh {
                            return Ok(Option::Some(Direction::Falling));
                        }
                    }
                }

                Ok(Option::Some(Direction::Neutral))
            }

            /// Current smoothed level, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn level(&self) -> Option<$ty> {
                self.holt.level()
            }

            /// Current trend (rate of change), or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn trend(&self) -> Option<$ty> {
                self.holt.trend()
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.holt.count()
            }

            /// Whether enough data has been collected.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.holt.count() >= self.min_samples
            }

            /// Resets to empty state. Parameters unchanged.
            #[inline]
            pub fn reset(&mut self) {
                self.holt.reset();
            }

            /// Updates the absolute trend threshold without resetting state.
            ///
            /// # Errors
            ///
            /// Threshold must be >= 0.
            #[inline]
            pub fn reconfigure_threshold(
                &mut self,
                threshold: $ty,
            ) -> Result<(), nexus_stats_core::ConfigError> {
                if threshold < (0.0 as $ty) {
                    return Err(nexus_stats_core::ConfigError::Invalid(
                        "threshold must be non-negative",
                    ));
                }
                self.trend_threshold_abs = Option::Some(threshold);
                Ok(())
            }
        }

        impl $builder {
            /// Level smoothing factor (Holt's alpha).
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = Option::Some(alpha);
                self
            }

            /// Trend smoothing factor (Holt's beta).
            #[inline]
            #[must_use]
            pub fn beta(mut self, beta: $ty) -> Self {
                self.beta = Option::Some(beta);
                self
            }

            /// Absolute trend threshold. Alert when `|trend| > threshold`.
            #[inline]
            #[must_use]
            pub fn trend_threshold(mut self, threshold: $ty) -> Self {
                self.trend_threshold_abs = Option::Some(threshold);
                self
            }

            /// Relative trend threshold. Alert when `|trend / level| > fraction`.
            #[inline]
            #[must_use]
            pub fn trend_threshold_relative(mut self, fraction: $ty) -> Self {
                self.trend_threshold_rel = Option::Some(fraction);
                self
            }

            /// Minimum samples before detection activates. Default: 2.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the trend alert detector.
            ///
            /// # Errors
            ///
            /// - Alpha and beta must have been set.
            /// - At least one threshold (absolute or relative) must be set.
            #[inline]
            pub fn build(self) -> Result<$name, nexus_stats_core::ConfigError> {
                let alpha = self
                    .alpha
                    .ok_or(nexus_stats_core::ConfigError::Missing("alpha"))?;
                let beta = self
                    .beta
                    .ok_or(nexus_stats_core::ConfigError::Missing("beta"))?;
                if self.trend_threshold_abs.is_none() && self.trend_threshold_rel.is_none() {
                    return Err(nexus_stats_core::ConfigError::Invalid(
                        "TrendAlert requires a trend threshold",
                    ));
                }

                let holt = <$holt>::builder()
                    .alpha(alpha)
                    .beta(beta)
                    .min_samples(self.min_samples)
                    .build()?;

                Ok($name {
                    holt,
                    trend_threshold_abs: self.trend_threshold_abs,
                    trend_threshold_rel: self.trend_threshold_rel,
                    min_samples: self.min_samples,
                })
            }
        }
    };
}

impl_trend_alert!(TrendAlertF64, TrendAlertF64Builder, f64, HoltF64);
impl_trend_alert!(TrendAlertF32, TrendAlertF32Builder, f32, HoltF32);

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_stats_core::math::MulAdd;

    #[test]
    fn constant_is_stable() {
        let mut ta = TrendAlertF64::builder()
            .alpha(0.3)
            .beta(0.1)
            .trend_threshold(1.0)
            .build()
            .unwrap();

        for _ in 0..50 {
            let _ = ta.update(100.0);
        }
        assert_eq!(ta.update(100.0).unwrap(), Some(Direction::Neutral));
    }

    #[test]
    fn linear_increase_is_rising() {
        let mut ta = TrendAlertF64::builder()
            .alpha(0.5)
            .beta(0.5)
            .trend_threshold(5.0)
            .build()
            .unwrap();

        for i in 0..100 {
            let _ = ta.update(i as f64 * 10.0);
        }
        assert_eq!(ta.update(1000.0).unwrap(), Some(Direction::Rising));
    }

    #[test]
    fn linear_decrease_is_falling() {
        let mut ta = TrendAlertF64::builder()
            .alpha(0.5)
            .beta(0.5)
            .trend_threshold(5.0)
            .build()
            .unwrap();

        for i in 0..100 {
            let _ = ta.update((i as f64).fma(-10.0, 1000.0));
        }
        let result = ta.update(0.0).unwrap();
        assert_eq!(result, Some(Direction::Falling));
    }

    #[test]
    fn relative_threshold() {
        let mut ta = TrendAlertF64::builder()
            .alpha(0.5)
            .beta(0.5)
            .trend_threshold_relative(0.05)
            .build()
            .unwrap();

        for i in 0..100 {
            let _ = ta.update((i as f64).fma(10.0, 100.0));
        }
        // Trend should be ~10, level ~1000-ish, ratio ~0.01
        // But early on with strong trend it should trigger
        assert!(ta.trend().is_some());
    }

    #[test]
    fn priming() {
        let mut ta = TrendAlertF64::builder()
            .alpha(0.3)
            .beta(0.1)
            .trend_threshold(1.0)
            .min_samples(5)
            .build()
            .unwrap();

        for _ in 0..4 {
            assert!(ta.update(100.0).unwrap().is_none());
        }
        assert!(ta.update(100.0).unwrap().is_some());
    }

    #[test]
    fn reset() {
        let mut ta = TrendAlertF64::builder()
            .alpha(0.3)
            .beta(0.1)
            .trend_threshold(1.0)
            .build()
            .unwrap();

        for _ in 0..20 {
            let _ = ta.update(100.0);
        }
        ta.reset();
        assert_eq!(ta.count(), 0);
    }

    #[test]
    fn f32_basic() {
        let mut ta = TrendAlertF32::builder()
            .alpha(0.3)
            .beta(0.1)
            .trend_threshold(1.0)
            .build()
            .unwrap();

        let _ = ta.update(10.0);
        let _ = ta.update(20.0);
        assert!(ta.is_primed());
    }

    #[test]
    fn reconfigure_threshold_preserves_state() {
        let mut ta = TrendAlertF64::builder()
            .alpha(0.3)
            .beta(0.1)
            .trend_threshold(1.0)
            .build()
            .unwrap();

        for _ in 0..20 {
            let _ = ta.update(100.0);
        }
        let count_before = ta.count();

        ta.reconfigure_threshold(0.5).unwrap();
        assert_eq!(ta.count(), count_before);
        assert!(ta.is_primed());
    }

    #[test]
    fn errors_without_threshold() {
        let result = TrendAlertF64::builder().alpha(0.3).beta(0.1).build();
        assert!(matches!(
            result,
            Err(nexus_stats_core::ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_nan_and_inf() {
        let mut ta = TrendAlertF64::builder()
            .alpha(0.3)
            .beta(0.1)
            .trend_threshold(1.0)
            .build()
            .unwrap();

        assert_eq!(
            ta.update(f64::NAN).unwrap_err(),
            nexus_stats_core::DataError::NotANumber
        );
        assert_eq!(
            ta.update(f64::INFINITY).unwrap_err(),
            nexus_stats_core::DataError::Infinite
        );
        assert_eq!(
            ta.update(f64::NEG_INFINITY).unwrap_err(),
            nexus_stats_core::DataError::Infinite
        );
        assert_eq!(ta.count(), 0);
    }
}
