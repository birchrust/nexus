macro_rules! impl_peak_hold_float {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Peak hold with decay — instant attack, configurable hold, exponential decay.
        ///
        /// Captures peaks instantly, holds them for a configurable number of
        /// samples, then decays exponentially.
        ///
        /// # Use Cases
        /// - VU meter / level indicator behavior
        /// - Peak envelope tracking
        /// - "What was the recent peak?" with graceful decay
        #[derive(Debug, Clone)]
        pub struct $name {
            peak: $ty,
            hold_samples: u64,
            decay_rate: $ty,
            hold_remaining: u64,
            count: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            hold_samples: u64,
            decay_rate: Option<$ty>,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    hold_samples: 0,
                    decay_rate: Option::None,
                }
            }

            /// Feeds a sample. Returns the current envelope value.
            ///
            /// New peaks are captured instantly. During the hold period, the
            /// peak is maintained. After hold expires, the envelope decays
            /// multiplicatively each sample.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> $ty {
                self.count += 1;

                // Instant attack — new peak
                if sample >= self.peak {
                    self.peak = sample;
                    self.hold_remaining = self.hold_samples;
                    return self.peak;
                }

                // Hold period
                if self.hold_remaining > 0 {
                    self.hold_remaining -= 1;
                    return self.peak;
                }

                // Decay
                self.peak *= self.decay_rate;

                // If sample is above decayed peak, capture it
                if sample > self.peak {
                    self.peak = sample;
                    self.hold_remaining = self.hold_samples;
                }

                self.peak
            }

            /// Current envelope value.
            #[inline]
            #[must_use]
            pub fn peak(&self) -> $ty { self.peak }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 { self.count }

            /// Resets the envelope.
            #[inline]
            pub fn reset(&mut self) {
                self.peak = 0.0 as $ty;
                self.hold_remaining = 0;
                self.count = 0;
            }
        }

        impl $builder {
            /// Number of samples to hold the peak before decaying. Default: 0.
            #[inline]
            #[must_use]
            pub fn hold_samples(mut self, n: u64) -> Self {
                self.hold_samples = n;
                self
            }

            /// Per-sample multiplicative decay rate (0 to 1). Default must be set.
            ///
            /// 0.99 = slow decay, 0.9 = fast decay.
            #[inline]
            #[must_use]
            pub fn decay_rate(mut self, rate: $ty) -> Self {
                self.decay_rate = Option::Some(rate);
                self
            }

            /// Builds the peak hold envelope.
            ///
            /// # Errors
            ///
            /// - decay_rate must have been set.
            /// - decay_rate must be in (0, 1].
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let rate = self.decay_rate.ok_or(crate::ConfigError::Missing("decay_rate"))?;
                if !(rate > 0.0 as $ty && rate <= 1.0 as $ty) {
                    return Err(crate::ConfigError::Invalid("decay_rate must be in (0, 1]"));
                }

                Ok($name {
                    peak: 0.0 as $ty,
                    hold_samples: self.hold_samples,
                    decay_rate: rate,
                    hold_remaining: 0,
                    count: 0,
                })
            }
        }
    };
}

macro_rules! impl_peak_hold_int {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Peak hold (integer) — instant attack, configurable hold, no decay.
        ///
        /// Integer variant tracks the peak during the hold window. After hold
        /// expires, the peak resets to the current sample (no exponential decay
        /// for integers — use the float variant for decay behavior).
        #[derive(Debug, Clone)]
        pub struct $name {
            peak: $ty,
            hold_samples: u64,
            hold_remaining: u64,
            count: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            hold_samples: u64,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder { hold_samples: 0 }
            }

            /// Feeds a sample. Returns the current peak.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> $ty {
                self.count += 1;

                if sample >= self.peak || self.count == 1 {
                    self.peak = sample;
                    self.hold_remaining = self.hold_samples;
                    return self.peak;
                }

                if self.hold_remaining > 0 {
                    self.hold_remaining -= 1;
                    return self.peak;
                }

                // Hold expired — reset to current sample
                self.peak = sample;
                self.hold_remaining = self.hold_samples;
                self.peak
            }

            /// Current peak value.
            #[inline]
            #[must_use]
            pub fn peak(&self) -> $ty { self.peak }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 { self.count }

            /// Resets the peak.
            #[inline]
            pub fn reset(&mut self) {
                self.peak = 0;
                self.hold_remaining = 0;
                self.count = 0;
            }
        }

        impl $builder {
            /// Number of samples to hold the peak. Default: 0.
            #[inline]
            #[must_use]
            pub fn hold_samples(mut self, n: u64) -> Self {
                self.hold_samples = n;
                self
            }

            /// Builds the peak hold tracker.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                Ok($name {
                    peak: 0,
                    hold_samples: self.hold_samples,
                    hold_remaining: 0,
                    count: 0,
                })
            }
        }
    };
}

impl_peak_hold_float!(PeakHoldF64, PeakHoldF64Builder, f64);
impl_peak_hold_float!(PeakHoldF32, PeakHoldF32Builder, f32);
impl_peak_hold_int!(PeakHoldI64, PeakHoldI64Builder, i64);
impl_peak_hold_int!(PeakHoldI32, PeakHoldI32Builder, i32);
impl_peak_hold_int!(PeakHoldI128, PeakHoldI128Builder, i128);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)]
    fn instant_attack() {
        let mut ph = PeakHoldF64::builder().decay_rate(0.95).hold_samples(5).build().unwrap();
        assert_eq!(ph.update(50.0), 50.0);
        assert_eq!(ph.update(100.0), 100.0); // instant capture
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn hold_period() {
        let mut ph = PeakHoldF64::builder().decay_rate(0.95).hold_samples(3).build().unwrap();
        let _ = ph.update(100.0);
        assert_eq!(ph.update(50.0), 100.0); // held
        assert_eq!(ph.update(50.0), 100.0); // held
        assert_eq!(ph.update(50.0), 100.0); // held (3rd hold sample)
    }

    #[test]
    fn decay_after_hold() {
        let mut ph = PeakHoldF64::builder().decay_rate(0.9).hold_samples(0).build().unwrap();
        let _ = ph.update(100.0);
        let v = ph.update(0.0); // decay immediately (no hold)
        assert!(v < 100.0, "should have decayed, got {v}");
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn new_peak_during_hold() {
        let mut ph = PeakHoldF64::builder().decay_rate(0.95).hold_samples(10).build().unwrap();
        let _ = ph.update(100.0);
        let _ = ph.update(50.0); // holding at 100
        assert_eq!(ph.update(200.0), 200.0); // new peak resets hold
    }

    #[test]
    fn i64_hold() {
        let mut ph = PeakHoldI64::builder().hold_samples(3).build().unwrap();
        let _ = ph.update(100);
        assert_eq!(ph.update(50), 100); // held
        assert_eq!(ph.update(50), 100); // held
        assert_eq!(ph.update(50), 100); // held
        assert_eq!(ph.update(50), 50);  // hold expired, reset to current
    }

    #[test]
    fn reset() {
        let mut ph = PeakHoldF64::builder().decay_rate(0.95).build().unwrap();
        let _ = ph.update(100.0);
        ph.reset();
        assert_eq!(ph.count(), 0);
    }

    #[test]
    fn errors_without_decay_rate() {
        let result = PeakHoldF64::builder().build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("decay_rate"))));
    }

    #[test]
    fn i128_basic() {
        let mut ph = PeakHoldI128::builder().hold_samples(3).build().unwrap();
        let _ = ph.update(100);
        assert_eq!(ph.update(50), 100); // held
    }
}
