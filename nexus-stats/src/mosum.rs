use crate::Shift;

macro_rules! impl_mosum {
    ($name:ident, $builder:ident, $ty:ty, $zero:expr) => {
        /// MOSUM — Moving Sum change detector.
        ///
        /// Windowed complement to CUSUM. Detects transient shifts (spikes)
        /// rather than persistent shifts. Uses a ring buffer of the last N
        /// deviations from target and tests whether their sum exceeds a threshold.
        ///
        /// # Use Cases
        /// - Spike detection ("latency spiked for 10 seconds")
        /// - Transient anomaly detection (as opposed to CUSUM's persistent shifts)
        ///
        /// # Const Generic
        ///
        /// `N` is the window size. The ring buffer lives on the stack.
        #[derive(Debug, Clone)]
        pub struct $name<const N: usize> {
            target: $ty,
            threshold: $ty,
            ring: [$ty; N],
            head: usize,
            sum: $ty,
            count: u64,
            min_samples: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder<const N: usize> {
            target: $ty,
            threshold: Option<$ty>,
            min_samples: u64,
        }

        impl<const N: usize> $name<N> {
            /// Creates a builder with the target (expected baseline mean).
            #[inline]
            #[must_use]
            pub fn builder(target: $ty) -> $builder<N> {
                $builder {
                    target,
                    threshold: Option::None,
                    min_samples: N as u64,
                }
            }

            /// Feeds a sample. Returns shift direction once primed.
            ///
            /// The window must be full (`N` samples) before detection activates.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<Shift> {
                let deviation = sample - self.target;

                // Subtract the value being evicted, add the new one
                self.sum = self.sum - self.ring[self.head] + deviation;
                self.ring[self.head] = deviation;
                self.head = (self.head + 1) % N;

                self.count += 1;

                if self.count < self.min_samples {
                    return Option::None;
                }

                if self.sum > self.threshold {
                    Option::Some(Shift::Upper)
                } else if self.sum < -self.threshold {
                    Option::Some(Shift::Lower)
                } else {
                    Option::Some(Shift::None)
                }
            }

            /// Current moving sum of deviations.
            #[inline]
            #[must_use]
            pub fn sum(&self) -> $ty {
                self.sum
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether the window is full and detection is active.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count >= self.min_samples
            }

            /// Resets to empty state. Parameters unchanged.
            #[inline]
            pub fn reset(&mut self) {
                self.ring = [$zero; N];
                self.head = 0;
                self.sum = $zero;
                self.count = 0;
            }
        }

        impl<const N: usize> $builder<N> {
            /// Decision threshold. The sum of deviations must exceed this
            /// (positive for upper shift, negative for lower).
            #[inline]
            #[must_use]
            pub fn threshold(mut self, threshold: $ty) -> Self {
                self.threshold = Option::Some(threshold);
                self
            }

            /// Minimum samples before detection activates. Default: `N`.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the MOSUM detector.
            ///
            /// # Panics
            ///
            /// - Threshold must have been set.
            /// - Threshold must be positive.
            /// - Window size `N` must be > 0.
            #[inline]
            #[must_use]
            pub fn build(self) -> $name<N> {
                assert!(N > 0, "MOSUM window size must be > 0");
                let threshold = self.threshold.expect("MOSUM threshold must be set");
                assert!(threshold > $zero, "MOSUM threshold must be positive");

                $name {
                    target: self.target,
                    threshold,
                    ring: [$zero; N],
                    head: 0,
                    sum: $zero,
                    count: 0,
                    min_samples: self.min_samples,
                }
            }
        }
    };
}

impl_mosum!(MosumF64, MosumF64Builder, f64, 0.0);
impl_mosum!(MosumF32, MosumF32Builder, f32, 0.0);
impl_mosum!(MosumI64, MosumI64Builder, i64, 0);
impl_mosum!(MosumI32, MosumI32Builder, i32, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_detection_at_target() {
        let mut mosum = MosumF64::<10>::builder(100.0)
            .threshold(50.0)
            .build();

        // Prime the window
        for _ in 0..10 {
            let _ = mosum.update(100.0);
        }

        // After priming, should not detect
        for _ in 0..100 {
            assert_eq!(mosum.update(100.0), Some(Shift::None));
        }
    }

    #[test]
    fn detects_upward_spike() {
        let mut mosum = MosumF64::<10>::builder(100.0)
            .threshold(50.0)
            .build();

        // Fill window with normal values
        for _ in 0..10 {
            let _ = mosum.update(100.0);
        }

        // Spike — 10 samples at 110 each → sum = 10 * 10 = 100 > 50
        let mut triggered = false;
        for _ in 0..10 {
            if mosum.update(110.0) == Some(Shift::Upper) {
                triggered = true;
                break;
            }
        }
        assert!(triggered, "should detect upward spike");
    }

    #[test]
    fn detects_downward_spike() {
        let mut mosum = MosumF64::<10>::builder(100.0)
            .threshold(50.0)
            .build();

        for _ in 0..10 {
            let _ = mosum.update(100.0);
        }

        let mut triggered = false;
        for _ in 0..10 {
            if mosum.update(90.0) == Some(Shift::Lower) {
                triggered = true;
                break;
            }
        }
        assert!(triggered, "should detect downward spike");
    }

    #[test]
    fn transient_clears_after_window() {
        let mut mosum = MosumF64::<5>::builder(100.0)
            .threshold(40.0)
            .build();

        // Fill with normal
        for _ in 0..5 {
            let _ = mosum.update(100.0);
        }

        // Spike
        for _ in 0..5 {
            let _ = mosum.update(120.0);
        }
        // Sum should be high now
        assert!(mosum.sum() > 0.0);

        // Return to normal — after a full window, sum should clear
        for _ in 0..5 {
            let _ = mosum.update(100.0);
        }
        assert!(
            mosum.sum().abs() < 1e-10,
            "sum should return to ~0 after normal window, got {}",
            mosum.sum()
        );
    }

    #[test]
    fn priming_returns_none() {
        let mut mosum = MosumF64::<10>::builder(100.0)
            .threshold(50.0)
            .build();

        for _ in 0..9 {
            assert_eq!(mosum.update(200.0), None);
        }
        assert!(!mosum.is_primed());
        assert!(mosum.update(200.0).is_some());
        assert!(mosum.is_primed());
    }

    #[test]
    fn reset_clears_state() {
        let mut mosum = MosumF64::<10>::builder(100.0)
            .threshold(50.0)
            .build();

        for _ in 0..20 {
            let _ = mosum.update(120.0);
        }

        mosum.reset();
        assert_eq!(mosum.count(), 0);
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(mosum.sum(), 0.0);
        }
    }

    #[test]
    fn i64_basic() {
        let mut mosum = MosumI64::<5>::builder(1000)
            .threshold(100)
            .build();

        for _ in 0..5 {
            let _ = mosum.update(1000);
        }
        assert_eq!(mosum.update(1000), Some(Shift::None));
    }

    #[test]
    fn i32_basic() {
        let mut mosum = MosumI32::<5>::builder(100)
            .threshold(50)
            .build();

        for _ in 0..5 {
            let _ = mosum.update(100);
        }
        assert_eq!(mosum.update(100), Some(Shift::None));
    }

    #[test]
    #[should_panic(expected = "threshold must be set")]
    fn panics_without_threshold() {
        let _ = MosumF64::<10>::builder(100.0).build();
    }
}
