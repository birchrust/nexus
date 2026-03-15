use crate::math::MulAdd;
macro_rules! impl_robust_z {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Robust z-score — EMA-based modified z-score with estimator freeze.
        ///
        /// Uses the EMA of absolute deviations (EMA-MAD) instead of standard
        /// deviation. More robust to outliers than Welford-based z-scores.
        /// Freezes the estimator when the z-score exceeds a reject threshold,
        /// preventing outliers from corrupting the baseline.
        ///
        /// Modified z-score: `0.6745 * (x - ema) / ema_abs_dev`
        ///
        /// The constant 0.6745 is the 0.75th quantile of the standard normal,
        /// making MAD comparable to standard deviation for normal distributions.
        ///
        /// # Use Cases
        /// - Outlier detection robust to contamination
        /// - Market data quality where outliers are common
        /// - Any signal where Welford's variance would be corrupted by spikes
        #[derive(Debug, Clone)]
        pub struct $name {
            alpha: $ty,
            one_minus_alpha: $ty,
            ema: $ty,
            ema_abs_dev: $ty,
            last_z: $ty,
            reject_threshold: $ty,
            count: u64,
            min_samples: u64,
            initialized: bool,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: Option<$ty>,
            reject_threshold: Option<$ty>,
            min_samples: u64,
        }

        impl $name {
            /// The constant relating MAD to standard deviation for normal distributions.
            const MAD_CONSTANT: $ty = 0.6745 as $ty;

            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    alpha: Option::None,
                    reject_threshold: Option::None,
                    min_samples: 10,
                }
            }

            /// Feeds a sample. Returns the modified z-score once primed.
            ///
            /// If the z-score exceeds `reject_threshold`, the EMA and MAD
            /// are NOT updated — the outlier is scored but doesn't corrupt
            /// the baseline.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<$ty> {
                self.count += 1;

                if !self.initialized {
                    self.ema = sample;
                    self.ema_abs_dev = 0.0 as $ty;
                    self.initialized = true;
                    self.last_z = 0.0 as $ty;
                    return if self.count >= self.min_samples {
                        Option::Some(0.0 as $ty)
                    } else {
                        Option::None
                    };
                }

                let abs_dev = (sample - self.ema).abs();

                // Compute z-score
                self.last_z = if self.ema_abs_dev > 0.0 as $ty {
                    Self::MAD_CONSTANT * (sample - self.ema) / self.ema_abs_dev
                } else {
                    0.0 as $ty
                };

                // Only update estimators if not rejected
                if self.last_z.abs() <= self.reject_threshold {
                    self.ema = self.alpha.fma(sample, self.one_minus_alpha * self.ema);
                    self.ema_abs_dev = self.alpha.fma(abs_dev, self.one_minus_alpha * self.ema_abs_dev);
                }

                if self.count >= self.min_samples {
                    Option::Some(self.last_z)
                } else {
                    Option::None
                }
            }

            /// Last computed z-score, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn z_score(&self) -> Option<$ty> {
                if self.count >= self.min_samples {
                    Option::Some(self.last_z)
                } else {
                    Option::None
                }
            }

            /// Current EMA baseline, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn baseline(&self) -> Option<$ty> {
                if self.count >= self.min_samples { Option::Some(self.ema) } else { Option::None }
            }

            /// Current EMA of absolute deviation (MAD proxy), or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn mad(&self) -> Option<$ty> {
                if self.count >= self.min_samples { Option::Some(self.ema_abs_dev) } else { Option::None }
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 { self.count }

            /// Whether the detector has reached `min_samples`.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool { self.count >= self.min_samples }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.ema = 0.0 as $ty;
                self.ema_abs_dev = 0.0 as $ty;
                self.last_z = 0.0 as $ty;
                self.count = 0;
                self.initialized = false;
            }
        }

        impl $builder {
            /// EMA smoothing factor.
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = Option::Some(alpha);
                self
            }

            /// Span for EMA smoothing.
            #[inline]
            #[must_use]
            pub fn span(mut self, n: u64) -> Self {
                self.alpha = Option::Some(2.0 as $ty / (n as $ty + 1.0 as $ty));
                self
            }

            /// Z-score threshold above which the estimator freezes. Default must be set.
            #[inline]
            #[must_use]
            pub fn reject_threshold(mut self, z: $ty) -> Self {
                self.reject_threshold = Option::Some(z);
                self
            }

            /// Minimum samples before z-scores are computed. Default: 10.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the robust z-score detector.
            ///
            /// # Panics
            ///
            /// - Alpha and reject_threshold must have been set.
            #[inline]
            #[must_use]
            pub fn build(self) -> $name {
                let alpha = self.alpha.expect("RobustZScore alpha must be set");
                let reject = self.reject_threshold.expect("RobustZScore reject_threshold must be set");
                assert!(alpha > 0.0 as $ty && alpha < 1.0 as $ty, "alpha must be in (0, 1)");
                assert!(reject > 0.0 as $ty, "reject_threshold must be positive");

                $name {
                    alpha,
                    one_minus_alpha: 1.0 as $ty - alpha,
                    ema: 0.0 as $ty,
                    ema_abs_dev: 0.0 as $ty,
                    last_z: 0.0 as $ty,
                    reject_threshold: reject,
                    count: 0,
                    min_samples: self.min_samples,
                    initialized: false,
                }
            }
        }
    };
}

