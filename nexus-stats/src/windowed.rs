// Windowed extrema using Nichols' 3-sample sub-window promotion algorithm.
//
// Ported from the Linux kernel's `win_minmax.h` (used by TCP BBR).
// Maintains the window extremum using only 3 stored samples, each covering
// a sub-window of `window/3` ticks. When a sub-window expires, the next
// candidate is promoted.
//
// State: 3 × (timestamp, value) + base Instant (std) or raw u64 window.

/// Internal sample stored per sub-window.
#[derive(Debug, Clone, Copy)]
struct Sample<T: Copy> {
    timestamp: u64, // nanos since base (std) or raw u64 (raw)
    value: T,
}

// =========================================================================
// Raw (u64 timestamp) variants — no_std compatible
// =========================================================================

macro_rules! impl_windowed_max_raw {
    ($name:ident, $ty:ty, $init:expr) => {
        /// Windowed maximum tracker using raw `u64` timestamps (Nichols' algorithm).
        ///
        /// Identical algorithm to the `Instant`-based variant but operates on
        /// caller-supplied `u64` timestamps directly. No `std` dependency.
        ///
        /// # Use Cases
        /// - Embedded / `no_std` environments
        /// - Pre-converted nanosecond timestamps
        /// - Deterministic replay with raw tick counters
        #[derive(Debug, Clone)]
        pub struct $name {
            window: u64,
            samples: [Sample<$ty>; 3],
            count: u64,
        }

        impl $name {
            /// Creates a new windowed max tracker.
            ///
            /// `window` is in the same units as the timestamps you will pass
            /// to [`update`](Self::update). Must be positive.
            #[inline]
            pub fn new(window: u64) -> Result<Self, crate::ConfigError> {
                if window == 0 {
                    return Err(crate::ConfigError::Invalid("window must be positive"));
                }
                let init = Sample {
                    timestamp: 0,
                    value: $init,
                };
                Ok(Self {
                    window,
                    samples: [init; 3],
                    count: 0,
                })
            }

            /// Feeds a sample at the given timestamp. Returns current window max.
            #[inline]
            #[must_use]
            pub fn update(&mut self, timestamp: u64, value: $ty) -> $ty {
                self.count += 1;
                let win = self.window;
                let s = &mut self.samples;

                if value >= s[0].value || timestamp.wrapping_sub(s[2].timestamp) > win {
                    s[0] = Sample { timestamp, value };
                    s[1] = s[0];
                    s[2] = s[0];
                    return s[0].value;
                }

                if timestamp.wrapping_sub(s[1].timestamp) > win / 3 {
                    s[1] = Sample { timestamp, value };
                    s[2] = s[1];
                } else if timestamp.wrapping_sub(s[2].timestamp) > win / 3 {
                    s[2] = Sample { timestamp, value };
                }

                if value >= s[1].value {
                    s[1] = Sample { timestamp, value };
                    s[2] = s[1];
                } else if value >= s[2].value {
                    s[2] = Sample { timestamp, value };
                }

                if timestamp.wrapping_sub(s[0].timestamp) > win {
                    s[0] = s[1];
                    s[1] = s[2];
                    s[2] = Sample { timestamp, value };
                } else if timestamp.wrapping_sub(s[1].timestamp) > win / 3 {
                    s[1] = s[2];
                    s[2] = Sample { timestamp, value };
                }

                s[0].value
            }

            /// Convenience wrapper that casts an `i64` timestamp to `u64`.
            #[inline]
            #[must_use]
            pub fn update_i64(&mut self, timestamp: i64, value: $ty) -> $ty {
                self.update(timestamp as u64, value)
            }

            /// Current window maximum, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn max(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.samples[0].value)
                }
            }

            /// Window size in raw units.
            #[inline]
            #[must_use]
            pub fn window(&self) -> u64 {
                self.window
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Resets to empty state. Window size is preserved.
            #[inline]
            pub fn reset(&mut self) {
                let init = Sample {
                    timestamp: 0,
                    value: $init,
                };
                self.samples = [init; 3];
                self.count = 0;
            }
        }
    };
}

