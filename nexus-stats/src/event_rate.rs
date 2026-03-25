use crate::math::MulAdd;
macro_rules! impl_event_rate_float {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Smoothed event rate tracker.
        ///
        /// Uses an EMA of inter-arrival times, inverted on query to produce
        /// a rate (events per unit time). The rate adapts smoothly to changes
        /// in event frequency.
        ///
        /// # Use Cases
        /// - Message throughput monitoring
        /// - Order rate tracking
        /// - Adaptive rate limiting input
        #[derive(Debug, Clone)]
        pub struct $name {
            alpha: $ty,
            one_minus_alpha: $ty,
            interval: $ty,
            last_timestamp: $ty,
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
                    min_samples: 2,
                }
            }

            /// Records an event at the given timestamp.
            ///
            /// If two events share a timestamp, the interval is zero and
            /// `rate()` returns `None` until a non-zero interval is observed.
            #[inline]
            pub fn tick(&mut self, timestamp: $ty) {
                self.count += 1;

                if self.count == 1 {
                    self.last_timestamp = timestamp;
                    return;
                }

                let dt = timestamp - self.last_timestamp;
                self.last_timestamp = timestamp;

                if self.count == 2 {
                    self.interval = dt;
                } else {
                    self.interval = self.alpha.fma(dt, self.one_minus_alpha * self.interval);
                }
            }

            /// Current smoothed event rate (events per unit time).
            ///
            /// Returns `None` if not primed or if interval is zero.
            #[inline]
            #[must_use]
            pub fn rate(&self) -> Option<$ty> {
                if self.count < self.min_samples || self.interval <= (0.0 as $ty) {
                    Option::None
                } else {
                    Option::Some(1.0 as $ty / self.interval)
                }
            }

            /// Current smoothed inter-event interval, or `None` if < 2 events.
            #[inline]
            #[must_use]
            pub fn interval(&self) -> Option<$ty> {
                if self.count >= 2 {
                    Option::Some(self.interval)
                } else {
                    Option::None
                }
            }

            /// Number of events recorded.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether the tracker has reached `min_samples`.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count >= self.min_samples
            }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.interval = 0.0 as $ty;
                self.last_timestamp = 0.0 as $ty;
                self.count = 0;
            }
        }

        impl $builder {
            /// Direct smoothing factor for interval EMA.
            #[inline]
            #[must_use]
            pub fn alpha(mut self, alpha: $ty) -> Self {
                self.alpha = Option::Some(alpha);
                self
            }

            /// Halflife for interval smoothing.
            #[inline]
            #[must_use]
            #[cfg(any(feature = "std", feature = "libm"))]
            pub fn halflife(mut self, halflife: $ty) -> Self {
                let ln2 = core::f64::consts::LN_2 as $ty;
                let alpha = 1.0 as $ty - crate::math::exp((-ln2 / halflife) as f64) as $ty;
                self.alpha = Option::Some(alpha);
                self
            }

            /// Span for interval smoothing.
            #[inline]
            #[must_use]
            pub fn span(mut self, n: u64) -> Self {
                let alpha = 2.0 as $ty / (n as $ty + 1.0 as $ty);
                self.alpha = Option::Some(alpha);
                self
            }

            /// Minimum events before rate is valid. Default: 2.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the event rate tracker.
            ///
            /// # Errors
            ///
            /// - Alpha must have been set.
            /// - Alpha must be in (0, 1) exclusive.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let alpha = self.alpha.ok_or(crate::ConfigError::Missing("alpha"))?;
                if !(alpha > 0.0 as $ty && alpha < 1.0 as $ty) {
                    return Err(crate::ConfigError::Invalid(
                        "EventRate alpha must be in (0, 1)",
                    ));
                }

                Ok($name {
                    alpha,
                    one_minus_alpha: 1.0 as $ty - alpha,
                    interval: 0.0 as $ty,
                    last_timestamp: 0.0 as $ty,
                    count: 0,
                    min_samples: self.min_samples,
                })
            }
        }
    };
}

impl_event_rate_float!(EventRateF64, EventRateF64Builder, f64);
impl_event_rate_float!(EventRateF32, EventRateF32Builder, f32);

