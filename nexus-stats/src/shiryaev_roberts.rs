/// Shiryaev-Roberts — Quasi-Bayesian change detection procedure.
///
/// Theoretically optimal for detecting unknown change times. Assumes
/// normal distribution for the likelihood ratio computation.
///
/// The statistic R evolves as: `R = (1 + R) * likelihood_ratio(x)`
/// where the likelihood ratio compares pre-change and post-change
/// normal distributions.
///
/// # Use Cases
/// - Same as CUSUM but with better average detection delay
/// - When CUSUM's sensitivity vs false alarm tradeoff isn't good enough
#[derive(Debug, Clone)]
pub struct ShiryaevRobertsF64 {
    pre_mean: f64,
    post_mean: f64,
    variance: f64,
    threshold: f64,
    r: f64,
    count: u64,
    min_samples: u64,
}

/// Builder for [`ShiryaevRobertsF64`].
#[derive(Debug, Clone)]
pub struct ShiryaevRobertsF64Builder {
    pre_mean: Option<f64>,
    post_mean: Option<f64>,
    variance: Option<f64>,
    threshold: Option<f64>,
    min_samples: u64,
}

impl ShiryaevRobertsF64 {
    /// Creates a builder.
    #[inline]
    #[must_use]
    pub fn builder() -> ShiryaevRobertsF64Builder {
        ShiryaevRobertsF64Builder {
            pre_mean: None,
            post_mean: None,
            variance: None,
            threshold: None,
            min_samples: 0,
        }
    }

    /// Feeds a sample. Returns `Some(true)` if change detected, `Some(false)`
    /// if not, or `None` if not yet primed.
    ///
    /// The likelihood ratio for normal distributions is:
    /// ```text
    /// LR = exp((x - μ₀)(μ₁ - μ₀) / σ² - (μ₁² - μ₀²) / (2σ²))
    ///    = exp((μ₁ - μ₀) * (x - (μ₀ + μ₁) / 2) / σ²)
    /// ```
    #[inline]
    #[must_use]
    pub fn update(&mut self, sample: f64) -> Option<bool> {
        self.count += 1;

        // Log-likelihood ratio for normal distribution
        let delta_mean = self.post_mean - self.pre_mean;
        let midpoint = f64::midpoint(self.pre_mean, self.post_mean);
        let log_lr = delta_mean * (sample - midpoint) / self.variance;

        // R = (1 + R) * exp(log_lr)
        self.r = (1.0 + self.r) * crate::math::exp(log_lr);

        if self.count < self.min_samples {
            return None;
        }

        Some(self.r > self.threshold)
    }

    /// Current R statistic value.
    #[inline]
    #[must_use]
    pub fn statistic(&self) -> f64 {
        self.r
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

    /// Resets R statistic and count. Parameters unchanged.
    #[inline]
    pub fn reset(&mut self) {
        self.r = 0.0;
        self.count = 0;
    }
}

impl ShiryaevRobertsF64Builder {
    /// Pre-change (null hypothesis) mean.
    #[inline]
    #[must_use]
    pub fn pre_change_mean(mut self, mean: f64) -> Self {
        self.pre_mean = Some(mean);
        self
    }

    /// Post-change (alternative hypothesis) mean.
    #[inline]
    #[must_use]
    pub fn post_change_mean(mut self, mean: f64) -> Self {
        self.post_mean = Some(mean);
        self
    }

    /// Known variance of the process.
    #[inline]
    #[must_use]
    pub fn variance(mut self, variance: f64) -> Self {
        self.variance = Some(variance);
        self
    }

    /// Decision threshold for the R statistic.
    #[inline]
    #[must_use]
    pub fn threshold(mut self, threshold: f64) -> Self {
        self.threshold = Some(threshold);
        self
    }

    /// Minimum samples before detection activates. Default: 0.
    #[inline]
    #[must_use]
    pub fn min_samples(mut self, min: u64) -> Self {
        self.min_samples = min;
        self
    }

