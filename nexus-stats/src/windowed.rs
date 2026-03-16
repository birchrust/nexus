// Windowed extrema using Nichols' 3-sample sub-window promotion algorithm.
//
// Ported from the Linux kernel's `win_minmax.h` (used by TCP BBR).
// Maintains the window extremum using only 3 stored samples, each covering
// a sub-window of `window/3` ticks. When a sub-window expires, the next
// candidate is promoted.
//
// State: 3 × (timestamp, value) — 48 bytes for f64, 40 bytes for i64.

/// Internal sample stored per sub-window.
#[derive(Debug, Clone, Copy)]
struct Sample<T: Copy> {
    timestamp: u64,
    value: T,
}

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
        }

        impl $name {
            /// Creates a new windowed max tracker.
            ///
            /// `window` is the size of the sliding window in timestamp units.
            /// The caller defines the timestamp semantics (nanoseconds, ticks, etc.).
            #[inline]
            pub fn new(window: u64) -> Result<Self, crate::ConfigError> {
                if window == 0 {
                    return Err(crate::ConfigError::Invalid("window must be positive"));
                }
                let init = Sample { timestamp: 0, value: $init };
                Ok(Self {
                    window,
                    samples: [init; 3],
                    count: 0,
                })
            }

            /// Feeds a sample at the given timestamp. Returns current window max.
            ///
            /// Timestamps must be monotonically non-decreasing.
            #[inline]
            #[must_use]
            pub fn update(&mut self, timestamp: u64, value: $ty) -> $ty {
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

            /// Window size.
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

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                let init = Sample { timestamp: 0, value: $init };
                self.samples = [init; 3];
                self.count = 0;
            }
        }
    };
}

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
        }

        impl $name {
            /// Creates a new windowed min tracker.
            #[inline]
            pub fn new(window: u64) -> Result<Self, crate::ConfigError> {
                if window == 0 {
                    return Err(crate::ConfigError::Invalid("window must be positive"));
                }
                let init = Sample { timestamp: 0, value: $init };
                Ok(Self {
                    window,
                    samples: [init; 3],
                    count: 0,
                })
            }

            /// Feeds a sample at the given timestamp. Returns current window min.
            ///
            /// Timestamps must be monotonically non-decreasing.
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

            /// Window size.
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

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                let init = Sample { timestamp: 0, value: $init };
                self.samples = [init; 3];
                self.count = 0;
            }
        }
    };
}

impl_windowed_max!(WindowedMaxF64, f64, f64::MIN);
impl_windowed_max!(WindowedMaxF32, f32, f32::MIN);
impl_windowed_max!(WindowedMaxI64, i64, i64::MIN);
impl_windowed_max!(WindowedMaxI32, i32, i32::MIN);

impl_windowed_min!(WindowedMinF64, f64, f64::MAX);
impl_windowed_min!(WindowedMinF32, f32, f32::MAX);
impl_windowed_min!(WindowedMinI64, i64, i64::MAX);
impl_windowed_min!(WindowedMinI32, i32, i32::MAX);

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Windowed Max
    // =========================================================================

    #[test]
    fn max_empty() {
        let wm = WindowedMaxF64::new(100).unwrap();
        assert!(wm.max().is_none());
        assert_eq!(wm.count(), 0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn max_single_sample() {
        let mut wm = WindowedMaxF64::new(100).unwrap();
        assert_eq!(wm.update(0, 42.0), 42.0);
        assert_eq!(wm.max(), Some(42.0));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn max_new_peak_replaces() {
        let mut wm = WindowedMaxF64::new(100).unwrap();
        let _ = wm.update(0, 10.0);
        let _ = wm.update(1, 20.0);
        assert_eq!(wm.update(2, 30.0), 30.0);
    }

    #[test]
    fn max_expires_after_window() {
        let mut wm = WindowedMaxF64::new(10).unwrap();
        let _ = wm.update(0, 100.0); // peak at t=0
        let _ = wm.update(5, 50.0);

        // At t=11, the peak at t=0 should have expired
        let result = wm.update(11, 60.0);
        assert!(result <= 60.0, "old peak should have expired, got {result}");
    }

    #[test]
    fn max_reset() {
        let mut wm = WindowedMaxF64::new(100).unwrap();
        let _ = wm.update(0, 42.0);
        wm.reset();
        assert!(wm.max().is_none());
        assert_eq!(wm.count(), 0);
    }

    #[test]
    fn max_i64_basic() {
        let mut wm = WindowedMaxI64::new(10).unwrap();
        assert_eq!(wm.update(0, 100), 100);
        assert_eq!(wm.update(1, 200), 200);
        assert_eq!(wm.update(2, 150), 200);
    }

    #[test]
    fn max_monotonic_decreasing_tracks_recent() {
        let mut wm = WindowedMaxF64::new(10).unwrap();
        // Decreasing values — max should eventually drop as window slides
        for t in 0..20u64 {
            let v = 100.0 - t as f64;
            let _ = wm.update(t, v);
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
        let wm = WindowedMinF64::new(100).unwrap();
        assert!(wm.min().is_none());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn min_single_sample() {
        let mut wm = WindowedMinF64::new(100).unwrap();
        assert_eq!(wm.update(0, 42.0), 42.0);
        assert_eq!(wm.min(), Some(42.0));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn min_new_low_replaces() {
        let mut wm = WindowedMinF64::new(100).unwrap();
        let _ = wm.update(0, 30.0);
        let _ = wm.update(1, 20.0);
        assert_eq!(wm.update(2, 10.0), 10.0);
    }

    #[test]
    fn min_expires_after_window() {
        let mut wm = WindowedMinF64::new(10).unwrap();
        let _ = wm.update(0, 10.0); // min at t=0
        let _ = wm.update(5, 50.0);

        // At t=11, the min at t=0 should have expired
        let result = wm.update(11, 40.0);
        assert!(result >= 40.0, "old min should have expired, got {result}");
    }

    #[test]
    fn min_rtt_tracking() {
        // Simulating BBR min RTT tracking
        let mut min_rtt = WindowedMinI64::new(100).unwrap();

        // Normal RTTs around 50
        for t in 0..50 {
            let _ = min_rtt.update(t, 50 + (t % 5) as i64);
        }
        assert!(min_rtt.min().unwrap() <= 50);

        // Spike to 200, then back to normal
        let _ = min_rtt.update(50, 200);
        // Min should still be the normal value, not the spike
        assert!(min_rtt.min().unwrap() <= 54);
    }

    #[test]
    fn min_reset() {
        let mut wm = WindowedMinF64::new(100).unwrap();
        let _ = wm.update(0, 42.0);
        wm.reset();
        assert!(wm.min().is_none());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn min_f32_basic() {
        let mut wm = WindowedMinF32::new(10).unwrap();
        assert_eq!(wm.update(0, 50.0), 50.0);
        assert_eq!(wm.update(1, 30.0), 30.0);
        assert_eq!(wm.update(2, 40.0), 30.0);
    }

    #[test]
    fn min_i32_basic() {
        let mut wm = WindowedMinI32::new(10).unwrap();
        assert_eq!(wm.update(0, 100), 100);
        assert_eq!(wm.update(1, 50), 50);
        assert_eq!(wm.update(2, 75), 50);
    }

    #[test]
    fn max_rejects_zero_window() {
        assert!(matches!(WindowedMaxF64::new(0), Err(crate::ConfigError::Invalid(_))));
    }

    #[test]
    fn min_rejects_zero_window() {
        assert!(matches!(WindowedMinF64::new(0), Err(crate::ConfigError::Invalid(_))));
    }
}