macro_rules! impl_event_rate_int {
    ($name:ident, $builder:ident, $ty:ty, $acc_ty:ty) => {
        /// Smoothed event rate tracker (integer variant).
        ///
        /// Uses kernel-style fixed-point EMA on inter-arrival ticks.
        /// Rate query returns `unit / smoothed_interval` where unit is
        /// the caller's time unit.
        #[derive(Debug, Clone)]
        pub struct $name {
            acc: $acc_ty,
            shift: u32,
            span: u64,
            last_timestamp: $ty,
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
                    min_samples: 2,
                }
            }

            /// Records an event at the given tick.
            #[inline]
            pub fn tick(&mut self, timestamp: $ty) {
                self.count += 1;

                if self.count == 1 {
                    self.last_timestamp = timestamp;
                    return;
                }

                let dt = timestamp - self.last_timestamp;
                self.last_timestamp = timestamp;

                if !self.initialized {
                    self.acc = (dt as $acc_ty) << self.shift;
                    self.initialized = true;
                } else {
                    let dt_shifted = (dt as $acc_ty) << self.shift;
                    self.acc += (dt_shifted - self.acc) >> self.shift;
                }
            }

            /// Current smoothed inter-event interval, or `None` if < 2 events.
            #[inline]
            #[must_use]
            pub fn interval(&self) -> Option<$ty> {
                if self.count >= 2 && self.initialized {
                    Option::Some((self.acc >> self.shift) as $ty)
                } else {
                    Option::None
                }
            }

            /// Effective span after rounding.
            #[inline]
            #[must_use]
            pub fn effective_span(&self) -> u64 {
                self.span
            }

            /// Number of events recorded.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether the tracker has reached `min_samples`.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count >= self.min_samples
            }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.acc = 0;
                self.last_timestamp = 0;
                self.count = 0;
                self.initialized = false;
            }
        }

        impl $builder {
            /// Smoothing span. Rounded up to next `2^k - 1`.
            #[inline]
            #[must_use]
            pub fn span(mut self, n: u64) -> Self {
                self.span = Option::Some(n);
                self
            }

            /// Minimum events before rate is valid. Default: 2.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the event rate tracker.
            ///
            /// # Errors
            ///
            /// - Span must have been set and >= 1.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let requested = self.span.ok_or(crate::ConfigError::Missing("span"))?;
                if requested < 1 {
                    return Err(crate::ConfigError::Invalid("EventRate span must be >= 1"));
                }

                let effective = crate::ema::next_power_of_two_minus_one(requested);
                let shift = crate::ema::log2_of_span_plus_one(effective);

                Ok($name {
                    acc: 0,
                    shift,
                    span: effective,
                    last_timestamp: 0,
                    count: 0,
                    min_samples: self.min_samples,
                    initialized: false,
                })
            }
        }
    };
}

impl_event_rate_int!(EventRateI64, EventRateI64Builder, i64, i128);
impl_event_rate_int!(EventRateI32, EventRateI32Builder, i32, i64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_rate() {
        let mut er = EventRateF64::builder().alpha(0.3).build().unwrap();

        // Events every 10 units → rate should converge to 0.1
        for i in 0..100 {
            er.tick(i as f64 * 10.0);
        }

        let rate = er.rate().unwrap();
        assert!((rate - 0.1).abs() < 0.01, "rate should be ~0.1, got {rate}");
    }

    #[test]
    fn burst_increases_rate() {
        let mut er = EventRateF64::builder().alpha(0.5).build().unwrap();

        // Normal rate: every 10 units
        for i in 0..20 {
            er.tick(i as f64 * 10.0);
        }
        let normal_rate = er.rate().unwrap();

        // Burst: events every 1 unit
        for i in 0..20 {
            er.tick(200.0 + i as f64);
        }
        let burst_rate = er.rate().unwrap();

        assert!(
            burst_rate > normal_rate,
            "burst rate ({burst_rate}) should exceed normal ({normal_rate})"
        );
    }

    #[test]
    fn priming() {
        let mut er = EventRateF64::builder()
            .alpha(0.3)
            .min_samples(5)
            .build()
            .unwrap();

        for i in 0..4 {
            er.tick(i as f64 * 10.0);
            assert!(er.rate().is_none());
        }
        er.tick(40.0);
        assert!(er.rate().is_some());
    }

    #[test]
    fn reset() {
        let mut er = EventRateF64::builder().alpha(0.3).build().unwrap();
        for i in 0..10 {
            er.tick(i as f64 * 10.0);
        }
        er.reset();
        assert_eq!(er.count(), 0);
        assert!(er.rate().is_none());
    }

    #[test]
    fn f32_basic() {
        let mut er = EventRateF32::builder().alpha(0.3).build().unwrap();
        er.tick(0.0);
        er.tick(10.0);
        assert!(er.rate().is_some());
    }

    #[test]
    fn i64_basic() {
        let mut er = EventRateI64::builder().span(7).build().unwrap();
        for i in 0..10 {
            er.tick(i * 100);
        }
        let interval = er.interval().unwrap();
        assert!(
            (interval - 100).abs() <= 1,
            "interval should be ~100, got {interval}"
        );
    }

    #[test]
    fn i32_basic() {
        let mut er = EventRateI32::builder().span(3).build().unwrap();
        er.tick(0);
        er.tick(50);
        assert!(er.interval().is_some());
    }

    #[test]
    fn zero_interval_returns_none() {
        let mut er = EventRateF64::builder().alpha(0.3).build().unwrap();
        er.tick(100.0);
        er.tick(100.0); // same timestamp → interval = 0
        // rate() should return None (division by zero guard)
        assert!(
            er.rate().is_none(),
            "rate should be None with zero interval"
        );
    }

    #[test]
    fn errors_without_alpha() {
        let result = EventRateF64::builder().build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("alpha"))));
    }
}
