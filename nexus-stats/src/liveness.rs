use crate::math::MulAdd;
macro_rules! impl_liveness_float {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// Liveness detector — EMA of inter-arrival times with deadline threshold.
        ///
        /// Detects when a source goes quiet by tracking the smoothed interval
        /// between events and comparing against a deadline.
        ///
        /// # Use Cases
        /// - Stale quote detection
        /// - Heartbeat monitoring
        /// - Feed health checking
        #[derive(Debug, Clone)]
        pub struct $name {
            alpha: $ty,
            one_minus_alpha: $ty,
            interval: $ty,
            last_timestamp: $ty,
            deadline_multiple: Option<$ty>,
            deadline_absolute: Option<$ty>,
            count: u64,
            min_samples: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            alpha: Option<$ty>,
            deadline_multiple: Option<$ty>,
            deadline_absolute: Option<$ty>,
            min_samples: u64,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    alpha: Option::None,
                    deadline_multiple: Option::None,
                    deadline_absolute: Option::None,
                    min_samples: 2,
                }
            }

            /// Records an event at the given timestamp. Returns `true` if alive.
            ///
            /// The first event only records the timestamp. The second event
            /// computes the first interval. Returns `true` until primed, then
            /// checks against the deadline.
            #[inline]
            #[must_use]
            pub fn record(&mut self, timestamp: $ty) -> bool {
                self.count += 1;

                if self.count == 1 {
                    self.last_timestamp = timestamp;
                    return true;
                }

                let dt = timestamp - self.last_timestamp;
                self.last_timestamp = timestamp;

                if self.count == 2 {
                    self.interval = dt;
                } else {
                    self.interval = self.alpha.fma(dt, self.one_minus_alpha * self.interval);
                }

                if self.count < self.min_samples {
                    return true;
                }

                self.is_alive_at_interval(dt)
            }

            /// Checks liveness at the given timestamp without recording an event.
            ///
            /// Returns `true` if the time since the last event is within the deadline.
            /// Returns `true` if not yet primed.
            #[inline]
            #[must_use]
            pub fn check(&self, now: $ty) -> bool {
                if self.count < self.min_samples {
                    return true;
                }

                let dt = now - self.last_timestamp;
                self.is_alive_at_interval(dt)
            }

            #[inline]
            fn is_alive_at_interval(&self, dt: $ty) -> bool {
                if let Some(multiple) = self.deadline_multiple {
                    return dt <= self.interval * multiple;
                }
                if let Some(absolute) = self.deadline_absolute {
                    return dt <= absolute;
                }
                true
            }

            /// Current smoothed inter-arrival time, or `None` if < 2 events.
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

            /// Whether the detector has reached `min_samples`.
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

            /// Switches to a deadline-multiple threshold, clearing any absolute deadline.
            #[inline]
            pub fn reconfigure_deadline_multiple(&mut self, n: $ty) {
                self.deadline_multiple = Option::Some(n);
                self.deadline_absolute = Option::None;
            }

            /// Switches to an absolute deadline threshold, clearing any multiple deadline.
            #[inline]
            pub fn reconfigure_deadline_absolute(&mut self, t: $ty) {
                self.deadline_absolute = Option::Some(t);
                self.deadline_multiple = Option::None;
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

            /// Alert when interval exceeds `n * smoothed_interval`.
            ///
            /// Typical values: 2.0-5.0.
            #[inline]
            #[must_use]
            pub fn deadline_multiple(mut self, n: $ty) -> Self {
                self.deadline_multiple = Option::Some(n);
                self
            }

            /// Alert when interval exceeds a fixed deadline.
            #[inline]
            #[must_use]
            pub fn deadline_absolute(mut self, t: $ty) -> Self {
                self.deadline_absolute = Option::Some(t);
                self
            }

            /// Minimum events before liveness checking activates. Default: 2.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the liveness detector.
            ///
            /// # Errors
            ///
            /// - Alpha must have been set.
            /// - Alpha must be in (0, 1) exclusive.
            /// - At least one deadline (multiple or absolute) must be set.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let alpha = self.alpha.ok_or(crate::ConfigError::Missing("alpha"))?;
                if !(alpha > 0.0 as $ty && alpha < 1.0 as $ty) {
                    return Err(crate::ConfigError::Invalid("Liveness alpha must be in (0, 1)"));
                }
                if self.deadline_multiple.is_none() && self.deadline_absolute.is_none() {
                    return Err(crate::ConfigError::Invalid("Liveness requires a deadline (use .deadline_multiple() or .deadline_absolute())"));
                }

                Ok($name {
                    alpha,
                    one_minus_alpha: 1.0 as $ty - alpha,
                    interval: 0.0 as $ty,
                    last_timestamp: 0.0 as $ty,
                    deadline_multiple: self.deadline_multiple,
                    deadline_absolute: self.deadline_absolute,
                    count: 0,
                    min_samples: self.min_samples,
                })
            }
        }
    };
}

