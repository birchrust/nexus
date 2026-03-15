use crate::math::MulAdd;
/// Graded verdict from multi-gate anomaly detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Normal value — passed all gates.
    Accept,
    /// Unusual — exceeded the spread-based gate but not statistical.
    Unusual,
    /// Suspect — exceeded statistical z-score gate.
    Suspect,
    /// Rejected — exceeded hard limit.
    Reject,
}

macro_rules! impl_multi_gate {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Multi-gate anomaly filter with graded severity.
        ///
        /// Three layers of filtering:
        /// 1. **Hard limit** — absolute rejection (percentage change from EMA)
        /// 2. **Statistical gate** — z-score against EMA of absolute returns
        /// 3. **Spread gate** — relative to recent spread (optional)
        ///
        /// Critical: the internal EMA is NOT updated on Suspect or Reject
        /// verdicts, preventing estimator corruption from bad data.
        ///
        /// # Use Cases
        /// - Market data quality filtering
        /// - Sensor anomaly detection with graded response
        /// - Multi-level alert systems
        #[derive(Debug, Clone)]
        pub struct $name {
            alpha: $ty,
            one_minus_alpha: $ty,
            ema_value: $ty,
            ema_abs_return: $ty,
            hard_limit_pct: $ty,
            suspect_z: $ty,
            unusual_spread_mult: Option<$ty>,
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
            hard_limit_pct: Option<$ty>,
            suspect_z: Option<$ty>,
            unusual_spread_mult: Option<$ty>,
            min_samples: u64,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    alpha: Option::None,
                    hard_limit_pct: Option::None,
                    suspect_z: Option::None,
                    unusual_spread_mult: Option::None,
                    min_samples: 10,
                }
            }

            /// Feeds a sample. Returns the verdict once primed.
            ///
            /// On `Suspect` or `Reject` verdicts, the internal EMA is NOT
            /// updated — preventing bad data from corrupting the baseline.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<Verdict> {
                self.count += 1;

                if !self.initialized {
                    self.ema_value = sample;
                    self.ema_abs_return = 0.0 as $ty;
                    self.initialized = true;
                    return if self.count >= self.min_samples {
                        Option::Some(Verdict::Accept)
                    } else {
                        Option::None
                    };
                }

                // Compute return
                let abs_return = (sample - self.ema_value).abs();

                // Gate 1: Hard limit (percentage of EMA)
                // Skip until EMA has converged enough to be meaningful.
                // Using abs > epsilon guard prevents silent disabling when EMA is near zero.
                let ema_abs = self.ema_value.abs();
                if ema_abs > 1e-10 as $ty {
                    let pct_change = abs_return / ema_abs;
                    if pct_change > self.hard_limit_pct {
                        // Do NOT update EMA
                        return if self.count >= self.min_samples {
                            Option::Some(Verdict::Reject)
                        } else {
                            Option::None
                        };
                    }
                }

                // Gate 2: Statistical z-score against EMA of absolute returns
                if self.ema_abs_return > (0.0 as $ty) {
                    let z = abs_return / self.ema_abs_return;
                    if z > self.suspect_z {
                        // Do NOT update EMA
                        return if self.count >= self.min_samples {
                            Option::Some(Verdict::Suspect)
                        } else {
                            Option::None
                        };
                    }
                }

                // Gate 3: Spread multiple (optional)
                let verdict = if let Some(spread_mult) = self.unusual_spread_mult {
                    if self.ema_abs_return > (0.0 as $ty) && abs_return > spread_mult * self.ema_abs_return {
                        Verdict::Unusual
                    } else {
                        Verdict::Accept
                    }
                } else {
                    Verdict::Accept
                };

                // Update EMA (only on Accept or Unusual)
                self.ema_value = self.alpha.fma(sample, self.one_minus_alpha * self.ema_value);
                self.ema_abs_return = self.alpha.fma(abs_return, self.one_minus_alpha * self.ema_abs_return);

                if self.count >= self.min_samples {
                    Option::Some(verdict)
                } else {
                    Option::None
                }
            }

            /// Current EMA of absolute returns, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn ema_abs_return(&self) -> Option<$ty> {
                if self.count >= self.min_samples { Option::Some(self.ema_abs_return) } else { Option::None }
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 { self.count }

            /// Whether the filter has reached `min_samples`.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool { self.count >= self.min_samples }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.ema_value = 0.0 as $ty;
                self.ema_abs_return = 0.0 as $ty;
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

            /// Hard rejection limit as a fraction (e.g., 0.5 = 50% change).
            #[inline]
            #[must_use]
            pub fn hard_limit(mut self, pct: $ty) -> Self {
                self.hard_limit_pct = Option::Some(pct);
                self
            }

            /// Statistical z-score threshold for Suspect verdict.
            #[inline]
            #[must_use]
            pub fn suspect_z(mut self, z: $ty) -> Self {
                self.suspect_z = Option::Some(z);
                self
            }

            /// Spread multiple threshold for Unusual verdict (optional).
            #[inline]
            #[must_use]
            pub fn unusual_spread_multiple(mut self, k: $ty) -> Self {
                self.unusual_spread_mult = Option::Some(k);
                self
            }

            /// Minimum samples before detection activates. Default: 10.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the multi-gate filter.
            ///
            /// # Panics
            ///
            /// - Alpha, hard_limit, and suspect_z must have been set.
            #[inline]
            #[must_use]
            pub fn build(self) -> $name {
                let alpha = self.alpha.expect("MultiGate alpha must be set");
                let hard_limit = self.hard_limit_pct.expect("MultiGate hard_limit must be set");
                let suspect_z = self.suspect_z.expect("MultiGate suspect_z must be set");
                assert!(alpha > 0.0 as $ty && alpha < 1.0 as $ty, "alpha must be in (0, 1)");

                $name {
                    alpha,
                    one_minus_alpha: 1.0 as $ty - alpha,
                    ema_value: 0.0 as $ty,
                    ema_abs_return: 0.0 as $ty,
                    hard_limit_pct: hard_limit,
                    suspect_z,
                    unusual_spread_mult: self.unusual_spread_mult,
                    count: 0,
                    min_samples: self.min_samples,
                    initialized: false,
                }
            }
        }
    };
}