    /// Builds the detector.
    ///
    /// # Panics
    ///
    /// - All of `pre_change_mean`, `post_change_mean`, `variance`, and `threshold`
    ///   must be set.
    /// - Variance must be positive.
    /// - Threshold must be positive.
    /// - Pre-change and post-change means must differ.
    #[inline]
    #[must_use]
    pub fn build(self) -> ShiryaevRobertsF64 {
        let pre_mean = self.pre_mean.expect("pre_change_mean must be set");
        let post_mean = self.post_mean.expect("post_change_mean must be set");
        let variance = self.variance.expect("variance must be set");
        let threshold = self.threshold.expect("threshold must be set");

        assert!(variance > 0.0, "variance must be positive");
        assert!(threshold > 0.0, "threshold must be positive");
        assert!(
            (post_mean - pre_mean).abs() > f64::EPSILON,
            "pre and post change means must differ"
        );

        ShiryaevRobertsF64 {
            pre_mean,
            post_mean,
            variance,
            threshold,
            r: 0.0,
            count: 0,
            min_samples: self.min_samples,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_detection_at_pre_change_mean() {
        let mut sr = ShiryaevRobertsF64::builder()
            .pre_change_mean(100.0)
            .post_change_mean(110.0)
            .variance(25.0)
            .threshold(100.0)
            .build();

        for _ in 0..100 {
            let result = sr.update(100.0);
            assert_eq!(result, Some(false), "should not detect at pre-change mean");
        }
    }

    #[test]
    fn detects_shift_to_post_change_mean() {
        let mut sr = ShiryaevRobertsF64::builder()
            .pre_change_mean(100.0)
            .post_change_mean(110.0)
            .variance(25.0)
            .threshold(100.0)
            .build();

        let mut detected = false;
        for _ in 0..100 {
            if sr.update(110.0) == Some(true) {
                detected = true;
                break;
            }
        }
        assert!(detected, "should detect shift to post-change mean");
    }

    #[test]
    fn statistic_grows_under_alternative() {
        let mut sr = ShiryaevRobertsF64::builder()
            .pre_change_mean(0.0)
            .post_change_mean(5.0)
            .variance(1.0)
            .threshold(1000.0)
            .build();

        let _ = sr.update(5.0);
        let r1 = sr.statistic();
        let _ = sr.update(5.0);
        let r2 = sr.statistic();

        assert!(r2 > r1, "R should grow under alternative hypothesis");
    }

    #[test]
    fn reset_clears_statistic() {
        let mut sr = ShiryaevRobertsF64::builder()
            .pre_change_mean(0.0)
            .post_change_mean(5.0)
            .variance(1.0)
            .threshold(100.0)
            .build();

        let _ = sr.update(5.0);
        assert!(sr.statistic() > 0.0);

        sr.reset();
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(sr.statistic(), 0.0);
        }
        assert_eq!(sr.count(), 0);
    }

    #[test]
    fn priming() {
        let mut sr = ShiryaevRobertsF64::builder()
            .pre_change_mean(0.0)
            .post_change_mean(5.0)
            .variance(1.0)
            .threshold(100.0)
            .min_samples(5)
            .build();

        for _ in 0..4 {
            assert_eq!(sr.update(5.0), None);
        }
        assert!(sr.update(5.0).is_some());
    }

    #[test]
    #[should_panic(expected = "variance must be set")]
    fn panics_without_variance() {
        let _ = ShiryaevRobertsF64::builder()
            .pre_change_mean(0.0)
            .post_change_mean(5.0)
            .threshold(100.0)
            .build();
    }

    #[test]
    #[should_panic(expected = "pre and post change means must differ")]
    fn panics_on_equal_means() {
        let _ = ShiryaevRobertsF64::builder()
            .pre_change_mean(5.0)
            .post_change_mean(5.0)
            .variance(1.0)
            .threshold(100.0)
            .build();
    }
}