macro_rules! impl_windowed_min_raw {
    ($name:ident, $ty:ty, $init:expr) => {
        /// Windowed minimum tracker using raw `u64` timestamps (Nichols' algorithm).
        ///
        /// Identical algorithm to the `Instant`-based variant but operates on
        /// caller-supplied `u64` timestamps directly. No `std` dependency.
        ///
        /// # Use Cases
        /// - Embedded / `no_std` environments
        /// - Pre-converted nanosecond timestamps
        /// - Deterministic replay with raw tick counters
        #[derive(Debug, Clone)]
        pub struct $name {
            window: u64,
            samples: [Sample<$ty>; 3],
            count: u64,
        }

        impl $name {
            /// Creates a new windowed min tracker.
            ///
            /// `window` is in the same units as the timestamps you will pass
            /// to [`update`](Self::update). Must be positive.
            #[inline]
            pub fn new(window: u64) -> Result<Self, crate::ConfigError> {
                if window == 0 {
                    return Err(crate::ConfigError::Invalid("window must be positive"));
                }
                let init = Sample {
                    timestamp: 0,
                    value: $init,
                };
                Ok(Self {
                    window,
                    samples: [init; 3],
                    count: 0,
                })
            }

            /// Feeds a sample at the given timestamp. Returns current window min.
            #[inline]
            #[must_use]
            pub fn update(&mut self, timestamp: u64, value: $ty) -> $ty {
                self.count += 1;
                let win = self.window;
                let s = &mut self.samples;

                if value <= s[0].value || timestamp.wrapping_sub(s[2].timestamp) > win {
                    s[0] = Sample { timestamp, value };
                    s[1] = s[0];
                    s[2] = s[0];
                    return s[0].value;
                }

                if timestamp.wrapping_sub(s[1].timestamp) > win / 3 {
                    s[1] = Sample { timestamp, value };
                    s[2] = s[1];
                } else if timestamp.wrapping_sub(s[2].timestamp) > win / 3 {
                    s[2] = Sample { timestamp, value };
                }

                if value <= s[1].value {
                    s[1] = Sample { timestamp, value };
                    s[2] = s[1];
                } else if value <= s[2].value {
                    s[2] = Sample { timestamp, value };
                }

                if timestamp.wrapping_sub(s[0].timestamp) > win {
                    s[0] = s[1];
                    s[1] = s[2];
                    s[2] = Sample { timestamp, value };
                } else if timestamp.wrapping_sub(s[1].timestamp) > win / 3 {
                    s[1] = s[2];
                    s[2] = Sample { timestamp, value };
                }

                s[0].value
            }

            /// Convenience wrapper that casts an `i64` timestamp to `u64`.
            #[inline]
            #[must_use]
            pub fn update_i64(&mut self, timestamp: i64, value: $ty) -> $ty {
                self.update(timestamp as u64, value)
            }

            /// Current window minimum, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn min(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.samples[0].value)
                }
            }

            /// Window size in raw units.
            #[inline]
            #[must_use]
            pub fn window(&self) -> u64 {
                self.window
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Resets to empty state. Window size is preserved.
            #[inline]
            pub fn reset(&mut self) {
                let init = Sample {
                    timestamp: 0,
                    value: $init,
                };
                self.samples = [init; 3];
                self.count = 0;
            }
        }
    };
}

impl_windowed_max_raw!(WindowedMaxF64Raw, f64, f64::MIN);
impl_windowed_max_raw!(WindowedMaxF32Raw, f32, f32::MIN);
impl_windowed_max_raw!(WindowedMaxI64Raw, i64, i64::MIN);
impl_windowed_max_raw!(WindowedMaxI32Raw, i32, i32::MIN);
impl_windowed_max_raw!(WindowedMaxI128Raw, i128, i128::MIN);

impl_windowed_min_raw!(WindowedMinF64Raw, f64, f64::MAX);
impl_windowed_min_raw!(WindowedMinF32Raw, f32, f32::MAX);
impl_windowed_min_raw!(WindowedMinI64Raw, i64, i64::MAX);
impl_windowed_min_raw!(WindowedMinI32Raw, i32, i32::MAX);
impl_windowed_min_raw!(WindowedMinI128Raw, i128, i128::MAX);

// =========================================================================
// Instant-based variants — requires std
// =========================================================================

#[cfg(feature = "std")]
use std::time::{Duration, Instant};