impl_liveness_float!(LivenessF64, LivenessF64Builder, f64);
impl_liveness_float!(LivenessF32, LivenessF32Builder, f32);

macro_rules! impl_liveness_int {
    ($name:ident, $builder:ident, $ty:ty, $acc_ty:ty) => {
        /// Liveness detector (integer variant) — fixed-point EMA of inter-arrival ticks.
        ///
        /// Uses kernel-style bit-shift arithmetic for the interval smoothing.
        /// Timestamps are integer ticks.
        #[derive(Debug, Clone)]
        pub struct $name {
            acc: $acc_ty,
            shift: u32,
            span: u64,
            last_timestamp: $ty,
            deadline_multiple: Option<u64>,
            deadline_absolute: Option<$ty>,
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
            deadline_multiple: Option<u64>,
            deadline_absolute: Option<$ty>,
            min_samples: u64,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    span: Option::None,
                    deadline_multiple: Option::None,
                    deadline_absolute: Option::None,
                    min_samples: 2,
                }
            }

            /// Records an event at the given tick. Returns `true` if alive.
            #[inline]
            #[must_use]
            pub fn record(&mut self, timestamp: $ty) -> bool {
                self.count += 1;

                if self.count == 1 {
                    self.last_timestamp = timestamp;
                    return true;
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

                if self.count < self.min_samples {
                    return true;
                }

                let smoothed = (self.acc >> self.shift) as $ty;
                self.is_alive_with(dt, smoothed)
            }

            /// Checks liveness at the given tick without recording.
            #[inline]
            #[must_use]
            pub fn check(&self, now: $ty) -> bool {
                if self.count < self.min_samples || !self.initialized {
                    return true;
                }

                let dt = now - self.last_timestamp;
                let smoothed = (self.acc >> self.shift) as $ty;
                self.is_alive_with(dt, smoothed)
            }

            #[inline]
            fn is_alive_with(&self, dt: $ty, smoothed: $ty) -> bool {
                if let Some(multiple) = self.deadline_multiple {
                    return dt <= smoothed * (multiple as $ty);
                }
                if let Some(absolute) = self.deadline_absolute {
                    return dt <= absolute;
                }
                true
            }

            /// Current smoothed inter-arrival interval, or `None` if < 2 events.
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

            /// Whether the detector has reached `min_samples`.
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

            /// Alert when interval exceeds `n * smoothed_interval`.
            #[inline]
            #[must_use]
            pub fn deadline_multiple(mut self, n: u64) -> Self {
                self.deadline_multiple = Option::Some(n);
                self
            }

            /// Alert when interval exceeds a fixed deadline (in ticks).
            #[inline]
            #[must_use]
            pub fn deadline_absolute(mut self, t: $ty) -> Self {
                self.deadline_absolute = Option::Some(t);
                self
            }

            /// Minimum events before liveness checking activates. Default: 2.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Builds the liveness detector.
            ///
            /// # Errors
            ///
            /// - Span must have been set and >= 1.
            /// - At least one deadline must be set.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let requested = self.span.ok_or(crate::ConfigError::Missing("span"))?;
                if requested < 1 {
                    return Err(crate::ConfigError::Invalid("Liveness span must be >= 1"));
                }
                if self.deadline_multiple.is_none() && self.deadline_absolute.is_none() {
                    return Err(crate::ConfigError::Invalid("Liveness requires a deadline"));
                }

                let effective = crate::ema::next_power_of_two_minus_one(requested);
                let shift = crate::ema::log2_of_span_plus_one(effective);

                Ok($name {
                    acc: 0,
                    shift,
                    span: effective,
                    last_timestamp: 0,
                    deadline_multiple: self.deadline_multiple,
                    deadline_absolute: self.deadline_absolute,
                    count: 0,
                    min_samples: self.min_samples,
                    initialized: false,
                })
            }
        }
    };
}

