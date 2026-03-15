// =============================================================================
// Float EMA
// =============================================================================

macro_rules! impl_ema_float {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// EMA — Exponential Moving Average.
        ///
        /// Smooths a streaming signal with exponential decay. Recent samples
        /// weighted more heavily. Equivalent to a first-order IIR low-pass filter.
        ///
        /// # Construction
        ///
        /// Three ways to configure the smoothing factor:
        /// - `alpha(a)` — direct, a ∈ (0, 1). Higher = more reactive.
        /// - `halflife(h)` — samples for weight to decay by half.
        /// - `span(n)` — pandas/finance convention, alpha = 2/(n+1).
        ///
        /// # Use Cases
        /// - Smoothing noisy latency measurements
        /// - Tracking moving average of throughput
        /// - Baseline estimation for anomaly detection
        #[derive(Debug, Clone)]
        pub struct $name {
            alpha: $ty,
            one_minus_alpha: $ty,
            value: $ty,
            count: u64,
            min_samples: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: Option<$ty>,
            min_samples: u64,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    alpha: Option::None,
                    min_samples: 1,
                }
            }

            /// Feeds a sample. Returns smoothed value once primed.
            ///
            /// First sample initializes the EMA directly (no smoothing).
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<$ty> {
                self.count += 1;

                if self.count == 1 {
                    self.value = sample;
                } else {
                    self.value = self.alpha.mul_add(sample, self.one_minus_alpha * self.value);
                }

                if self.count >= self.min_samples {
                    Option::Some(self.value)
                } else {
                    Option::None
                }
            }

            /// Current smoothed value, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn value(&self) -> Option<$ty> {
                if self.count >= self.min_samples {
                    Option::Some(self.value)
                } else {
                    Option::None
                }
            }

            /// The smoothing factor alpha.
            #[inline]
            #[must_use]
            pub fn alpha(&self) -> $ty {
                self.alpha
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether the EMA has reached `min_samples`.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count >= self.min_samples
            }

            /// Resets to uninitialized state. Parameters unchanged.
            #[inline]
            pub fn reset(&mut self) {
                self.value = 0.0 as $ty;
                self.count = 0;
            }
        }

        impl $builder {
            /// Direct smoothing factor. Must be in (0, 1) exclusive.
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = Option::Some(alpha);
                self
            }

            /// Samples for weight to decay by half.
            ///
            /// Computes: `alpha = 1 - exp(-ln(2) / halflife)`
            #[inline]
            #[must_use]
            pub fn halflife(mut self, halflife: $ty) -> Self {
                let ln2 = core::f64::consts::LN_2 as $ty;
                let alpha = 1.0 as $ty - crate::math::exp((-ln2 / halflife) as f64) as $ty;
                self.alpha = Option::Some(alpha);
                self
            }

            /// Number of samples for center of mass (pandas convention).
            ///
            /// Computes: `alpha = 2 / (n + 1)`
            #[inline]
            #[must_use]
            pub fn span(mut self, n: u64) -> Self {
                let alpha = 2.0 as $ty / (n as $ty + 1.0 as $ty);
                self.alpha = Option::Some(alpha);
                self
            }

            /// Minimum samples before value is considered valid. Default: 1.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the EMA.
            ///
            /// # Panics
            ///
            /// - Alpha must have been set (via `alpha`, `halflife`, or `span`).
            /// - Alpha must be in (0, 1) exclusive.
            #[inline]
            #[must_use]
            pub fn build(self) -> $name {
                let alpha = self.alpha.expect("EMA alpha must be set (use .alpha(), .halflife(), or .span())");
                assert!(alpha > 0.0 as $ty && alpha < 1.0 as $ty, "EMA alpha must be in (0, 1), got {alpha}");

                $name {
                    alpha,
                    one_minus_alpha: 1.0 as $ty - alpha,
                    value: 0.0 as $ty,
                    count: 0,
                    min_samples: self.min_samples,
                }
            }
        }
    };
}