#[cfg(feature = "std")]
macro_rules! impl_windowed_max {
    ($name:ident, $ty:ty, $init:expr) => {
        /// Windowed maximum tracker (Nichols' algorithm).
        ///
        /// Efficiently tracks the maximum value within a sliding time window
        /// using only 3 stored samples. O(1) amortized per update.
        ///
        /// # Use Cases
        /// - Max throughput tracking
        /// - BBR-style bandwidth estimation
        /// - Peak detection within a time window
        #[derive(Debug, Clone)]
        pub struct $name {
            window: u64,
            samples: [Sample<$ty>; 3],
            count: u64,
            base: Instant,
        }

        impl $name {
            /// Creates a new windowed max tracker with `Instant::now()` as base.
            #[inline]
            pub fn new(window: Duration) -> Result<Self, crate::ConfigError> {
                Self::with_base(window, Instant::now())
            }

            /// Creates a new windowed max tracker with an explicit base instant.
            ///
            /// All timestamps passed to `update()` are measured relative to
            /// this base. Use this for deterministic testing.
            #[inline]
            pub fn with_base(window: Duration, base: Instant) -> Result<Self, crate::ConfigError> {
                let window_ns = u64::try_from(window.as_nanos())
                    .map_err(|_| crate::ConfigError::Invalid("window duration too large"))?;
                if window_ns == 0 {
                    return Err(crate::ConfigError::Invalid("window must be positive"));
                }
                let init = Sample {
                    timestamp: 0,
                    value: $init,
                };
                Ok(Self {
                    window: window_ns,
                    samples: [init; 3],
                    count: 0,
                    base,
                })
            }

            #[inline]
            fn nanos_since_base(&self, now: Instant) -> u64 {
                let nanos = now.saturating_duration_since(self.base).as_nanos();
                if nanos > u64::MAX as u128 {
                    u64::MAX
                } else {
                    nanos as u64
                }
            }

            /// Feeds a sample at the given time. Returns current window max.
            #[inline]
            #[must_use]
            pub fn update(&mut self, now: Instant, value: $ty) -> $ty {
                let timestamp = self.nanos_since_base(now);
                self.count += 1;
                let win = self.window;
                let s = &mut self.samples;

                // If new value >= current max, it becomes the new best
                if value >= s[0].value || timestamp.wrapping_sub(s[2].timestamp) > win {
                    // Reset all — new value is the best
                    s[0] = Sample { timestamp, value };
                    s[1] = s[0];
                    s[2] = s[0];
                    return s[0].value;
                }

                // If second sub-window candidate has expired, promote third
                if timestamp.wrapping_sub(s[1].timestamp) > win / 3 {
                    s[1] = Sample { timestamp, value };
                    s[2] = s[1];
                } else if timestamp.wrapping_sub(s[2].timestamp) > win / 3 {
                    s[2] = Sample { timestamp, value };
                }

                // Update second/third if better
                if value >= s[1].value {
                    s[1] = Sample { timestamp, value };
                    s[2] = s[1];
                } else if value >= s[2].value {
                    s[2] = Sample { timestamp, value };
                }

                // Check if best has expired
                if timestamp.wrapping_sub(s[0].timestamp) > win {
                    s[0] = s[1];
                    s[1] = s[2];
                    s[2] = Sample { timestamp, value };
                } else if timestamp.wrapping_sub(s[1].timestamp) > win / 3 {
                    s[1] = s[2];
                    s[2] = Sample { timestamp, value };
                }

                s[0].value
            }

            /// Current window maximum, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn max(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.samples[0].value)
                }
            }

            /// Window size as Duration.
            #[inline]
            #[must_use]
            pub fn window(&self) -> Duration {
                Duration::from_nanos(self.window)
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Resets to empty state with `now` as the new time base.
            ///
            /// Pass the same base `Instant` used at construction for
            /// deterministic testing.
            #[inline]
            pub fn reset(&mut self, now: Instant) {
                let init = Sample {
                    timestamp: 0,
                    value: $init,
                };
                self.samples = [init; 3];
                self.count = 0;
                self.base = now;
            }
        }
    };
}