impl_liveness_int!(LivenessI64, LivenessI64Builder, i64, i128);
impl_liveness_int!(LivenessI32, LivenessI32Builder, i32, i64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alive_while_events_arrive() {
        let mut lv = LivenessF64::builder()
            .alpha(0.3)
            .deadline_multiple(3.0)
            .build()
            .unwrap();

        // Regular events every 10 units
        for i in 0..20 {
            assert!(lv.record(i as f64 * 10.0), "should be alive at event {i}");
        }
    }

    #[test]
    fn dead_after_silence() {
        let mut lv = LivenessF64::builder()
            .alpha(0.3)
            .deadline_multiple(3.0)
            .build()
            .unwrap();

        // Regular events every 10 units
        for i in 0..10 {
            let _ = lv.record(i as f64 * 10.0);
        }

        // Check after long silence — should be dead
        // Smoothed interval ≈ 10, deadline = 3 * 10 = 30, silence = 100
        assert!(!lv.check(190.0), "should be dead after long silence");
    }

    #[test]
    fn recovery_after_resume() {
        let mut lv = LivenessF64::builder()
            .alpha(0.3)
            .deadline_multiple(3.0)
            .build()
            .unwrap();

        for i in 0..10 {
            let _ = lv.record(i as f64 * 10.0);
        }

        // Dead check
        assert!(!lv.check(200.0));

        // Resume events — should recover
        assert!(lv.record(200.0)); // records, interval updates
        assert!(lv.record(210.0));
    }

    #[test]
    fn absolute_deadline() {
        let mut lv = LivenessF64::builder()
            .alpha(0.3)
            .deadline_absolute(50.0)
            .build()
            .unwrap();

        let _ = lv.record(0.0);
        let _ = lv.record(10.0);

        // Within deadline
        assert!(lv.check(55.0));
        // Exceeds deadline
        assert!(!lv.check(65.0));
    }

    #[test]
    fn not_primed_always_alive() {
        let mut lv = LivenessF64::builder()
            .alpha(0.3)
            .deadline_multiple(3.0)
            .min_samples(5)
            .build()
            .unwrap();

        // Even with huge gaps, returns true before primed
        assert!(lv.record(0.0));
        assert!(lv.record(1000.0));
        assert!(!lv.is_primed());
    }

    #[test]
    fn i64_basic() {
        let mut lv = LivenessI64::builder()
            .span(7)
            .deadline_multiple(3)
            .build()
            .unwrap();

        for i in 0..10 {
            assert!(lv.record(i * 100));
        }

        // Long silence
        assert!(!lv.check(2000));
    }

    #[test]
    fn i32_basic() {
        let mut lv = LivenessI32::builder()
            .span(3)
            .deadline_absolute(500)
            .build()
            .unwrap();

        let _ = lv.record(0);
        let _ = lv.record(100);
        assert!(lv.check(400));
        assert!(!lv.check(700));
    }

    #[test]
    fn reset_clears_state() {
        let mut lv = LivenessF64::builder()
            .alpha(0.3)
            .deadline_multiple(3.0)
            .build()
            .unwrap();

        for i in 0..10 {
            let _ = lv.record(i as f64 * 10.0);
        }

        lv.reset();
        assert_eq!(lv.count(), 0);
        assert!(lv.interval().is_none());
    }

    #[test]
    fn reconfigure_deadline_multiple() {
        let mut lv = LivenessF64::builder()
            .alpha(0.3)
            .deadline_absolute(50.0)
            .build()
            .unwrap();

        let _ = lv.record(0.0);
        let _ = lv.record(10.0);

        // With absolute 50, check at 55 is alive
        assert!(lv.check(55.0));

        // Switch to multiple=2 — smoothed interval ~10, deadline=20
        lv.reconfigure_deadline_multiple(2.0);
        // 55 - 10 = 45 > 20, should be dead
        assert!(!lv.check(55.0));
    }

    #[test]
    fn reconfigure_deadline_absolute() {
        let mut lv = LivenessF64::builder()
            .alpha(0.3)
            .deadline_multiple(3.0)
            .build()
            .unwrap();

        for i in 0..10 {
            let _ = lv.record(i as f64 * 10.0);
        }

        // Switch to absolute deadline
        lv.reconfigure_deadline_absolute(5.0);
        // Last event at 90, check at 100 => dt=10 > 5
        assert!(!lv.check(100.0));
    }

    #[test]
    fn errors_without_alpha() {
        let result = LivenessF64::builder().deadline_multiple(3.0).build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("alpha"))));
    }

    #[test]
    fn errors_without_deadline() {
        let result = LivenessF64::builder().alpha(0.3).build();
        assert!(matches!(result, Err(crate::ConfigError::Invalid(_))));
    }
}

