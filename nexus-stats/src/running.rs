macro_rules! impl_running_min {
    ($name:ident, $ty:ty, $init:expr) => {
        /// All-time minimum tracker.
        ///
        /// Tracks the smallest value ever seen. One comparison per update.
        ///
        /// # Use Cases
        /// - Best-case latency tracking (all-time min RTT)
        /// - Low-water mark for prices or levels
        /// - Input to range calculations (max - min)
        #[derive(Debug, Clone)]
        pub struct $name {
            min: $ty,
            count: u64,
        }

        impl $name {
            /// Creates a new empty tracker.
            #[inline]
            #[must_use]
            pub const fn new() -> Self {
                Self {
                    min: $init,
                    count: 0,
                }
            }

            /// Feeds a sample. Returns the current all-time minimum.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> $ty {
                self.count += 1;
                if sample < self.min {
                    self.min = sample;
                }
                self.min
            }

            /// All-time minimum, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn min(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.min)
                }
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether at least one sample has been fed.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count > 0
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.min = $init;
                self.count = 0;
            }
        }

        impl Default for $name {
            #[inline]
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

macro_rules! impl_running_max {
    ($name:ident, $ty:ty, $init:expr) => {
        /// All-time maximum tracker.
        ///
        /// Tracks the largest value ever seen. One comparison per update.
        ///
        /// # Use Cases
        /// - High-water mark tracking (peak throughput, max latency)
        /// - Capacity planning (peak resource usage)
        /// - Input to range calculations (max - min)
        #[derive(Debug, Clone)]
        pub struct $name {
            max: $ty,
            count: u64,
        }

        impl $name {
            /// Creates a new empty tracker.
            #[inline]
            #[must_use]
            pub const fn new() -> Self {
                Self {
                    max: $init,
                    count: 0,
                }
            }

            /// Feeds a sample. Returns the current all-time maximum.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> $ty {
                self.count += 1;
                if sample > self.max {
                    self.max = sample;
                }
                self.max
            }

            /// All-time maximum, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn max(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.max)
                }
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Whether at least one sample has been fed.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.count > 0
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.max = $init;
                self.count = 0;
            }
        }

        impl Default for $name {
            #[inline]
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

impl_running_min!(RunningMinF64, f64, f64::MAX);
impl_running_min!(RunningMinF32, f32, f32::MAX);
impl_running_min!(RunningMinI64, i64, i64::MAX);
impl_running_min!(RunningMinI32, i32, i32::MAX);
impl_running_min!(RunningMinI128, i128, i128::MAX);

impl_running_max!(RunningMaxF64, f64, f64::MIN);
impl_running_max!(RunningMaxF32, f32, f32::MIN);
impl_running_max!(RunningMaxI64, i64, i64::MIN);
impl_running_max!(RunningMaxI32, i32, i32::MIN);
impl_running_max!(RunningMaxI128, i128, i128::MIN);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_empty() {
        let rm = RunningMinF64::new();
        assert!(rm.min().is_none());
        assert!(!rm.is_primed());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn min_tracks() {
        let mut rm = RunningMinF64::new();
        assert_eq!(rm.update(50.0), 50.0);
        assert_eq!(rm.update(30.0), 30.0);
        assert_eq!(rm.update(40.0), 30.0); // still 30
        assert_eq!(rm.update(10.0), 10.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn min_reset() {
        let mut rm = RunningMinF64::new();
        let _ = rm.update(10.0);
        rm.reset();
        assert!(rm.min().is_none());
        assert_eq!(rm.update(50.0), 50.0);
    }

    #[test]
    fn min_i64() {
        let mut rm = RunningMinI64::new();
        assert_eq!(rm.update(100), 100);
        assert_eq!(rm.update(50), 50);
        assert_eq!(rm.update(75), 50);
    }

    #[test]
    fn min_default() {
        let rm = RunningMinF64::default();
        assert_eq!(rm.count(), 0);
    }

    #[test]
    fn max_empty() {
        let rm = RunningMaxF64::new();
        assert!(rm.max().is_none());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn max_tracks() {
        let mut rm = RunningMaxF64::new();
        assert_eq!(rm.update(30.0), 30.0);
        assert_eq!(rm.update(50.0), 50.0);
        assert_eq!(rm.update(40.0), 50.0); // still 50
        assert_eq!(rm.update(90.0), 90.0);
    }

    #[test]
    fn max_i64() {
        let mut rm = RunningMaxI64::new();
        assert_eq!(rm.update(50), 50);
        assert_eq!(rm.update(100), 100);
        assert_eq!(rm.update(75), 100);
    }

    #[test]
    fn max_i32() {
        let mut rm = RunningMaxI32::new();
        assert_eq!(rm.update(10), 10);
        assert_eq!(rm.update(20), 20);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn min_f32() {
        let mut rm = RunningMinF32::new();
        assert_eq!(rm.update(50.0), 50.0);
        assert_eq!(rm.update(30.0), 30.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn max_f32() {
        let mut rm = RunningMaxF32::new();
        assert_eq!(rm.update(30.0), 30.0);
        assert_eq!(rm.update(50.0), 50.0);
    }

    #[test]
    fn min_i128() {
        let mut rm = RunningMinI128::new();
        assert_eq!(rm.update(100), 100);
        assert_eq!(rm.update(50), 50);
        assert_eq!(rm.update(75), 50);
    }

    #[test]
    fn max_i128() {
        let mut rm = RunningMaxI128::new();
        assert_eq!(rm.update(50), 50);
        assert_eq!(rm.update(100), 100);
        assert_eq!(rm.update(75), 100);
    }
}