#[cfg(feature = "std")]
macro_rules! impl_windowed_min {
    ($name:ident, $ty:ty, $init:expr) => {
        /// Windowed minimum tracker (Nichols' algorithm).
        ///
        /// Efficiently tracks the minimum value within a sliding time window
        /// using only 3 stored samples. O(1) amortized per update.
        ///
        /// # Use Cases
        /// - Min RTT tracking (BBR)
        /// - Minimum price in a window
        /// - Best-case latency estimation
        #[derive(Debug, Clone)]
        pub struct $name {
            window: u64,
            samples: [Sample<$ty>; 3],
            count: u64,
            base: Instant,
        }

        impl $name {
            /// Creates a new windowed min tracker with `Instant::now()` as base.
            #[inline]
            pub fn new(window: Duration) -> Result<Self, crate::ConfigError> {
                Self::with_base(window, Instant::now())
            }

            /// Creates a new windowed min tracker with an explicit base instant.
            #[inline]
            pub fn with_base(window: Duration, base: Instant) -> Result<Self, crate::ConfigError> {
                let window_ns = u64::try_from(window.as_nanos())
                    .map_err(|_| crate::ConfigError::Invalid("window duration too large"))?;
                if window_ns == 0 {
                    return Err(crate::ConfigError::Invalid("window must be positive"));
                }
                let init = Sample {
                    timestamp: 0,
                    value: $init,
                };
                Ok(Self {
                    window: window_ns,
                    samples: [init; 3],
                    count: 0,
                    base,
                })
            }

            #[inline]
            fn nanos_since_base(&self, now: Instant) -> u64 {
                let nanos = now.saturating_duration_since(self.base).as_nanos();
                if nanos > u64::MAX as u128 {
                    u64::MAX
                } else {
                    nanos as u64
                }
            }

            /// Feeds a sample at the given time. Returns current window min.
            #[inline]
            #[must_use]
            pub fn update(&mut self, now: Instant, value: $ty) -> $ty {
                let timestamp = self.nanos_since_base(now);
                self.count += 1;
                let win = self.window;
                let s = &mut self.samples;

                if value <= s[0].value || timestamp.wrapping_sub(s[2].timestamp) > win {
                    s[0] = Sample { timestamp, value };
                    s[1] = s[0];
                    s[2] = s[0];
                    return s[0].value;
                }

                if timestamp.wrapping_sub(s[1].timestamp) > win / 3 {
                    s[1] = Sample { timestamp, value };
                    s[2] = s[1];
                } else if timestamp.wrapping_sub(s[2].timestamp) > win / 3 {
                    s[2] = Sample { timestamp, value };
                }

                if value <= s[1].value {
                    s[1] = Sample { timestamp, value };
                    s[2] = s[1];
                } else if value <= s[2].value {
                    s[2] = Sample { timestamp, value };
                }

                if timestamp.wrapping_sub(s[0].timestamp) > win {
                    s[0] = s[1];
                    s[1] = s[2];
                    s[2] = Sample { timestamp, value };
                } else if timestamp.wrapping_sub(s[1].timestamp) > win / 3 {
                    s[1] = s[2];
                    s[2] = Sample { timestamp, value };
                }

                s[0].value
            }

            /// Current window minimum, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn min(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.samples[0].value)
                }
            }

            /// Window size as Duration.
            #[inline]
            #[must_use]
            pub fn window(&self) -> Duration {
                Duration::from_nanos(self.window)
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Resets to empty state with `now` as the new time base.
            ///
            /// Pass the same base `Instant` used at construction for
            /// deterministic testing.
            #[inline]
            pub fn reset(&mut self, now: Instant) {
                let init = Sample {
                    timestamp: 0,
                    value: $init,
                };
                self.samples = [init; 3];
                self.count = 0;
                self.base = now;
            }
        }
    };
}

#[cfg(feature = "std")]
impl_windowed_max!(WindowedMaxF64, f64, f64::MIN);
#[cfg(feature = "std")]
impl_windowed_max!(WindowedMaxF32, f32, f32::MIN);
#[cfg(feature = "std")]
impl_windowed_max!(WindowedMaxI64, i64, i64::MIN);
#[cfg(feature = "std")]
impl_windowed_max!(WindowedMaxI32, i32, i32::MIN);
#[cfg(feature = "std")]
impl_windowed_max!(WindowedMaxI128, i128, i128::MIN);

#[cfg(feature = "std")]
impl_windowed_min!(WindowedMinF64, f64, f64::MAX);
#[cfg(feature = "std")]
impl_windowed_min!(WindowedMinF32, f32, f32::MAX);
#[cfg(feature = "std")]
impl_windowed_min!(WindowedMinI64, i64, i64::MAX);
#[cfg(feature = "std")]
impl_windowed_min!(WindowedMinI32, i32, i32::MAX);
#[cfg(feature = "std")]
impl_windowed_min!(WindowedMinI128, i128, i128::MAX);