// =============================================================================
// LivenessInstant — Instant-based liveness detector
// =============================================================================

#[cfg(feature = "std")]
mod instant_liveness {
    use std::time::{Duration, Instant};

    /// Liveness detector using `Instant` timestamps.
    ///
    /// EMA of inter-arrival times with deadline threshold. Timestamps
    /// are `Instant`, deadlines are `Duration` or multiples.
    #[derive(Debug, Clone)]
    pub struct LivenessInstant {
        alpha: f64,
        one_minus_alpha: f64,
        interval: f64, // seconds
        last_timestamp: Option<Instant>,
        deadline_multiple: Option<f64>,
        deadline_absolute: Option<Duration>,
        count: u64,
        min_samples: u64,
    }

    /// Builder for [`LivenessInstant`].
    #[derive(Debug, Clone)]
    pub struct LivenessInstantBuilder {
        alpha: Option<f64>,
        deadline_multiple: Option<f64>,
        deadline_absolute: Option<Duration>,
        min_samples: u64,
    }

    impl LivenessInstant {
        /// Creates a builder.
        #[inline]
        #[must_use]
        pub fn builder() -> LivenessInstantBuilder {
            LivenessInstantBuilder {
                alpha: None,
                deadline_multiple: None,
                deadline_absolute: None,
                min_samples: 2,
            }
        }

        /// Records an event. Returns `true` if alive.
        #[inline]
        #[must_use]
        pub fn record(&mut self, now: Instant) -> bool {
            self.count += 1;

            let dt = if let Some(last) = self.last_timestamp {
                now.saturating_duration_since(last).as_secs_f64()
            } else {
                self.last_timestamp = Some(now);
                return true;
            };
            self.last_timestamp = Some(now);

            if self.count == 2 {
                self.interval = dt;
            } else {
                self.interval = self.alpha.mul_add(dt, self.one_minus_alpha * self.interval);
            }

            if self.count < self.min_samples {
                return true;
            }

            self.is_alive_at_interval(dt)
        }

        /// Checks liveness without recording. Returns `true` if alive.
        #[inline]
        #[must_use]
        pub fn check(&self, now: Instant) -> bool {
            if self.count < self.min_samples {
                return true;
            }
            let dt = match self.last_timestamp {
                Some(last) => now.saturating_duration_since(last).as_secs_f64(),
                None => return true,
            };
            self.is_alive_at_interval(dt)
        }

        #[inline]
        fn is_alive_at_interval(&self, dt: f64) -> bool {
            if let Some(multiple) = self.deadline_multiple {
                return dt <= self.interval * multiple;
            }
            if let Some(absolute) = self.deadline_absolute {
                return dt <= absolute.as_secs_f64();
            }
            true
        }

        /// Current smoothed inter-arrival time as Duration.
        #[inline]
        #[must_use]
        pub fn interval(&self) -> Option<Duration> {
            if self.count >= 2 && self.interval > 0.0 {
                Some(Duration::from_secs_f64(self.interval))
            } else {
                None
            }
        }

        /// Number of events recorded.
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

        /// Resets. `now` becomes the new time reference.
        #[inline]
        pub fn reset(&mut self, now: Instant) {
            self.interval = 0.0;
            self.last_timestamp = Some(now);
            self.count = 0;
        }

        /// Switch to deadline-multiple threshold.
        #[inline]
        pub fn reconfigure_deadline_multiple(&mut self, n: f64) {
            self.deadline_multiple = Some(n);
            self.deadline_absolute = None;
        }

