// Gamma-Poisson — Bayesian Event Rate Estimation
//
// Conjugate prior: Gamma(alpha, beta) with Poisson likelihood.
// Posterior after observing k events in t exposure: Gamma(alpha + k, beta + t).
// Posterior mean rate = alpha / beta.
//
// 32 bytes per instance. Zero allocation.

#![allow(clippy::suboptimal_flops, clippy::float_cmp, clippy::neg_cmp_op_on_partial_ord)]

macro_rules! impl_gamma_poisson {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Bayesian event rate estimator using the Gamma-Poisson conjugate prior.
        ///
        /// Maintains a Gamma posterior over the Poisson rate parameter.
        /// Each observation adds event counts and exposure time, updating the
        /// posterior analytically — no sampling, no iteration.
        ///
        /// # Use Cases
        /// - "What's the expected message arrival rate given what we've seen?"
        /// - Estimating fill rates, error rates, or tick rates with uncertainty
        /// - Credible intervals on event rates from limited data
        ///
        /// # Complexity
        /// - O(1) per observation, O(1) per query.
        /// - 32 bytes state (f64), 16 bytes (f32). Zero allocation.
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use nexus_stats::", stringify!($name), ";")]
        ///
        #[doc = concat!("let mut gp = ", stringify!($name), "::new();")]
        /// gp.observe(100, 10.0);  // 100 events in 10 seconds
        /// let rate = gp.rate();
        /// // With weak prior (1,1), rate ≈ 101/11 ≈ 9.18
        #[doc = concat!("assert!((rate - 9.18 as ", stringify!($ty), ").abs() < 0.01 as ", stringify!($ty), ");")]
        /// ```
        #[derive(Debug, Clone)]
        pub struct $name {
            alpha: $ty,
            beta: $ty,
            prior_alpha: $ty,
            prior_beta: $ty,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: $ty,
            beta: $ty,
        }

        impl $name {
            /// Creates a builder with default prior (alpha=1, beta=1).
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    alpha: 1.0 as $ty,
                    beta: 1.0 as $ty,
                }
            }

            /// Creates an estimator with a weakly informative prior (alpha=1, beta=1).
            #[inline]
            #[must_use]
            pub fn new() -> Self {
                Self {
                    alpha: 1.0 as $ty,
                    beta: 1.0 as $ty,
                    prior_alpha: 1.0 as $ty,
                    prior_beta: 1.0 as $ty,
                }
            }

            /// Creates an estimator with a specific Gamma prior.
            ///
            /// # Errors
            ///
            /// Returns `ConfigError::Invalid` if `alpha <= 0` or `beta <= 0`.
            #[inline]
            pub fn with_prior(alpha: $ty, beta: $ty) -> Result<Self, crate::ConfigError> {
                if !(alpha > 0.0 as $ty) {
                    return Err(crate::ConfigError::Invalid("alpha must be > 0"));
                }
                if !(beta > 0.0 as $ty) {
                    return Err(crate::ConfigError::Invalid("beta must be > 0"));
                }
                Ok(Self {
                    alpha,
                    beta,
                    prior_alpha: alpha,
                    prior_beta: beta,
                })
            }

            /// Feeds an observation: `count` events observed over `exposure` time.
            ///
            /// Updates the posterior: alpha += count, beta += exposure.
            #[inline]
            pub fn observe(&mut self, count: u64, exposure: $ty) {
                self.alpha += count as $ty;
                self.beta += exposure;
            }

            /// Posterior mean rate (alpha / beta).
            #[inline]
            #[must_use]
            pub fn rate(&self) -> $ty {
                self.alpha / self.beta
            }

            /// Posterior variance (alpha / beta²).
            #[inline]
            #[must_use]
            pub fn variance(&self) -> $ty {
                self.alpha / (self.beta * self.beta)
            }

            /// Approximate credible interval using normal approximation.
            ///
            /// Returns `(lower, upper)` bounds for the given confidence level,
            /// or `None` if no exposure has been observed or confidence is not in (0, 1).
            ///
            /// Uses the Abramowitz & Stegun rational approximation for the
            /// inverse normal CDF.
            #[cfg(any(feature = "std", feature = "libm"))]
            #[inline]
            #[must_use]
            pub fn credible_interval(&self, confidence: $ty) -> Option<($ty, $ty)> {
                if self.total_exposure() <= 0.0 as $ty {
                    return Option::None;
                }
                if !(confidence > 0.0 as $ty && confidence < 1.0 as $ty) {
                    return Option::None;
                }

                let tail = (1.0 as $ty - confidence) / 2.0 as $ty;

                // Abramowitz & Stegun rational approximation for inverse normal
                #[allow(clippy::cast_possible_truncation)]
                let t = crate::math::sqrt(
                    -2.0 * crate::math::ln(tail as f64),
                ) as $ty;
                let z = t
                    - (2.515517 as $ty + 0.802853 as $ty * t + 0.010328 as $ty * t * t)
                        / (1.0 as $ty
                            + 1.432788 as $ty * t
                            + 0.189269 as $ty * t * t
                            + 0.001308 as $ty * t * t * t);

                #[allow(clippy::cast_possible_truncation)]
                let std_dev = crate::math::sqrt(self.variance() as f64) as $ty;
                let mean = self.rate();

                Option::Some((mean - z * std_dev, mean + z * std_dev))
            }

            /// Total event count observed (excluding prior).
            #[inline]
            #[must_use]
            pub fn total_count(&self) -> $ty {
                self.alpha - self.prior_alpha
            }

            /// Total exposure time observed (excluding prior).
            #[inline]
            #[must_use]
            pub fn total_exposure(&self) -> $ty {
                self.beta - self.prior_beta
            }

            /// Total event count as integer.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.total_count() as u64
            }

            /// Whether any exposure has been observed.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.total_exposure() > 0.0 as $ty
            }

            /// Resets to the original prior, discarding all observations.
            #[inline]
            pub fn reset(&mut self) {
                self.alpha = self.prior_alpha;
                self.beta = self.prior_beta;
            }
        }

        impl Default for $name {
            #[inline]
            fn default() -> Self {
                Self::new()
            }
        }

        impl $builder {
            /// Sets the shape parameter (prior alpha). Must be > 0.
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = alpha;
                self
            }

            /// Sets the rate parameter (prior beta). Must be > 0.
            #[inline]
            #[must_use]
            pub fn beta(mut self, beta: $ty) -> Self {
                self.beta = beta;
                self
            }

            /// Builds the estimator, validating parameters.
            ///
            /// # Errors
            ///
            /// Returns `ConfigError::Invalid` if alpha or beta are not positive.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                if !(self.alpha > 0.0 as $ty) {
                    return Err(crate::ConfigError::Invalid("alpha must be > 0"));
                }
                if !(self.beta > 0.0 as $ty) {
                    return Err(crate::ConfigError::Invalid("beta must be > 0"));
                }
                Ok($name {
                    alpha: self.alpha,
                    beta: self.beta,
                    prior_alpha: self.alpha,
                    prior_beta: self.beta,
                })
            }
        }
    };
}