#[cfg(test)]
mod raw_tests {
    use super::*;

    #[test]
    fn raw_max_basic() {
        let mut wm = WindowedMaxF64Raw::new(100).unwrap();
        assert_eq!(wm.update(0, 10.0), 10.0);
        assert_eq!(wm.update(50, 20.0), 20.0);
    }

    #[test]
    fn raw_max_expires() {
        let mut wm = WindowedMaxF64Raw::new(10).unwrap();
        let _ = wm.update(0, 100.0);
        let _ = wm.update(5, 50.0);
        let result = wm.update(11, 60.0);
        assert!(result <= 60.0);
    }

    #[test]
    fn raw_min_basic() {
        let mut wm = WindowedMinI64Raw::new(100).unwrap();
        assert_eq!(wm.update(0, 100), 100);
        assert_eq!(wm.update(1, 50), 50);
    }

    #[test]
    fn raw_max_i64_convenience() {
        let mut wm = WindowedMaxF64Raw::new(1000).unwrap();
        assert_eq!(wm.update_i64(100i64, 42.0), 42.0);
    }

    #[test]
    fn raw_rejects_zero_window() {
        assert!(WindowedMaxF64Raw::new(0).is_err());
    }
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn t(base: Instant, nanos: u64) -> Instant {
        base + Duration::from_nanos(nanos)
    }

    // =========================================================================
    // Windowed Max
    // =========================================================================