impl_ema_float!(EmaF64, EmaF64Builder, f64);
impl_ema_float!(EmaF32, EmaF32Builder, f32);

// =============================================================================
// Integer EMA (kernel-style fixed-point)
// =============================================================================

/// Rounds `n` up to the next value of the form `2^k - 1`.
///
/// Examples: 1→1, 2→3, 3→3, 4→7, 5→7, 10→15, 20→31.
#[inline]
const fn next_power_of_two_minus_one(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    // next_power_of_two(n + 1) - 1
    // But we need to handle the case where n+1 is already a power of 2
    let v = n + 1;
    let p = v.next_power_of_two();
    p - 1
}

/// Returns log2 of (n + 1), where n must be of the form 2^k - 1.
#[inline]
const fn log2_of_span_plus_one(span: u64) -> u32 {
    // span = 2^k - 1, so span + 1 = 2^k
    (span + 1).trailing_zeros()
}

macro_rules! impl_ema_int {
    ($name:ident, $builder:ident, $ty:ty, $acc_ty:ty) => {
        /// EMA — Exponential Moving Average (integer fixed-point variant).
        ///
        /// Uses the Linux kernel pattern: bit-shift arithmetic with integer-only
        /// operations. No floating point. Weight is derived from span, rounded
        /// up to the next valid value (`2^k - 1`) for shift optimization.
        ///
        /// The weight reciprocal is `span + 1` (a power of 2), so division
        /// becomes a right-shift by `k` bits.
        ///
        /// # Use Cases
        /// - Smoothing nanosecond latency measurements without float
        /// - Kernel-style EWMA for counters and durations
        #[derive(Debug, Clone)]
        pub struct $name {
            /// Accumulator — stores value << shift for precision
            acc: $acc_ty,
            /// Bit shift amount: log2(span + 1)
            shift: u32,
            /// Effective span (2^k - 1)
            span: u64,
            count: u64,
            min_samples: u64,
            initialized: bool,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            span: Option<u64>,
            min_samples: u64,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    span: Option::None,
                    min_samples: 1,
                }
            }

            /// Feeds a sample. Returns smoothed value once primed.
            ///
            /// First sample initializes the accumulator directly.
            ///
            /// Update formula (kernel pattern):
            /// ```text
            /// acc += (sample << shift) - acc) >> shift
            /// ```
            /// This is equivalent to: `value = value + (sample - value) / (span + 1)`
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<$ty> {
                self.count += 1;

                if !self.initialized {
                    self.acc = (sample as $acc_ty) << self.shift;
                    self.initialized = true;
                } else {
                    let sample_shifted = (sample as $acc_ty) << self.shift;
                    self.acc += (sample_shifted - self.acc) >> self.shift;
                }

                if self.count >= self.min_samples {
                    Option::Some((self.acc >> self.shift) as $ty)
                } else {
                    Option::None
                }
            }

            /// Current smoothed value, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn value(&self) -> Option<$ty> {
                if self.count >= self.min_samples && self.initialized {
                    Option::Some((self.acc >> self.shift) as $ty)
                } else {
                    Option::None
                }
            }

            /// The actual span after rounding up to `2^k - 1`.
            #[inline]
            #[must_use]
            pub fn effective_span(&self) -> u64 {
                self.span
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether the EMA has reached `min_samples`.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count >= self.min_samples
            }

            /// Resets to uninitialized state. Parameters unchanged.
            #[inline]
            pub fn reset(&mut self) {
                self.acc = 0;
                self.count = 0;
                self.initialized = false;
            }
        }

        impl $builder {
            /// Minimum span. Rounded up to next `2^k - 1`.
            ///
            /// Call [`
            #[doc = stringify!($name)]
            /// ::effective_span()`] after build to see the actual value.
            #[inline]
            #[must_use]
            pub fn span(mut self, n: u64) -> Self {
                self.span = Option::Some(n);
                self
            }

            /// Minimum samples before value is valid. Default: 1.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the EMA.
            ///
            /// # Panics
            ///
            /// - Span must have been set.
            /// - Span must be >= 1.
            #[inline]
            #[must_use]
            pub fn build(self) -> $name {
                let requested = self.span.expect("EMA span must be set (use .span())");
                assert!(requested >= 1, "EMA span must be >= 1");

                let effective = next_power_of_two_minus_one(requested);
                let shift = log2_of_span_plus_one(effective);

                $name {
                    acc: 0,
                    shift,
                    span: effective,
                    count: 0,
                    min_samples: self.min_samples,
                    initialized: false,
                }
            }
        }
    };
}

