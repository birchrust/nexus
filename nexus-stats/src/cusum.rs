/// Direction of a detected mean shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    /// No shift detected.
    None,
    /// Mean shifted upward (e.g., latency increased).
    Upper,
    /// Mean shifted downward (e.g., latency recovered).
    Lower,
}

macro_rules! impl_cusum {
    ($name:ident, $builder:ident, $ty:ty, min_slack = $min_slack:expr) => {
        /// CUSUM — Cumulative Sum change detector (Page, 1954).
        ///
        /// Detects persistent shifts in the mean of a streaming process
        /// in either direction. Signals when cumulative deviation from a
        /// target exceeds a threshold.
        ///
        /// Supports asymmetric slack and threshold parameters for different
        /// sensitivity to upward vs downward shifts.
        ///
        /// # Use Cases
        /// - Exchange ack latency degradation (detect shift up)
        /// - Recovery detection (detect shift back down)
        /// - Market data feed quality monitoring
        #[derive(Debug, Clone)]
        pub struct $name {
            target: $ty,
            slack_upper: $ty,
            slack_lower: $ty,
            threshold_upper: $ty,
            threshold_lower: $ty,
            upper: $ty,
            lower: $ty,
            count: u64,
            min_samples: u64,
            // Track whether user explicitly set slack/threshold so
            // reset_with_target knows whether to recompute defaults.
            slack_upper_explicit: bool,
            slack_lower_explicit: bool,
            threshold_upper_explicit: bool,
            threshold_lower_explicit: bool,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        ///
        /// # Example
        ///
        /// ```
        /// use nexus_stats::*;
        #[doc = concat!("let mut cusum = ", stringify!($name), "::builder(100 as ", stringify!($ty), ")")]
        ///     .slack(5 as _)
        ///     .threshold(50 as _)
        ///     .min_samples(20)
        ///     .build();
        /// ```
        #[derive(Debug, Clone)]
        pub struct $builder {
            target: $ty,
            slack_upper: Option<$ty>,
            slack_lower: Option<$ty>,
            threshold_upper: Option<$ty>,
            threshold_lower: Option<$ty>,
            min_samples: u64,
            seed_upper: Option<$ty>,
            seed_lower: Option<$ty>,
        }

        impl $name {
            /// Creates a builder with the target (expected baseline mean).
            #[inline]
            #[must_use]
            pub fn builder(target: $ty) -> $builder {
                $builder {
                    target,
                    slack_upper: Option::None,
                    slack_lower: Option::None,
                    threshold_upper: Option::None,
                    threshold_lower: Option::None,
                    min_samples: 0,
                    seed_upper: Option::None,
                    seed_lower: Option::None,
                }
            }

            /// Feeds a sample. Returns shift direction once primed.
            ///
            /// Returns `None` until `min_samples` have been processed.
            /// After priming, returns `Some(Shift::Upper)`, `Some(Shift::Lower)`,
            /// or `Some(Shift::None)`.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<Shift> {
                self.count += 1;

                let diff = sample - self.target;

                // S_high = max(0, S_high + (x - target) - slack_upper)
                let s_high = self.upper + diff - self.slack_upper;
                self.upper = if s_high > (0 as $ty) { s_high } else { 0 as $ty };

                // S_low = max(0, S_low + (target - x) - slack_lower)
                let s_low = self.lower - diff - self.slack_lower;
                self.lower = if s_low > (0 as $ty) { s_low } else { 0 as $ty };

                if self.count < self.min_samples {
                    return Option::None;
                }

                if self.upper > self.threshold_upper {
                    Option::Some(Shift::Upper)
                } else if self.lower > self.threshold_lower {
                    Option::Some(Shift::Lower)
                } else {
                    Option::Some(Shift::None)
                }
            }

            /// Upper cumulative sum (tracks upward drift).
            #[inline]
            #[must_use]
            pub fn upper(&self) -> $ty {
                self.upper
            }

            /// Lower cumulative sum (tracks downward drift).
            #[inline]
            #[must_use]
            pub fn lower(&self) -> $ty {
                self.lower
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether the detector has reached `min_samples`.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count >= self.min_samples
            }

            /// Resets cumulative sums and count to zero. Parameters unchanged.
            #[inline]
            pub fn reset(&mut self) {
                self.upper = 0 as $ty;
                self.lower = 0 as $ty;
                self.count = 0;
            }

            /// Resets and updates the target mean.
            ///
            /// If slack or threshold were not explicitly set by the user,
            /// they are recomputed from the new target using the defaults
            /// (5% and 50% of target respectively).
            #[inline]
            pub fn reset_with_target(&mut self, new_target: $ty) {
                self.target = new_target;
                self.upper = 0 as $ty;
                self.lower = 0 as $ty;
                self.count = 0;

                if !self.slack_upper_explicit {
                    self.slack_upper = $builder::default_slack(new_target);
                }
                if !self.slack_lower_explicit {
                    self.slack_lower = $builder::default_slack(new_target);
                }
                if !self.threshold_upper_explicit {
                    self.threshold_upper = $builder::default_threshold(new_target);
                }
                if !self.threshold_lower_explicit {
                    self.threshold_lower = $builder::default_threshold(new_target);
                }
            }

            /// The target (expected baseline mean).
            #[inline]
            #[must_use]
            pub fn target(&self) -> $ty {
                self.target
            }

            /// Upper slack parameter.
            #[inline]
            #[must_use]
            pub fn slack_upper(&self) -> $ty {
                self.slack_upper
            }

            /// Lower slack parameter.
            #[inline]
            #[must_use]
            pub fn slack_lower(&self) -> $ty {
                self.slack_lower
            }

            /// Upper threshold parameter.
            #[inline]
            #[must_use]
            pub fn threshold_upper(&self) -> $ty {
                self.threshold_upper
            }

            /// Lower threshold parameter.
            #[inline]
            #[must_use]
            pub fn threshold_lower(&self) -> $ty {
                self.threshold_lower
            }

            /// Minimum samples required before detection activates.
            #[inline]
            #[must_use]
            pub fn min_samples(&self) -> u64 {
                self.min_samples
            }
        }

        impl $builder {
            #[inline]
            fn default_slack(target: $ty) -> $ty {
                // 5% of target magnitude, floored to $min_slack
                let abs_target = if target < (0 as $ty) { (0 as $ty) - target } else { target };
                let slack = abs_target / (20 as $ty);
                if slack < ($min_slack as $ty) { $min_slack as $ty } else { slack }
            }

            #[inline]
            fn default_threshold(target: $ty) -> $ty {
                // 50% of target magnitude
                let abs_target = if target < (0 as $ty) { (0 as $ty) - target } else { target };
                abs_target / (2 as $ty)
            }

            /// Sets both upper and lower slack (symmetric sensitivity).
            ///
            /// Slack controls sensitivity — smaller values detect smaller shifts
            /// but increase false alarm rate. Typically set to half the minimum
            /// shift you want to detect.
            #[inline]
            #[must_use]
            pub fn slack(mut self, slack: $ty) -> Self {
                self.slack_upper = Option::Some(slack);
                self.slack_lower = Option::Some(slack);
                self
            }

            /// Sets the upper slack independently.
            ///
            /// Controls sensitivity to upward shifts only.
            #[inline]
            #[must_use]
            pub fn slack_upper(mut self, slack: $ty) -> Self {
                self.slack_upper = Option::Some(slack);
                self
            }

            /// Sets the lower slack independently.
            ///
            /// Controls sensitivity to downward shifts only.
            #[inline]
            #[must_use]
            pub fn slack_lower(mut self, slack: $ty) -> Self {
                self.slack_lower = Option::Some(slack);
                self
            }

            /// Sets both upper and lower thresholds (symmetric decision boundary).
            ///
            /// Larger thresholds mean fewer false alarms but slower detection.
            #[inline]
            #[must_use]
            pub fn threshold(mut self, threshold: $ty) -> Self {
                self.threshold_upper = Option::Some(threshold);
                self.threshold_lower = Option::Some(threshold);
                self
            }

            /// Sets the upper threshold independently.
            ///
            /// Decision boundary for upward shift detection only.
            #[inline]
            #[must_use]
            pub fn threshold_upper(mut self, threshold: $ty) -> Self {
                self.threshold_upper = Option::Some(threshold);
                self
            }

            /// Sets the lower threshold independently.
            ///
            /// Decision boundary for downward shift detection only.
            #[inline]
            #[must_use]
            pub fn threshold_lower(mut self, threshold: $ty) -> Self {
                self.threshold_lower = Option::Some(threshold);
                self
            }

            /// Minimum samples before detection activates. Default: 0.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Pre-loads the upper cumulative sum from calibration data.
            ///
            /// When seeded, `is_primed()` returns true immediately.
            #[inline]
            #[must_use]
            pub fn seed_upper(mut self, val: $ty) -> Self {
                self.seed_upper = Option::Some(val);
                self
            }

            /// Pre-loads the lower cumulative sum from calibration data.
            ///
            /// When seeded, `is_primed()` returns true immediately.
            #[inline]
            #[must_use]
            pub fn seed_lower(mut self, val: $ty) -> Self {
                self.seed_lower = Option::Some(val);
                self
            }

            /// Builds the detector.
            ///
            /// # Panics
            ///
            /// - Slack values must be non-negative.
            /// - Threshold values must be positive.
            #[inline]
            #[must_use]
            pub fn build(self) -> $name {
                let slack_upper_explicit = self.slack_upper.is_some();
                let slack_lower_explicit = self.slack_lower.is_some();
                let threshold_upper_explicit = self.threshold_upper.is_some();
                let threshold_lower_explicit = self.threshold_lower.is_some();

                let slack_upper = self.slack_upper.unwrap_or_else(|| Self::default_slack(self.target));
                let slack_lower = self.slack_lower.unwrap_or_else(|| Self::default_slack(self.target));
                let threshold_upper = self.threshold_upper.unwrap_or_else(|| Self::default_threshold(self.target));
                let threshold_lower = self.threshold_lower.unwrap_or_else(|| Self::default_threshold(self.target));

                assert!(slack_upper >= (0 as $ty), "slack_upper must be non-negative");
                assert!(slack_lower >= (0 as $ty), "slack_lower must be non-negative");
                assert!(threshold_upper > (0 as $ty), "threshold_upper must be positive");
                assert!(threshold_lower > (0 as $ty), "threshold_lower must be positive");

                let seeded = self.seed_upper.is_some() || self.seed_lower.is_some();
                let initial_count = if seeded { self.min_samples } else { 0 };

                $name {
                    target: self.target,
                    slack_upper,
                    slack_lower,
                    threshold_upper,
                    threshold_lower,
                    upper: self.seed_upper.unwrap_or(0 as $ty),
                    lower: self.seed_lower.unwrap_or(0 as $ty),
                    count: initial_count,
                    min_samples: self.min_samples,
                    slack_upper_explicit,
                    slack_lower_explicit,
                    threshold_upper_explicit,
                    threshold_lower_explicit,
                }
            }
        }
    };
}