    #[test]
    fn max_empty() {
        let base = Instant::now();
        let wm = WindowedMaxF64::with_base(Duration::from_nanos(100), base).unwrap();
        assert!(wm.max().is_none());
        assert_eq!(wm.count(), 0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn max_single_sample() {
        let base = Instant::now();
        let mut wm = WindowedMaxF64::with_base(Duration::from_nanos(100), base).unwrap();
        assert_eq!(wm.update(t(base, 0), 42.0), 42.0);
        assert_eq!(wm.max(), Some(42.0));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn max_new_peak_replaces() {
        let base = Instant::now();
        let mut wm = WindowedMaxF64::with_base(Duration::from_nanos(100), base).unwrap();
        let _ = wm.update(t(base, 0), 10.0);
        let _ = wm.update(t(base, 1), 20.0);
        assert_eq!(wm.update(t(base, 2), 30.0), 30.0);
    }

    #[test]
    fn max_expires_after_window() {
        let base = Instant::now();
        let mut wm = WindowedMaxF64::with_base(Duration::from_nanos(10), base).unwrap();
        let _ = wm.update(t(base, 0), 100.0); // peak at t=0
        let _ = wm.update(t(base, 5), 50.0);

        // At t=11, the peak at t=0 should have expired
        let result = wm.update(t(base, 11), 60.0);
        assert!(result <= 60.0, "old peak should have expired, got {result}");
    }

    #[test]
    fn max_reset() {
        let base = Instant::now();
        let mut wm = WindowedMaxF64::with_base(Duration::from_nanos(100), base).unwrap();
        let _ = wm.update(t(base, 0), 42.0);
        wm.reset(base);
        assert!(wm.max().is_none());
        assert_eq!(wm.count(), 0);
    }

    #[test]
    fn max_i64_basic() {
        let base = Instant::now();
        let mut wm = WindowedMaxI64::with_base(Duration::from_nanos(10), base).unwrap();
        assert_eq!(wm.update(t(base, 0), 100), 100);
        assert_eq!(wm.update(t(base, 1), 200), 200);
        assert_eq!(wm.update(t(base, 2), 150), 200);
    }

    #[test]
    fn max_monotonic_decreasing_tracks_recent() {
        let base = Instant::now();
        let mut wm = WindowedMaxF64::with_base(Duration::from_nanos(10), base).unwrap();
        // Decreasing values — max should eventually drop as window slides
        for ts in 0..20u64 {
            let v = 100.0 - ts as f64;
            let _ = wm.update(t(base, ts), v);
        }
        // The max should not still be 100.0 (that was at t=0, window=10, now at t=19)
        let m = wm.max().unwrap();
        assert!(m < 100.0, "old max should have expired, got {m}");
    }

    // =========================================================================
    // Windowed Min
    // =========================================================================

    #[test]
    fn min_empty() {
        let base = Instant::now();
        let wm = WindowedMinF64::with_base(Duration::from_nanos(100), base).unwrap();
        assert!(wm.min().is_none());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn min_single_sample() {
        let base = Instant::now();
        let mut wm = WindowedMinF64::with_base(Duration::from_nanos(100), base).unwrap();
        assert_eq!(wm.update(t(base, 0), 42.0), 42.0);
        assert_eq!(wm.min(), Some(42.0));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn min_new_low_replaces() {
        let base = Instant::now();
        let mut wm = WindowedMinF64::with_base(Duration::from_nanos(100), base).unwrap();
        let _ = wm.update(t(base, 0), 30.0);
        let _ = wm.update(t(base, 1), 20.0);
        assert_eq!(wm.update(t(base, 2), 10.0), 10.0);
    }

    #[test]
    fn min_expires_after_window() {
        let base = Instant::now();
        let mut wm = WindowedMinF64::with_base(Duration::from_nanos(10), base).unwrap();
        let _ = wm.update(t(base, 0), 10.0); // min at t=0
        let _ = wm.update(t(base, 5), 50.0);

        // At t=11, the min at t=0 should have expired
        let result = wm.update(t(base, 11), 40.0);
        assert!(result >= 40.0, "old min should have expired, got {result}");
    }

    #[test]
    fn min_rtt_tracking() {
        let base = Instant::now();
        // Simulating BBR min RTT tracking
        let mut min_rtt = WindowedMinI64::with_base(Duration::from_nanos(100), base).unwrap();

        // Normal RTTs around 50
        for ts in 0..50 {
            let _ = min_rtt.update(t(base, ts), 50 + (ts % 5) as i64);
        }
        assert!(min_rtt.min().unwrap() <= 50);

        // Spike to 200, then back to normal
        let _ = min_rtt.update(t(base, 50), 200);
        // Min should still be the normal value, not the spike
        assert!(min_rtt.min().unwrap() <= 54);
    }

    #[test]
    fn min_reset() {
        let base = Instant::now();
        let mut wm = WindowedMinF64::with_base(Duration::from_nanos(100), base).unwrap();
        let _ = wm.update(t(base, 0), 42.0);
        wm.reset(base);
        assert!(wm.min().is_none());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn min_f32_basic() {
        let base = Instant::now();
        let mut wm = WindowedMinF32::with_base(Duration::from_nanos(10), base).unwrap();
        assert_eq!(wm.update(t(base, 0), 50.0), 50.0);
        assert_eq!(wm.update(t(base, 1), 30.0), 30.0);
        assert_eq!(wm.update(t(base, 2), 40.0), 30.0);
    }

    #[test]
    fn min_i32_basic() {
        let base = Instant::now();
        let mut wm = WindowedMinI32::with_base(Duration::from_nanos(10), base).unwrap();
        assert_eq!(wm.update(t(base, 0), 100), 100);
        assert_eq!(wm.update(t(base, 1), 50), 50);
        assert_eq!(wm.update(t(base, 2), 75), 50);
    }

    #[test]
    fn max_rejects_zero_window() {
        let base = Instant::now();
        assert!(matches!(
            WindowedMaxF64::with_base(Duration::from_nanos(0), base),
            Err(crate::ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn min_rejects_zero_window() {
        let base = Instant::now();
        assert!(matches!(
            WindowedMinF64::with_base(Duration::from_nanos(0), base),
            Err(crate::ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn max_i128_basic() {
        let base = Instant::now();
        let mut wm = WindowedMaxI128::with_base(Duration::from_nanos(10), base).unwrap();
        assert_eq!(wm.update(t(base, 0), 100), 100);
        assert_eq!(wm.update(t(base, 1), 200), 200);
        assert_eq!(wm.update(t(base, 2), 150), 200);
    }

    #[test]
    fn min_i128_basic() {
        let base = Instant::now();
        let mut wm = WindowedMinI128::with_base(Duration::from_nanos(10), base).unwrap();
        assert_eq!(wm.update(t(base, 0), 100), 100);
        assert_eq!(wm.update(t(base, 1), 50), 50);
        assert_eq!(wm.update(t(base, 2), 75), 50);
    }

    #[test]
    fn window_overflow_returns_error() {
        let result = WindowedMaxF64::new(Duration::from_secs(u64::MAX));
        assert!(matches!(result, Err(crate::ConfigError::Invalid(_))));
    }
}
