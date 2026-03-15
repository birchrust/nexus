use crate::{EmaF32, EmaF64, WelfordF32, WelfordF64};

/// Anomaly direction from adaptive threshold detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anomaly {
    /// Within normal bounds.
    Normal,
    /// z-score exceeds upper threshold.
    High,
    /// z-score exceeds lower threshold (negative).
    Low,
}

macro_rules! impl_adaptive_threshold {
    ($name:ident, $builder:ident, $ty:ty, $ema:ty, $ema_builder:ident, $welford:ty) => {
        /// Adaptive threshold — z-score anomaly detection using EMA baseline + Welford std dev.
        ///
        /// Internally composes an EMA (for the moving baseline) with Welford's
        /// algorithm (for standard deviation). Signals anomaly when the z-score
        /// of a sample exceeds the configured threshold.
        ///
        /// # Use Cases
        /// - Latency anomaly detection with adaptive baseline
        /// - Price spike detection relative to recent volatility
        /// - Any signal where "abnormal" depends on recent context
        #[derive(Debug, Clone)]
        pub struct $name {
            ema: $ema,
            welford: $welford,
            z_threshold: $ty,
            last_z: $ty,
            min_samples: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: Option<$ty>,
            z_threshold: $ty,
            min_samples: u64,
            seed_mean: Option<$ty>,
            seed_std_dev: Option<$ty>,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    alpha: Option::None,
                    z_threshold: 3.0 as $ty,
                    min_samples: 20,
                    seed_mean: Option::None,
                    seed_std_dev: Option::None,
                }
            }

            /// Feeds a sample. Returns anomaly direction once primed.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<Anomaly> {
                let _ = self.ema.update(sample);
                self.welford.update(sample);

                if self.welford.count() < self.min_samples {
                    return Option::None;
                }

                let baseline = self.ema.value()?;
                let sd = match self.welford.std_dev() {
                    Some(v) if v > 0.0 as $ty => v,
                    _ => {
                        self.last_z = 0.0 as $ty;
                        return Option::Some(Anomaly::Normal);
                    }
                };

                self.last_z = (sample - baseline) / sd;

                if self.last_z > self.z_threshold {
                    Option::Some(Anomaly::High)
                } else if self.last_z < -self.z_threshold {
                    Option::Some(Anomaly::Low)
                } else {
                    Option::Some(Anomaly::Normal)
                }
            }

            /// Current EMA baseline, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn baseline(&self) -> Option<$ty> {
                if self.welford.count() >= self.min_samples {
                    self.ema.value()
                } else {
                    Option::None
                }
            }

            /// Current standard deviation from Welford, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn std_dev(&self) -> Option<$ty> {
                if self.welford.count() >= self.min_samples {
                    self.welford.std_dev()
                } else {
                    Option::None
                }
            }

            /// Last computed z-score, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn z_score(&self) -> Option<$ty> {
                if self.welford.count() >= self.min_samples {
                    Option::Some(self.last_z)
                } else {
                    Option::None
                }
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.welford.count()
            }

            /// Whether enough data has been collected.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.welford.count() >= self.min_samples
            }

            /// Resets to empty state. Parameters unchanged.
            #[inline]
            pub fn reset(&mut self) {
                self.ema.reset();
                self.welford.reset();
                self.last_z = 0.0 as $ty;
            }
        }

        impl $builder {
            /// EMA smoothing factor for the baseline.
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = Option::Some(alpha);
                self
            }

            /// Halflife for baseline smoothing.
            #[inline]
            #[must_use]
            pub fn halflife(mut self, halflife: $ty) -> Self {
                let ln2 = core::f64::consts::LN_2 as $ty;
                self.alpha = Option::Some(1.0 as $ty - crate::math::exp((-ln2 / halflife) as f64) as $ty);
                self
            }

            /// Span for baseline smoothing.
            #[inline]
            #[must_use]
            pub fn span(mut self, n: u64) -> Self {
                self.alpha = Option::Some(2.0 as $ty / (n as $ty + 1.0 as $ty));
                self
            }

            /// Z-score threshold for anomaly detection. Default: 3.0.
            #[inline]
            #[must_use]
            pub fn z_threshold(mut self, z: $ty) -> Self {
                self.z_threshold = z;
                self
            }

            /// Minimum samples before detection activates. Default: 20.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Pre-load baseline from calibration data, skipping warmup.
            #[inline]
            #[must_use]
            pub fn seed(mut self, mean: $ty, std_dev: $ty) -> Self {
                self.seed_mean = Option::Some(mean);
                self.seed_std_dev = Option::Some(std_dev);
                self
            }

            /// Builds the adaptive threshold detector.
            ///
            /// # Panics
            ///
            /// - Alpha must have been set.
            /// - Alpha must be in (0, 1) exclusive.
            /// - z_threshold must be positive.
            #[inline]
            #[must_use]
            pub fn build(self) -> $name {
                let alpha = self.alpha.expect("AdaptiveThreshold alpha must be set");
                assert!(alpha > 0.0 as $ty && alpha < 1.0 as $ty, "alpha must be in (0, 1)");
                assert!(self.z_threshold > 0.0 as $ty, "z_threshold must be positive");

                let ema = if let Some(seed_mean) = self.seed_mean {
                    <$ema>::builder().alpha(alpha).seed(seed_mean).min_samples(1).build()
                } else {
                    <$ema>::builder().alpha(alpha).min_samples(1).build()
                };

                // Welford doesn't support seeding directly — if seeded, we
                // set min_samples to allow immediate priming and accept that
                // the first few std_dev estimates will be from the seed approximation.
                let min_samples = if self.seed_mean.is_some() { 2 } else { self.min_samples };

                $name {
                    ema,
                    welford: <$welford>::new(),
                    z_threshold: self.z_threshold,
                    last_z: 0.0 as $ty,
                    min_samples,
                }
            }
        }
    };
}