impl_cusum!(CusumF64, CusumF64Builder, f64, min_slack = 0.0);
impl_cusum!(CusumF32, CusumF32Builder, f32, min_slack = 0.0);
impl_cusum!(CusumI64, CusumI64Builder, i64, min_slack = 1);
impl_cusum!(CusumI32, CusumI32Builder, i32, min_slack = 1);

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Basic shift detection
    // =========================================================================

    #[test]
    fn detects_upward_shift() {
        let mut cusum = CusumF64::builder(100.0)
            .slack(5.0)
            .threshold(50.0)
            .build();

        // Feed normal samples — should not trigger
        for _ in 0..10 {
            let result = cusum.update(100.0);
            assert_eq!(result, Some(Shift::None));
        }

        // Feed elevated samples — should eventually trigger upper
        let mut triggered = false;
        for _ in 0..100 {
            if cusum.update(120.0) == Some(Shift::Upper) {
                triggered = true;
                break;
            }
        }
        assert!(triggered, "should have detected upward shift");
    }

    #[test]
    fn detects_downward_shift() {
        let mut cusum = CusumF64::builder(100.0)
            .slack(5.0)
            .threshold(50.0)
            .build();

        // Feed depressed samples — should eventually trigger lower
        let mut triggered = false;
        for _ in 0..100 {
            if cusum.update(80.0) == Some(Shift::Lower) {
                triggered = true;
                break;
            }
        }
        assert!(triggered, "should have detected downward shift");
    }

    #[test]
    fn no_false_positive_at_target() {
        let mut cusum = CusumF64::builder(100.0)
            .slack(5.0)
            .threshold(50.0)
            .build();

        for _ in 0..1000 {
            assert_eq!(cusum.update(100.0), Some(Shift::None));
        }
    }

    // =========================================================================
    // Priming behavior
    // =========================================================================

    #[test]
    fn returns_none_before_primed() {
        let mut cusum = CusumF64::builder(100.0)
            .slack(5.0)
            .threshold(50.0)
            .min_samples(10)
            .build();

        for _ in 0..9 {
            assert_eq!(cusum.update(200.0), None);
        }
        assert!(!cusum.is_primed());

        // 10th sample should be primed
        let result = cusum.update(200.0);
        assert!(result.is_some());
        assert!(cusum.is_primed());
    }

    #[test]
    fn primed_immediately_with_zero_min_samples() {
        let mut cusum = CusumF64::builder(100.0)
            .slack(5.0)
            .threshold(50.0)
            .build();

        assert_eq!(cusum.min_samples(), 0);
        // First sample should return Some
        assert!(cusum.update(100.0).is_some());
    }

    // =========================================================================
    // Reset
    // =========================================================================

    #[test]
    #[allow(clippy::float_cmp)]
    fn reset_clears_state() {
        let mut cusum = CusumF64::builder(100.0)
            .slack(5.0)
            .threshold(50.0)
            .build();

        for _ in 0..10 {
            let _ = cusum.update(120.0);
        }
        assert!(cusum.upper() > 0.0);
        assert!(cusum.count() > 0);

        cusum.reset();
        assert_eq!(cusum.upper(), 0.0);
        assert_eq!(cusum.lower(), 0.0);
        assert_eq!(cusum.count(), 0);
    }

    #[test]
    fn reset_with_target_updates_defaults() {
        let mut cusum = CusumF64::builder(100.0).build();

        // Defaults based on 100.0
        let original_slack = cusum.slack_upper();
        let original_threshold = cusum.threshold_upper();

        cusum.reset_with_target(200.0);

        // Defaults should scale with new target
        assert!(cusum.slack_upper() > original_slack);
        assert!(cusum.threshold_upper() > original_threshold);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn reset_with_target_preserves_explicit_params() {
        let mut cusum = CusumF64::builder(100.0)
            .slack(10.0)
            .threshold(75.0)
            .build();

        cusum.reset_with_target(200.0);

        // Explicit values should not change
        assert_eq!(cusum.slack_upper(), 10.0);
        assert_eq!(cusum.slack_lower(), 10.0);
        assert_eq!(cusum.threshold_upper(), 75.0);
        assert_eq!(cusum.threshold_lower(), 75.0);
    }

    // =========================================================================
    // Asymmetric slack and threshold
    // =========================================================================

    #[test]
    fn asymmetric_slack() {
        // Tight upper slack (sensitive to increases), loose lower slack
        let mut cusum = CusumF64::builder(100.0)
            .slack_upper(2.0)
            .slack_lower(10.0)
            .threshold(50.0)
            .build();

        // Small upward deviation should accumulate faster than downward
        for _ in 0..10 {
            let _ = cusum.update(110.0);
        }
        let upper_after = cusum.upper();

        cusum.reset();
        for _ in 0..10 {
            let _ = cusum.update(90.0);
        }
        let lower_after = cusum.lower();

        // Upper should accumulate more (slack_upper=2 eats less of the deviation)
        assert!(upper_after > lower_after,
            "upper ({upper_after}) should accumulate faster than lower ({lower_after}) with tighter slack");
    }

    #[test]
    fn asymmetric_threshold() {
        let mut cusum = CusumF64::builder(100.0)
            .slack(5.0)
            .threshold_upper(20.0)    // trigger fast on increases
            .threshold_lower(500.0)   // very slow to trigger on decreases
            .build();

        // Upward shift should trigger quickly
        let mut upper_triggered = false;
        for _ in 0..20 {
            if cusum.update(120.0) == Some(Shift::Upper) {
                upper_triggered = true;
                break;
            }
        }
        assert!(upper_triggered);

        // Downward shift should NOT trigger with same number of samples
        // deviation per sample: 15, over 20 samples = 300 < 500
        cusum.reset();
        let mut lower_triggered = false;
        for _ in 0..20 {
            if cusum.update(80.0) == Some(Shift::Lower) {
                lower_triggered = true;
                break;
            }
        }
        assert!(!lower_triggered, "lower should not trigger with high threshold");
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn symmetric_slack_sets_both() {
        let cusum = CusumF64::builder(100.0)
            .slack(7.5)
            .build();

        assert_eq!(cusum.slack_upper(), 7.5);
        assert_eq!(cusum.slack_lower(), 7.5);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn symmetric_threshold_sets_both() {
        let cusum = CusumF64::builder(100.0)
            .threshold(42.0)
            .build();

        assert_eq!(cusum.threshold_upper(), 42.0);
        assert_eq!(cusum.threshold_lower(), 42.0);
    }

    // =========================================================================
    // Builder validation
    // =========================================================================

    #[test]
    #[should_panic(expected = "slack_upper must be non-negative")]
    fn panics_on_negative_slack_upper() {
        let _ = CusumF64::builder(100.0).slack_upper(-1.0).build();
    }

    #[test]
    #[should_panic(expected = "slack_lower must be non-negative")]
    fn panics_on_negative_slack_lower() {
        let _ = CusumF64::builder(100.0).slack_lower(-1.0).build();
    }

    #[test]
    #[should_panic(expected = "threshold_upper must be positive")]
    fn panics_on_zero_threshold() {
        let _ = CusumF64::builder(100.0).threshold(0.0).build();
    }

    #[test]
    #[should_panic(expected = "threshold_lower must be positive")]
    fn panics_on_negative_threshold_lower() {
        let _ = CusumF64::builder(100.0).threshold_lower(-1.0).build();
    }

    // =========================================================================
    // Integer variants
    // =========================================================================

    #[test]
    fn i64_detects_upward_shift() {
        let mut cusum = CusumI64::builder(1000)
            .slack(50)
            .threshold(500)
            .build();

        let mut triggered = false;
        for _ in 0..100 {
            if cusum.update(1200) == Some(Shift::Upper) {
                triggered = true;
                break;
            }
        }
        assert!(triggered);
    }

    #[test]
    fn i32_basic() {
        let mut cusum = CusumI32::builder(100)
            .slack(5)
            .threshold(50)
            .build();

        assert_eq!(cusum.update(100), Some(Shift::None));
    }

    #[test]
    fn f32_basic() {
        let mut cusum = CusumF32::builder(100.0)
            .slack(5.0)
            .threshold(50.0)
            .build();

        assert_eq!(cusum.update(100.0), Some(Shift::None));
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn count_increments() {
        let mut cusum = CusumF64::builder(100.0)
            .slack(5.0)
            .threshold(50.0)
            .build();

        assert_eq!(cusum.count(), 0);
        let _ = cusum.update(100.0);
        assert_eq!(cusum.count(), 1);
        let _ = cusum.update(100.0);
        assert_eq!(cusum.count(), 2);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn upper_and_lower_start_at_zero() {
        let cusum = CusumF64::builder(100.0)
            .slack(5.0)
            .threshold(50.0)
            .build();

        assert_eq!(cusum.upper(), 0.0);
        assert_eq!(cusum.lower(), 0.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn cusum_at_exactly_slack_no_accumulation() {
        let mut cusum = CusumF64::builder(100.0)
            .slack(5.0)
            .threshold(50.0)
            .build();

        // Deviation exactly equals slack — S_high = max(0, 0 + 5 - 5) = 0
        let _ = cusum.update(105.0);
        assert_eq!(cusum.upper(), 0.0);
    }

    #[test]
    fn integer_default_slack_floor() {
        // target=10, 10/20 = 0 would truncate, but floor is 1
        // Ensures at least 1 unit of noise tolerance for integer types
        let cusum = CusumI64::builder(10)
            .threshold(5)
            .build();

        assert_eq!(cusum.slack_upper(), 1);
        assert_eq!(cusum.slack_lower(), 1);

        // Larger target: 100/20 = 5, no floor needed
        let cusum = CusumI64::builder(100)
            .threshold(50)
            .build();
        assert_eq!(cusum.slack_upper(), 5);
    }
}
