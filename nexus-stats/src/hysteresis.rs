macro_rules! impl_hysteresis {
    ($name:ident, $ty:ty) => {
        /// Hysteresis filter — Schmitt trigger with separate high/low thresholds.
        ///
        /// Transitions to `true` when sample exceeds the high threshold.
        /// Transitions to `false` when sample drops below the low threshold.
        /// Between the thresholds, the state is unchanged — preventing oscillation.
        ///
        /// # Use Cases
        /// - Thermostat logic (turn on at low, turn off at high)
        /// - Alert suppression (don't flap at boundary)
        /// - Binary state from a noisy analog signal
        #[derive(Debug, Clone)]
        pub struct $name {
            low: $ty,
            high: $ty,
            state: bool,
        }

        impl $name {
            /// Creates a new hysteresis filter.
            ///
            /// `low_threshold` must be less than `high_threshold`.
            ///
            /// # Panics
            ///
            /// Panics if `low >= high`.
            #[inline]
            #[must_use]
            pub fn new(low_threshold: $ty, high_threshold: $ty) -> Self {
                assert!(low_threshold < high_threshold, "low threshold must be less than high");
                Self { low: low_threshold, high: high_threshold, state: false }
            }

            /// Feeds a sample. Returns the current state.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> bool {
                if sample >= self.high {
                    self.state = true;
                } else if sample <= self.low {
                    self.state = false;
                }
                self.state
            }

            /// Current state.
            #[inline]
            #[must_use]
            pub fn state(&self) -> bool { self.state }

            /// Resets state to false.
            #[inline]
            pub fn reset(&mut self) { self.state = false; }
        }
    };
}

impl_hysteresis!(HysteresisF64, f64);
impl_hysteresis!(HysteresisF32, f32);
impl_hysteresis!(HysteresisI64, i64);
impl_hysteresis!(HysteresisI32, i32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rising_crosses_high() {
        let mut h = HysteresisF64::new(30.0, 70.0);
        assert!(!h.update(50.0)); // between thresholds, starts false
        assert!(h.update(80.0));  // crosses high
    }

    #[test]
    fn falling_crosses_low() {
        let mut h = HysteresisF64::new(30.0, 70.0);
        let _ = h.update(80.0); // true
        assert!(h.update(50.0)); // between, stays true
        assert!(!h.update(20.0)); // crosses low
    }

    #[test]
    fn no_oscillation_at_boundary() {
        let mut h = HysteresisF64::new(30.0, 70.0);
        let _ = h.update(80.0); // true

        // Oscillate between thresholds — state should not change
        for _ in 0..10 {
            assert!(h.update(50.0));
            assert!(h.update(60.0));
            assert!(h.update(40.0));
        }
    }

    #[test]
    fn i64_basic() {
        let mut h = HysteresisI64::new(30, 70);
        assert!(!h.update(50));
        assert!(h.update(75));
        assert!(h.update(50)); // between, stays true
        assert!(!h.update(25));
    }

    #[test]
    fn reset() {
        let mut h = HysteresisF64::new(30.0, 70.0);
        let _ = h.update(80.0);
        h.reset();
        assert!(!h.state());
    }

    #[test]
    #[should_panic(expected = "low threshold must be less than high")]
    fn panics_on_invalid_thresholds() {
        let _ = HysteresisF64::new(70.0, 30.0);
    }
}