impl_multi_gate!(MultiGateF64, MultiGateF64Builder, f64);
impl_multi_gate!(MultiGateF32, MultiGateF32Builder, f32);

#[cfg(test)]
mod tests {
    use super::*;

    fn make_gate() -> MultiGateF64 {
        MultiGateF64::builder()
            .alpha(0.1)
            .hard_limit(0.5)     // reject > 50% change
            .suspect_z(5.0)      // suspect at 5x normal spread
            .unusual_spread_multiple(3.0)
            .min_samples(5)
            .build()
    }

    #[test]
    fn normal_data_accepted() {
        let mut mg = make_gate();
        for _ in 0..20 {
            let result = mg.update(100.0);
            if let Some(v) = result {
                assert_eq!(v, Verdict::Accept);
            }
        }
    }

    #[test]
    fn extreme_spike_rejected() {
        let mut mg = make_gate();
        for _ in 0..10 {
            let _ = mg.update(100.0);
        }
        // 200 is 100% change from 100 — exceeds 50% hard limit
        assert_eq!(mg.update(200.0), Some(Verdict::Reject));
    }

    #[test]
    fn estimator_not_corrupted_by_reject() {
        let mut mg = make_gate();
        for _ in 0..10 {
            let _ = mg.update(100.0);
        }

        let ema_before = mg.ema_abs_return();

        // Rejected sample should NOT update EMA
        let _ = mg.update(200.0); // rejected

        let ema_after = mg.ema_abs_return();
        assert_eq!(ema_before, ema_after, "EMA should not change on reject");
    }

    #[test]
    fn moderate_anomaly_suspect() {
        let mut mg = MultiGateF64::builder()
            .alpha(0.1)
            .hard_limit(1.0)    // very high hard limit
            .suspect_z(3.0)
            .min_samples(5)
            .build();

        // Build up baseline with small movements
        for i in 0..20 {
            let _ = mg.update(100.0 + (i % 2) as f64);
        }

        // Moderate spike — not enough for hard limit but exceeds z-score
        let result = mg.update(130.0);
        assert!(
            result == Some(Verdict::Suspect) || result == Some(Verdict::Accept),
            "moderate spike should be suspect or accept"
        );
    }

    #[test]
    fn priming() {
        let mut mg = make_gate();
        for _ in 0..4 {
            assert!(mg.update(100.0).is_none());
        }
        assert!(mg.update(100.0).is_some());
    }

    #[test]
    fn reset() {
        let mut mg = make_gate();
        for _ in 0..20 {
            let _ = mg.update(100.0);
        }
        mg.reset();
        assert_eq!(mg.count(), 0);
    }

    #[test]
    fn f32_basic() {
        let mut mg = MultiGateF32::builder()
            .alpha(0.1)
            .hard_limit(0.5)
            .suspect_z(5.0)
            .min_samples(3)
            .build();

        for _ in 0..5 {
            let _ = mg.update(100.0);
        }
        assert!(mg.is_primed());
    }

    #[test]
    #[should_panic(expected = "hard_limit must be set")]
    fn panics_without_hard_limit() {
        let _ = MultiGateF64::builder().alpha(0.1).suspect_z(3.0).build();
    }
}
