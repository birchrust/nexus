macro_rules! impl_first_diff {
    ($name:ident, $ty:ty, $zero:expr) => {
        /// First difference — `x[n] - x[n-1]`.
        ///
        /// Returns the change between consecutive samples.
        /// `None` on the first sample (no previous value to diff against).
        ///
        /// # Use Cases
        /// - Computing returns from prices
        /// - Velocity from position
        /// - Rate of change of any signal
        #[derive(Debug, Clone)]
        pub struct $name {
            prev: $ty,
            initialized: bool,
        }

        impl $name {
            /// Creates a new first-difference filter.
            #[inline]
            #[must_use]
            pub const fn new() -> Self {
                Self {
                    prev: $zero,
                    initialized: false,
                }
            }

            /// Feeds a sample. Returns `Some(x[n] - x[n-1])` or `None` on first sample.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<$ty> {
                if !self.initialized {
                    self.prev = sample;
                    self.initialized = true;
                    return Option::None;
                }
                let diff = sample - self.prev;
                self.prev = sample;
                Option::Some(diff)
            }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.prev = $zero;
                self.initialized = false;
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

macro_rules! impl_second_diff {
    ($name:ident, $ty:ty, $zero:expr) => {
        /// Second difference — `x[n] - 2*x[n-1] + x[n-2]`.
        ///
        /// Returns the acceleration (change in change) of a signal.
        /// `None` until the third sample.
        ///
        /// # Use Cases
        /// - Acceleration from position
        /// - Curvature detection
        /// - "Is the rate of change itself changing?"
        #[derive(Debug, Clone)]
        pub struct $name {
            prev2: $ty,
            prev1: $ty,
            count: u64,
        }

        impl $name {
            /// Creates a new second-difference filter.
            #[inline]
            #[must_use]
            pub const fn new() -> Self {
                Self {
                    prev2: $zero,
                    prev1: $zero,
                    count: 0,
                }
            }

            /// Feeds a sample. Returns `Some(x[n] - 2*x[n-1] + x[n-2])` or `None`
            /// until 3 samples have been fed.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<$ty> {
                self.count += 1;

                if self.count == 1 {
                    self.prev1 = sample;
                    return Option::None;
                }
                if self.count == 2 {
                    self.prev2 = self.prev1;
                    self.prev1 = sample;
                    return Option::None;
                }

                let two = (2 as $ty);
                let diff2 = sample - two * self.prev1 + self.prev2;
                self.prev2 = self.prev1;
                self.prev1 = sample;
                Option::Some(diff2)
            }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.prev2 = $zero;
                self.prev1 = $zero;
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

impl_first_diff!(FirstDiffF64, f64, 0.0);
impl_first_diff!(FirstDiffF32, f32, 0.0);
impl_first_diff!(FirstDiffI64, i64, 0);
impl_first_diff!(FirstDiffI32, i32, 0);
impl_first_diff!(FirstDiffI128, i128, 0);

impl_second_diff!(SecondDiffF64, f64, 0.0);
impl_second_diff!(SecondDiffF32, f32, 0.0);
impl_second_diff!(SecondDiffI64, i64, 0);
impl_second_diff!(SecondDiffI32, i32, 0);
impl_second_diff!(SecondDiffI128, i128, 0);

#[cfg(test)]
mod tests {
    use super::*;

    // First diff
    #[test]
    fn first_diff_none_on_first() {
        let mut fd = FirstDiffF64::new();
        assert!(fd.update(100.0).is_none());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn first_diff_computes() {
        let mut fd = FirstDiffF64::new();
        let _ = fd.update(100.0);
        assert_eq!(fd.update(110.0), Some(10.0));
        assert_eq!(fd.update(105.0), Some(-5.0));
    }

    #[test]
    fn first_diff_i64() {
        let mut fd = FirstDiffI64::new();
        let _ = fd.update(100);
        assert_eq!(fd.update(130), Some(30));
    }

    #[test]
    fn first_diff_reset() {
        let mut fd = FirstDiffF64::new();
        let _ = fd.update(100.0);
        fd.reset();
        assert!(fd.update(50.0).is_none()); // re-initialized
    }

    // Second diff
    #[test]
    fn second_diff_none_until_third() {
        let mut sd = SecondDiffF64::new();
        assert!(sd.update(1.0).is_none());
        assert!(sd.update(2.0).is_none());
        assert!(sd.update(3.0).is_some());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn second_diff_linear_is_zero() {
        let mut sd = SecondDiffF64::new();
        let _ = sd.update(10.0);
        let _ = sd.update(20.0);
        // Linear: 10, 20, 30 → second diff = 30 - 2*20 + 10 = 0
        assert_eq!(sd.update(30.0), Some(0.0));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn second_diff_quadratic() {
        let mut sd = SecondDiffF64::new();
        // x^2: 1, 4, 9 → 9 - 2*4 + 1 = 2
        let _ = sd.update(1.0);
        let _ = sd.update(4.0);
        assert_eq!(sd.update(9.0), Some(2.0));
    }

    #[test]
    fn second_diff_i64() {
        let mut sd = SecondDiffI64::new();
        let _ = sd.update(10);
        let _ = sd.update(20);
        assert_eq!(sd.update(30), Some(0)); // linear
        assert_eq!(sd.update(50), Some(10)); // acceleration
    }

    #[test]
    fn second_diff_reset() {
        let mut sd = SecondDiffF64::new();
        let _ = sd.update(1.0);
        let _ = sd.update(2.0);
        sd.reset();
        assert!(sd.update(5.0).is_none());
    }

    #[test]
    fn first_diff_i128() {
        let mut fd = FirstDiffI128::new();
        let _ = fd.update(100);
        assert_eq!(fd.update(130), Some(30));
    }

    #[test]
    fn second_diff_i128() {
        let mut sd = SecondDiffI128::new();
        let _ = sd.update(10);
        let _ = sd.update(20);
        assert_eq!(sd.update(30), Some(0)); // linear
    }
}