impl_ema_int!(EmaI64, EmaI64Builder, i64, i128);
impl_ema_int!(EmaI32, EmaI32Builder, i32, i64);

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Float EMA
    // =========================================================================

    #[test]
    fn first_sample_initializes() {
        let mut ema = EmaF64::builder().alpha(0.5).build();
        assert_eq!(ema.update(100.0), Some(100.0));
        assert_eq!(ema.value(), Some(100.0));
    }

    #[test]
    fn convergence_toward_constant() {
        let mut ema = EmaF64::builder().alpha(0.1).build();

        // Initialize with 0
        let _ = ema.update(0.0);

        // Feed constant 100 — should converge
        for _ in 0..1000 {
            let _ = ema.update(100.0);
        }

        let val = ema.value().unwrap();
        assert!((val - 100.0).abs() < 0.01, "EMA should converge to 100, got {val}");
    }

    #[test]
    fn higher_alpha_reacts_faster() {
        let mut fast = EmaF64::builder().alpha(0.9).build();
        let mut slow = EmaF64::builder().alpha(0.1).build();

        let _ = fast.update(0.0);
        let _ = slow.update(0.0);

        let _ = fast.update(100.0);
        let _ = slow.update(100.0);

        let fast_val = fast.value().unwrap();
        let slow_val = slow.value().unwrap();

        assert!(fast_val > slow_val,
            "fast ({fast_val}) should react more than slow ({slow_val})");
    }

    #[test]
    fn priming_behavior() {
        let mut ema = EmaF64::builder()
            .alpha(0.5)
            .min_samples(5)
            .build();

        for i in 1..5 {
            assert_eq!(ema.update(100.0), None, "sample {i} should not be primed");
            assert!(!ema.is_primed());
        }

        assert!(ema.update(100.0).is_some());
        assert!(ema.is_primed());
    }

    #[test]
    fn reset_clears_state() {
        let mut ema = EmaF64::builder().alpha(0.5).build();
        let _ = ema.update(100.0);
        let _ = ema.update(200.0);

        ema.reset();
        assert_eq!(ema.count(), 0);
        assert_eq!(ema.value(), None);

        // Re-initialize should work
        assert_eq!(ema.update(50.0), Some(50.0));
    }

    #[test]
    fn span_computes_alpha() {
        let ema = EmaF64::builder().span(19).build();
        // alpha = 2 / (19 + 1) = 0.1
        assert!((ema.alpha() - 0.1).abs() < 1e-10);
    }

    #[test]
    fn halflife_computes_alpha() {
        let ema = EmaF64::builder().halflife(1.0).build();
        // halflife=1: alpha = 1 - exp(-ln2) = 1 - 0.5 = 0.5
        assert!((ema.alpha() - 0.5).abs() < 1e-10);
    }

    #[test]
    #[should_panic(expected = "alpha must be set")]
    fn panics_without_alpha() {
        let _ = EmaF64::builder().build();
    }

    #[test]
    #[should_panic(expected = "alpha must be in (0, 1)")]
    fn panics_on_alpha_zero() {
        let _ = EmaF64::builder().alpha(0.0).build();
    }

    #[test]
    #[should_panic(expected = "alpha must be in (0, 1)")]
    fn panics_on_alpha_one() {
        let _ = EmaF64::builder().alpha(1.0).build();
    }

    #[test]
    fn f32_basic() {
        let mut ema = EmaF32::builder().alpha(0.5).build();
        assert_eq!(ema.update(100.0), Some(100.0));
        let v = ema.update(200.0).unwrap();
        assert!((v - 150.0).abs() < 0.01);
    }

    // =========================================================================
    // Integer EMA
    // =========================================================================

    #[test]
    fn span_rounding() {
        // 1 → 1 (2^1 - 1)
        let ema = EmaI64::builder().span(1).build();
        assert_eq!(ema.effective_span(), 1);

        // 2 → 3 (2^2 - 1)
        let ema = EmaI64::builder().span(2).build();
        assert_eq!(ema.effective_span(), 3);

        // 3 → 3
        let ema = EmaI64::builder().span(3).build();
        assert_eq!(ema.effective_span(), 3);

        // 7 → 7 (2^3 - 1, exact)
        let ema = EmaI64::builder().span(7).build();
        assert_eq!(ema.effective_span(), 7);

        // 10 → 15 (2^4 - 1)
        let ema = EmaI64::builder().span(10).build();
        assert_eq!(ema.effective_span(), 15);

        // 20 → 31 (2^5 - 1)
        let ema = EmaI64::builder().span(20).build();
        assert_eq!(ema.effective_span(), 31);
    }

    #[test]
    fn int_first_sample_initializes() {
        let mut ema = EmaI64::builder().span(7).build();
        assert_eq!(ema.update(1000), Some(1000));
    }

    #[test]
    fn int_convergence() {
        let mut ema = EmaI64::builder().span(7).build();

        let _ = ema.update(0);
        for _ in 0..10_000 {
            let _ = ema.update(1000);
        }

        let val = ema.value().unwrap();
        // Should converge close to 1000 (integer precision may be off by 1)
        assert!((val - 1000).abs() <= 1, "should converge to ~1000, got {val}");
    }

    #[test]
    fn int_no_drift_over_many_samples() {
        let mut ema = EmaI64::builder().span(15).build();

        // Feed constant value — should not drift
        for _ in 0..100_000 {
            let _ = ema.update(500);
        }

        let val = ema.value().unwrap();
        assert_eq!(val, 500, "constant input should produce exact output, got {val}");
    }

    #[test]
    fn int_priming() {
        let mut ema = EmaI64::builder()
            .span(7)
            .min_samples(5)
            .build();

        for _ in 0..4 {
            assert_eq!(ema.update(100), None);
        }
        assert!(ema.update(100).is_some());
    }

    #[test]
    fn int_reset() {
        let mut ema = EmaI64::builder().span(7).build();
        let _ = ema.update(1000);
        let _ = ema.update(2000);

        ema.reset();
        assert_eq!(ema.count(), 0);
        assert_eq!(ema.value(), None);
    }

    #[test]
    fn i32_basic() {
        let mut ema = EmaI32::builder().span(3).build();
        assert_eq!(ema.update(100), Some(100));
    }

    #[test]
    #[should_panic(expected = "span must be set")]
    fn int_panics_without_span() {
        let _ = EmaI64::builder().build();
    }

    #[test]
    fn int_vs_float_comparison() {
        // Both should produce similar results on the same input
        let mut int_ema = EmaI64::builder().span(15).build();
        let mut float_ema = EmaF64::builder().span(15).build();

        let samples = [100, 110, 95, 105, 120, 90, 100, 115, 85, 100];

        for &s in &samples {
            let _ = int_ema.update(s);
            let _ = float_ema.update(s as f64);
        }

        let int_val = int_ema.value().unwrap();
        let float_val = float_ema.value().unwrap();

        // Should be within a few units of each other
        let diff = (int_val as f64 - float_val).abs();
        assert!(diff < 5.0,
            "int ({int_val}) and float ({float_val}) should be close, diff={diff}");
    }
}