impl_gamma_poisson!(GammaPoissonF64, GammaPoissonF64Builder, f64);
impl_gamma_poisson!(GammaPoissonF32, GammaPoissonF32Builder, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_after_observation() {
        let mut gp = GammaPoissonF64::new();
        gp.observe(100, 10.0);
        // Posterior: Gamma(1+100, 1+10) = Gamma(101, 11)
        // Mean = 101/11 ≈ 9.1818...
        let rate = gp.rate();
        assert!((rate - 101.0 / 11.0).abs() < 1e-10);
    }

    #[test]
    fn variance_decreases_with_exposure() {
        let mut gp = GammaPoissonF64::new();
        gp.observe(10, 1.0);
        let v1 = gp.variance();
        gp.observe(100, 10.0);
        let v2 = gp.variance();
        assert!(v2 < v1, "variance should decrease with more exposure");
    }

    #[cfg(any(feature = "std", feature = "libm"))]
    #[test]
    fn credible_interval_narrows_with_data() {
        let mut gp = GammaPoissonF64::new();
        gp.observe(10, 1.0);
        let (lo1, hi1) = gp.credible_interval(0.95).unwrap();
        let width1 = hi1 - lo1;

        gp.observe(1000, 100.0);
        let (lo2, hi2) = gp.credible_interval(0.95).unwrap();
        let width2 = hi2 - lo2;

        assert!(width2 < width1, "interval should narrow with more data");
        // Rate should be within the interval
        let rate = gp.rate();
        assert!(rate >= lo2 && rate <= hi2);
    }

    #[test]
    fn reset_restores_prior() {
        let mut gp = GammaPoissonF64::with_prior(2.0, 3.0).unwrap();
        gp.observe(50, 5.0);
        assert!(gp.count() > 0);
        gp.reset();
        assert_eq!(gp.count(), 0);
        assert_eq!(gp.total_exposure(), 0.0);
        assert_eq!(gp.rate(), 2.0 / 3.0);
    }

    #[test]
    fn with_prior_validation() {
        assert!(GammaPoissonF64::with_prior(0.0, 1.0).is_err());
        assert!(GammaPoissonF64::with_prior(-1.0, 1.0).is_err());
        assert!(GammaPoissonF64::with_prior(1.0, 0.0).is_err());
        assert!(GammaPoissonF64::with_prior(1.0, -1.0).is_err());
        assert!(GammaPoissonF64::with_prior(f64::NAN, 1.0).is_err());
        assert!(GammaPoissonF64::with_prior(1.0, f64::NAN).is_err());
        assert!(GammaPoissonF64::with_prior(1.0, 1.0).is_ok());
    }

    #[test]
    fn f32_variant() {
        let mut gp = GammaPoissonF32::new();
        gp.observe(50, 5.0);
        let rate = gp.rate();
        // Gamma(51, 6) → 51/6 = 8.5
        assert!((rate - 8.5_f32).abs() < 0.01);
    }

    #[test]
    fn default_is_new() {
        let a = GammaPoissonF64::new();
        let b = GammaPoissonF64::default();
        assert_eq!(a.rate(), b.rate());
        assert_eq!(a.variance(), b.variance());
    }

    #[test]
    fn batch_observation_accumulates() {
        let mut gp = GammaPoissonF64::new();
        gp.observe(10, 1.0);
        gp.observe(20, 2.0);
        gp.observe(30, 3.0);
        // Total: 60 events, 6.0 exposure
        assert_eq!(gp.count(), 60);
        assert!((gp.total_exposure() - 6.0).abs() < 1e-10);
        // Posterior: Gamma(61, 7) → 61/7
        assert!((gp.rate() - 61.0 / 7.0).abs() < 1e-10);
    }

    #[cfg(any(feature = "std", feature = "libm"))]
    #[test]
    fn credible_interval_none_without_exposure() {
        let gp = GammaPoissonF64::new();
        assert!(gp.credible_interval(0.95).is_none());
    }

    #[cfg(any(feature = "std", feature = "libm"))]
    #[test]
    fn credible_interval_none_for_invalid_confidence() {
        let mut gp = GammaPoissonF64::new();
        gp.observe(10, 1.0);
        assert!(gp.credible_interval(0.0).is_none());
        assert!(gp.credible_interval(1.0).is_none());
        assert!(gp.credible_interval(-0.5).is_none());
        assert!(gp.credible_interval(1.5).is_none());
    }

    #[test]
    fn builder_defaults() {
        let gp = GammaPoissonF64::builder().build().unwrap();
        assert_eq!(gp.rate(), 1.0); // Gamma(1,1) → mean = 1
    }

    #[test]
    fn builder_custom_prior() {
        let gp = GammaPoissonF64::builder()
            .alpha(5.0)
            .beta(2.0)
            .build()
            .unwrap();
        assert_eq!(gp.rate(), 2.5); // 5/2
    }

    #[test]
    fn builder_validation() {
        assert!(GammaPoissonF64::builder().alpha(0.0).build().is_err());
        assert!(GammaPoissonF64::builder().beta(-1.0).build().is_err());
    }
}
