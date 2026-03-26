macro_rules! impl_drawdown_float {
    ($name:ident, $ty:ty, $zero:expr) => {
        /// Drawdown monitor — tracks peak value and current/maximum drawdown.
        ///
        /// Drawdown is the decline from peak to current value. Useful for risk
        /// monitoring and circuit breakers ("if PnL drops $X from peak, halt").
        ///
        /// # Use Cases
        /// - PnL circuit breaker
        /// - Position risk monitoring
        /// - Performance tracking (max drawdown as a risk metric)
        #[derive(Debug, Clone)]
        pub struct $name {
            peak: $ty,
            current: $ty,
            max_drawdown: $ty,
            count: u64,
        }

        impl $name {
            /// Creates a new empty drawdown monitor.
            #[inline]
            #[must_use]
            pub const fn new() -> Self {
                Self {
                    peak: $zero,
                    current: $zero,
                    max_drawdown: $zero,
                    count: 0,
                }
            }

            /// Feeds a sample. Returns the current drawdown (peak - current).
            /// Returns 0 at new peaks.
            ///
            /// # Errors
            ///
            /// Returns `DataError::NotANumber` if the sample is NaN, or
            /// `DataError::Infinite` if the sample is infinite.
            #[inline]
            pub fn update(&mut self, sample: $ty) -> Result<$ty, crate::DataError> {
                check_finite!(sample);
                self.count += 1;
                self.current = sample;

                if self.count == 1 || sample > self.peak {
                    self.peak = sample;
                }

                let dd = self.peak - self.current;
                if dd > self.max_drawdown {
                    self.max_drawdown = dd;
                }

                Ok(dd)
            }

            /// Highest value seen, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn peak(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.peak)
                }
            }

            /// Current drawdown (peak - last sample). Zero if empty.
            #[inline]
            #[must_use]
            pub fn drawdown(&self) -> $ty {
                if self.count == 0 {
                    $zero
                } else {
                    self.peak - self.current
                }
            }

            /// Worst drawdown ever observed. Zero if empty.
            #[inline]
            #[must_use]
            pub fn max_drawdown(&self) -> $ty {
                self.max_drawdown
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
                self.peak = $zero;
                self.current = $zero;
                self.max_drawdown = $zero;
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

macro_rules! impl_drawdown_int {
    ($name:ident, $ty:ty, $zero:expr) => {
        /// Drawdown monitor — tracks peak value and current/maximum drawdown.
        ///
        /// Drawdown is the decline from peak to current value. Useful for risk
        /// monitoring and circuit breakers ("if PnL drops $X from peak, halt").
        ///
        /// # Use Cases
        /// - PnL circuit breaker
        /// - Position risk monitoring
        /// - Performance tracking (max drawdown as a risk metric)
        #[derive(Debug, Clone)]
        pub struct $name {
            peak: $ty,
            current: $ty,
            max_drawdown: $ty,
            count: u64,
        }

        impl $name {
            /// Creates a new empty drawdown monitor.
            #[inline]
            #[must_use]
            pub const fn new() -> Self {
                Self {
                    peak: $zero,
                    current: $zero,
                    max_drawdown: $zero,
                    count: 0,
                }
            }

            /// Feeds a sample. Returns the current drawdown (peak - current).
            /// Returns 0 at new peaks.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> $ty {
                self.count += 1;
                self.current = sample;

                if self.count == 1 || sample > self.peak {
                    self.peak = sample;
                }

                let dd = self.peak - self.current;
                if dd > self.max_drawdown {
                    self.max_drawdown = dd;
                }

                dd
            }

            /// Highest value seen, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn peak(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.peak)
                }
            }

            /// Current drawdown (peak - last sample). Zero if empty.
            #[inline]
            #[must_use]
            pub fn drawdown(&self) -> $ty {
                if self.count == 0 {
                    $zero
                } else {
                    self.peak - self.current
                }
            }

            /// Worst drawdown ever observed. Zero if empty.
            #[inline]
            #[must_use]
            pub fn max_drawdown(&self) -> $ty {
                self.max_drawdown
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
                self.peak = $zero;
                self.current = $zero;
                self.max_drawdown = $zero;
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

impl_drawdown_float!(DrawdownF64, f64, 0.0);
impl_drawdown_float!(DrawdownF32, f32, 0.0);
impl_drawdown_int!(DrawdownI64, i64, 0);
impl_drawdown_int!(DrawdownI32, i32, 0);
impl_drawdown_int!(DrawdownI128, i128, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)]
    fn empty_state() {
        let dd = DrawdownF64::new();
        assert_eq!(dd.count(), 0);
        assert!(!dd.is_primed());
        assert!(dd.peak().is_none());
        assert_eq!(dd.drawdown(), 0.0);
        assert_eq!(dd.max_drawdown(), 0.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn first_sample_sets_peak() {
        let mut dd = DrawdownF64::new();
        let result = dd.update(100.0).unwrap();
        assert_eq!(result, 0.0); // no drawdown at first sample
        assert_eq!(dd.peak(), Some(100.0));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn drawdown_from_peak() {
        let mut dd = DrawdownF64::new();
        let _ = dd.update(100.0).unwrap();
        let result = dd.update(90.0).unwrap();
        assert_eq!(result, 10.0);
        assert_eq!(dd.drawdown(), 10.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn new_peak_resets_drawdown() {
        let mut dd = DrawdownF64::new();
        let _ = dd.update(100.0).unwrap();
        let _ = dd.update(90.0).unwrap();
        assert_eq!(dd.drawdown(), 10.0);

        let result = dd.update(110.0).unwrap(); // new peak
        assert_eq!(result, 0.0);
        assert_eq!(dd.peak(), Some(110.0));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn max_drawdown_tracks_worst() {
        let mut dd = DrawdownF64::new();
        let _ = dd.update(100.0).unwrap();
        let _ = dd.update(80.0).unwrap(); // drawdown = 20
        let _ = dd.update(110.0).unwrap(); // new peak, drawdown = 0
        let _ = dd.update(100.0).unwrap(); // drawdown = 10

        assert_eq!(dd.max_drawdown(), 20.0); // worst was 20
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn reset_clears_all() {
        let mut dd = DrawdownF64::new();
        let _ = dd.update(100.0).unwrap();
        let _ = dd.update(80.0).unwrap();

        dd.reset();
        assert_eq!(dd.count(), 0);
        assert!(dd.peak().is_none());
        assert_eq!(dd.max_drawdown(), 0.0);
    }

    #[test]
    fn default_is_empty() {
        let dd = DrawdownF64::default();
        assert_eq!(dd.count(), 0);
    }

    #[test]
    fn i64_basic() {
        let mut dd = DrawdownI64::new();
        let _ = dd.update(1000);
        let _ = dd.update(800);
        assert_eq!(dd.drawdown(), 200);
        assert_eq!(dd.max_drawdown(), 200);
    }

    #[test]
    fn i32_basic() {
        let mut dd = DrawdownI32::new();
        let _ = dd.update(100);
        assert_eq!(dd.update(90), 10);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn f32_basic() {
        let mut dd = DrawdownF32::new();
        let _ = dd.update(100.0).unwrap();
        assert_eq!(dd.update(95.0).unwrap(), 5.0);
    }

    #[test]
    fn i128_basic() {
        let mut dd = DrawdownI128::new();
        let _ = dd.update(1000);
        assert_eq!(dd.update(800), 200);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn monotonic_increasing_no_drawdown() {
        let mut dd = DrawdownF64::new();
        for i in 0..100 {
            let result = dd.update(i as f64).unwrap();
            assert_eq!(result, 0.0);
        }
        assert_eq!(dd.max_drawdown(), 0.0);
    }

    #[test]
    fn rejects_nan_and_inf() {
        let mut dd = DrawdownF64::new();
        assert!(matches!(
            dd.update(f64::NAN),
            Err(crate::DataError::NotANumber)
        ));
        assert!(matches!(
            dd.update(f64::INFINITY),
            Err(crate::DataError::Infinite)
        ));
        assert!(matches!(
            dd.update(f64::NEG_INFINITY),
            Err(crate::DataError::Infinite)
        ));

        let mut dd32 = DrawdownF32::new();
        assert!(matches!(
            dd32.update(f32::NAN),
            Err(crate::DataError::NotANumber)
        ));
        assert!(matches!(
            dd32.update(f32::INFINITY),
            Err(crate::DataError::Infinite)
        ));
    }
}
