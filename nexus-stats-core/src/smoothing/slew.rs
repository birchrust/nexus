macro_rules! impl_slew_float {
    ($name:ident, $ty:ty) => {
        /// Slew rate limiter — clamps output change per sample.
        ///
        /// Limits how fast the output can change between consecutive samples.
        /// Useful for preventing sudden jumps from propagating through a system.
        ///
        /// # Use Cases
        /// - Smoothing control signals
        /// - Preventing sudden parameter changes from destabilizing a system
        /// - Rate-limiting position updates
        #[derive(Debug, Clone)]
        pub struct $name {
            max_rate: $ty,
            value: $ty,
            initialized: bool,
        }

        impl $name {
            /// Creates a new slew limiter with the given maximum change per sample.
            #[inline]
            pub fn new(max_rate: $ty) -> Result<Self, crate::ConfigError> {
                #[allow(clippy::neg_cmp_op_on_partial_ord)]
                if !(max_rate > 0.0 as $ty) {
                    return Err(crate::ConfigError::Invalid("max_rate must be positive"));
                }
                Ok(Self {
                    max_rate,
                    value: 0.0 as $ty,
                    initialized: false,
                })
            }

            /// Feeds a sample. Returns the rate-limited output.
            ///
            /// # Errors
            ///
            /// Returns `DataError::NotANumber` if the sample is NaN, or
            /// `DataError::Infinite` if the sample is infinite.
            #[inline]
            pub fn update(&mut self, sample: $ty) -> Result<$ty, crate::DataError> {
                check_finite!(sample);
                if !self.initialized {
                    self.value = sample;
                    self.initialized = true;
                    return Ok(sample);
                }

                self.value = sample.clamp(self.value - self.max_rate, self.value + self.max_rate);
                Ok(self.value)
            }

            /// Current output value.
            #[inline]
            #[must_use]
            pub fn value(&self) -> $ty {
                self.value
            }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.value = 0.0 as $ty;
                self.initialized = false;
            }
        }
    };
}

macro_rules! impl_slew_int {
    ($name:ident, $ty:ty, $zero:expr) => {
        /// Slew rate limiter — clamps output change per sample.
        ///
        /// Limits how fast the output can change between consecutive samples.
        /// Useful for preventing sudden jumps from propagating through a system.
        ///
        /// # Use Cases
        /// - Smoothing control signals
        /// - Preventing sudden parameter changes from destabilizing a system
        /// - Rate-limiting position updates
        #[derive(Debug, Clone)]
        pub struct $name {
            max_rate: $ty,
            value: $ty,
            initialized: bool,
        }

        impl $name {
            /// Creates a new slew limiter with the given maximum change per sample.
            #[inline]
            pub fn new(max_rate: $ty) -> Result<Self, crate::ConfigError> {
                #[allow(clippy::neg_cmp_op_on_partial_ord)]
                if !(max_rate > $zero) {
                    return Err(crate::ConfigError::Invalid("max_rate must be positive"));
                }
                Ok(Self {
                    max_rate,
                    value: $zero,
                    initialized: false,
                })
            }

            /// Feeds a sample. Returns the rate-limited output.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> $ty {
                if !self.initialized {
                    self.value = sample;
                    self.initialized = true;
                    return sample;
                }

                self.value = sample.clamp(self.value - self.max_rate, self.value + self.max_rate);
                self.value
            }

            /// Current output value.
            #[inline]
            #[must_use]
            pub fn value(&self) -> $ty {
                self.value
            }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.value = $zero;
                self.initialized = false;
            }
        }
    };
}

impl_slew_float!(SlewF64, f64);
impl_slew_float!(SlewF32, f32);
impl_slew_int!(SlewI64, i64, 0);
impl_slew_int!(SlewI32, i32, 0);
impl_slew_int!(SlewI128, i128, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)]
    fn spike_clamped() {
        let mut s = SlewF64::new(10.0).unwrap();
        assert_eq!(s.update(100.0).unwrap(), 100.0); // first sample, pass through
        assert_eq!(s.update(200.0).unwrap(), 110.0); // clamped: 100 + 10
        assert_eq!(s.update(200.0).unwrap(), 120.0); // clamped: 110 + 10
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn gradual_passes_through() {
        let mut s = SlewF64::new(10.0).unwrap();
        assert_eq!(s.update(100.0).unwrap(), 100.0);
        assert_eq!(s.update(105.0).unwrap(), 105.0); // within rate, passes through
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn negative_clamped() {
        let mut s = SlewF64::new(10.0).unwrap();
        assert_eq!(s.update(100.0).unwrap(), 100.0);
        assert_eq!(s.update(50.0).unwrap(), 90.0); // clamped: 100 - 10
    }

    #[test]
    fn i64_basic() {
        let mut s = SlewI64::new(5).unwrap();
        assert_eq!(s.update(100), 100);
        assert_eq!(s.update(200), 105);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn reset() {
        let mut s = SlewF64::new(10.0).unwrap();
        s.update(100.0).unwrap();
        s.reset();
        assert_eq!(s.update(50.0).unwrap(), 50.0); // re-initialized
    }

    #[test]
    fn rejects_zero_max_rate() {
        assert!(matches!(
            SlewF64::new(0.0),
            Err(crate::ConfigError::Invalid(_))
        ));
        assert!(matches!(
            SlewI64::new(0),
            Err(crate::ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn i128_basic() {
        let mut s = SlewI128::new(5).unwrap();
        assert_eq!(s.update(100), 100);
        assert_eq!(s.update(200), 105);
    }

    #[test]
    fn rejects_nan_and_inf() {
        let mut s = SlewF64::new(10.0).unwrap();
        assert!(matches!(
            s.update(f64::NAN),
            Err(crate::DataError::NotANumber)
        ));
        assert!(matches!(
            s.update(f64::INFINITY),
            Err(crate::DataError::Infinite)
        ));
        assert!(matches!(
            s.update(f64::NEG_INFINITY),
            Err(crate::DataError::Infinite)
        ));
    }
}
