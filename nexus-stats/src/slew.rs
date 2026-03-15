macro_rules! impl_slew {
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
            #[must_use]
            pub fn new(max_rate: $ty) -> Self {
                Self { max_rate, value: $zero, initialized: false }
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

                let delta = sample - self.value;
                if delta > self.max_rate {
                    self.value += self.max_rate;
                } else if delta < -self.max_rate {
                    self.value -= self.max_rate;
                } else {
                    self.value = sample;
                }
                self.value
            }

            /// Current output value.
            #[inline]
            #[must_use]
            pub fn value(&self) -> $ty { self.value }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.value = $zero;
                self.initialized = false;
            }
        }
    };
}

impl_slew!(SlewF64, f64, 0.0);
impl_slew!(SlewF32, f32, 0.0);
impl_slew!(SlewI64, i64, 0);
impl_slew!(SlewI32, i32, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)]
    fn spike_clamped() {
        let mut s = SlewF64::new(10.0);
        assert_eq!(s.update(100.0), 100.0); // first sample, pass through
        assert_eq!(s.update(200.0), 110.0); // clamped: 100 + 10
        assert_eq!(s.update(200.0), 120.0); // clamped: 110 + 10
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn gradual_passes_through() {
        let mut s = SlewF64::new(10.0);
        assert_eq!(s.update(100.0), 100.0);
        assert_eq!(s.update(105.0), 105.0); // within rate, passes through
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn negative_clamped() {
        let mut s = SlewF64::new(10.0);
        assert_eq!(s.update(100.0), 100.0);
        assert_eq!(s.update(50.0), 90.0); // clamped: 100 - 10
    }

    #[test]
    fn i64_basic() {
        let mut s = SlewI64::new(5);
        assert_eq!(s.update(100), 100);
        assert_eq!(s.update(200), 105);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn reset() {
        let mut s = SlewF64::new(10.0);
        let _ = s.update(100.0);
        s.reset();
        assert_eq!(s.update(50.0), 50.0); // re-initialized
    }
}