impl_adaptive_threshold!(AdaptiveThresholdF64, AdaptiveThresholdF64Builder, f64, EmaF64, EmaF64Builder, WelfordF64);
impl_adaptive_threshold!(AdaptiveThresholdF32, AdaptiveThresholdF32Builder, f32, EmaF32, EmaF32Builder, WelfordF32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_high_anomaly() {
        let mut at = AdaptiveThresholdF64::builder()
            .alpha(0.1)
            .z_threshold(2.0)
            .min_samples(20)
            .build();

        // Feed normal samples around 100
        for _ in 0..50 {
            let _ = at.update(100.0);
        }

        // Spike — should be anomalous
        let result = at.update(200.0);
        assert_eq!(result, Some(Anomaly::High));
    }

    #[test]
    fn detects_low_anomaly() {
        let mut at = AdaptiveThresholdF64::builder()
            .alpha(0.1)
            .z_threshold(2.0)
            .min_samples(20)
            .build();

        for _ in 0..50 {
            let _ = at.update(100.0);
        }

        // Drop — need some variance first
        // Feed varying input to build std_dev
        let mut at2 = AdaptiveThresholdF64::builder()
            .alpha(0.1)
            .z_threshold(2.0)
            .min_samples(20)
            .build();

        for i in 0..50 {
            let _ = at2.update(100.0 + (i % 5) as f64);
        }

        let result = at2.update(50.0);
        assert_eq!(result, Some(Anomaly::Low));
    }

    #[test]
    fn no_false_positive_at_normal() {
        let mut at = AdaptiveThresholdF64::builder()
            .alpha(0.1)
            .z_threshold(3.0)
            .min_samples(20)
            .build();

        for i in 0..100 {
            let sample = 100.0 + (i % 3) as f64;
            let result = at.update(sample);
            if let Some(anomaly) = result {
                assert_eq!(anomaly, Anomaly::Normal, "false positive at sample {i}");
            }
        }
    }

    #[test]
    fn priming() {
        let mut at = AdaptiveThresholdF64::builder()
            .alpha(0.1)
            .min_samples(10)
            .build();

        for _ in 0..9 {
            assert!(at.update(100.0).is_none());
        }
        assert!(!at.is_primed());
        assert!(at.update(100.0).is_some());
        assert!(at.is_primed());
    }

    #[test]
    fn seeded_startup() {
        let mut at = AdaptiveThresholdF64::builder()
            .alpha(0.1)
            .z_threshold(3.0)
            .seed(100.0, 5.0)
            .build();

        // Should be primed quickly since seeded
        let _ = at.update(100.0);
        let _ = at.update(100.0);
        assert!(at.is_primed());
        assert!(at.baseline().is_some());
    }

    #[test]
    fn reset() {
        let mut at = AdaptiveThresholdF64::builder()
            .alpha(0.1)
            .min_samples(5)
            .build();

        for _ in 0..20 {
            let _ = at.update(100.0);
        }
        at.reset();
        assert_eq!(at.count(), 0);
        assert!(!at.is_primed());
    }

    #[test]
    fn f32_basic() {
        let mut at = AdaptiveThresholdF32::builder()
            .alpha(0.1)
            .min_samples(5)
            .build();

        for _ in 0..10 {
            let _ = at.update(100.0);
        }
        assert!(at.is_primed());
    }

    #[test]
    #[should_panic(expected = "alpha must be set")]
    fn panics_without_alpha() {
        let _ = AdaptiveThresholdF64::builder().build();
    }
}