impl_robust_z!(RobustZScoreF64, RobustZScoreF64Builder, f64);
impl_robust_z!(RobustZScoreF32, RobustZScoreF32Builder, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_signal_low_z() {
        let mut rz = RobustZScoreF64::builder()
            .alpha(0.1)
            .reject_threshold(10.0)
            .min_samples(5)
            .build();

        for _ in 0..20 {
            let _ = rz.update(100.0);
        }

        let z = rz.z_score().unwrap();
        assert!(z.abs() < 0.1, "stable signal should have ~zero z-score, got {z}");
    }

    #[test]
    fn outlier_high_z() {
        let mut rz = RobustZScoreF64::builder()
            .alpha(0.1)
            .reject_threshold(10.0)
            .min_samples(5)
            .build();

        // Build baseline around 100 with small variation
        for i in 0..20 {
            let _ = rz.update(100.0 + (i % 3) as f64);
        }

        // Outlier
        let z = rz.update(200.0).unwrap();
        assert!(z.abs() > 3.0, "outlier should have high z-score, got {z}");
    }

    #[test]
    fn estimator_freeze_on_reject() {
        let mut rz = RobustZScoreF64::builder()
            .alpha(0.1)
            .reject_threshold(3.0)
            .min_samples(5)
            .build();

        for i in 0..20 {
            let _ = rz.update(100.0 + (i % 2) as f64);
        }

        let baseline_before = rz.baseline().unwrap();

        // Feed outlier that exceeds reject threshold
        let _ = rz.update(500.0);

        let baseline_after = rz.baseline().unwrap();
        assert!(
            (baseline_before - baseline_after).abs() < 1e-10,
            "baseline should not move on rejected sample"
        );
    }

    #[test]
    fn recovery_after_freeze() {
        let mut rz = RobustZScoreF64::builder()
            .alpha(0.1)
            .reject_threshold(5.0)
            .min_samples(5)
            .build();

        for _ in 0..20 {
            let _ = rz.update(100.0);
        }
        let _ = rz.update(500.0); // rejected

        // Normal samples resume — estimator should continue updating
        for _ in 0..10 {
            let _ = rz.update(100.0);
        }

        let z = rz.z_score().unwrap();
        assert!(z.abs() < 1.0, "should recover after freeze, got {z}");
    }

    #[test]
    fn priming() {
        let mut rz = RobustZScoreF64::builder()
            .alpha(0.1)
            .reject_threshold(5.0)
            .min_samples(10)
            .build();

        for _ in 0..9 {
            assert!(rz.update(100.0).is_none());
        }
        assert!(rz.update(100.0).is_some());
    }

    #[test]
    fn reset() {
        let mut rz = RobustZScoreF64::builder()
            .alpha(0.1)
            .reject_threshold(5.0)
            .min_samples(5)
            .build();

        for _ in 0..20 {
            let _ = rz.update(100.0);
        }
        rz.reset();
        assert_eq!(rz.count(), 0);
    }

    #[test]
    fn f32_basic() {
        let mut rz = RobustZScoreF32::builder()
            .alpha(0.1)
            .reject_threshold(5.0)
            .min_samples(5)
            .build();

        for _ in 0..10 {
            let _ = rz.update(100.0);
        }
        assert!(rz.is_primed());
    }

    #[test]
    #[should_panic(expected = "reject_threshold must be set")]
    fn panics_without_reject_threshold() {
        let _ = RobustZScoreF64::builder().alpha(0.1).build();
    }
}