        /// Switch to absolute deadline threshold.
        #[inline]
        pub fn reconfigure_deadline_absolute(&mut self, d: Duration) {
            self.deadline_absolute = Some(d);
            self.deadline_multiple = None;
        }
    }

    impl LivenessInstantBuilder {
        /// Direct smoothing factor.
        #[inline]
        #[must_use]
        pub fn alpha(mut self, alpha: f64) -> Self {
            self.alpha = Some(alpha);
            self
        }

        /// Span for smoothing: `alpha = 2 / (n + 1)`.
        #[inline]
        #[must_use]
        pub fn span(mut self, n: u64) -> Self {
            self.alpha = Some(2.0 / (n as f64 + 1.0));
            self
        }

        /// Minimum events before checking. Default: 2.
        #[inline]
        #[must_use]
        pub fn min_samples(mut self, min: u64) -> Self {
            self.min_samples = min;
            self
        }

        /// Deadline as multiple of smoothed interval.
        #[inline]
        #[must_use]
        pub fn deadline_multiple(mut self, n: f64) -> Self {
            self.deadline_multiple = Some(n);
            self
        }

        /// Deadline as absolute Duration.
        #[inline]
        #[must_use]
        pub fn deadline_absolute(mut self, d: Duration) -> Self {
            self.deadline_absolute = Some(d);
            self
        }

        /// Builds the liveness detector.
        ///
        /// # Errors
        ///
        /// Alpha must be in (0, 1). At least one deadline required.
        #[inline]
        pub fn build(self) -> Result<LivenessInstant, crate::ConfigError> {
            let alpha = self.alpha.ok_or(crate::ConfigError::Missing("alpha"))?;
            if !(alpha > 0.0 && alpha < 1.0) {
                return Err(crate::ConfigError::Invalid(
                    "LivenessInstant alpha must be in (0, 1)",
                ));
            }
            if self.deadline_multiple.is_none() && self.deadline_absolute.is_none() {
                return Err(crate::ConfigError::Invalid(
                    "LivenessInstant requires at least one deadline (multiple or absolute)",
                ));
            }

            Ok(LivenessInstant {
                alpha,
                one_minus_alpha: 1.0 - alpha,
                interval: 0.0,
                last_timestamp: None,
                deadline_multiple: self.deadline_multiple,
                deadline_absolute: self.deadline_absolute,
                count: 0,
                min_samples: self.min_samples,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn alive_within_deadline() {
            let base = Instant::now();
            let mut lv = LivenessInstant::builder()
                .span(10)
                .deadline_multiple(3.0)
                .build()
                .unwrap();
            assert!(lv.record(base));
            assert!(lv.record(base + Duration::from_secs(1)));
            // 2 seconds later — 2x interval, within 3x deadline
            assert!(lv.check(base + Duration::from_secs(3)));
        }

        #[test]
        fn dead_beyond_deadline() {
            let base = Instant::now();
            let mut lv = LivenessInstant::builder()
                .span(10)
                .deadline_multiple(3.0)
                .build()
                .unwrap();
            assert!(lv.record(base));
            assert!(lv.record(base + Duration::from_secs(1)));
            // 10 seconds later — 10x interval, exceeds 3x deadline
            assert!(!lv.check(base + Duration::from_secs(11)));
        }

        #[test]
        fn absolute_deadline() {
            let base = Instant::now();
            let mut lv = LivenessInstant::builder()
                .span(10)
                .deadline_absolute(Duration::from_secs(5))
                .build()
                .unwrap();
            assert!(lv.record(base));
            assert!(lv.record(base + Duration::from_secs(1)));
            assert!(lv.check(base + Duration::from_secs(5))); // within 5s
            assert!(!lv.check(base + Duration::from_secs(7))); // beyond 5s
        }

        #[test]
        fn reset_clears() {
            let base = Instant::now();
            let mut lv = LivenessInstant::builder()
                .span(10)
                .deadline_multiple(3.0)
                .build()
                .unwrap();
            lv.record(base);
            lv.record(base + Duration::from_secs(1));
            lv.reset(base + Duration::from_secs(2));
            assert_eq!(lv.count(), 0);
        }
    }
}

#[cfg(feature = "std")]
pub use instant_liveness::{LivenessInstant, LivenessInstantBuilder};
