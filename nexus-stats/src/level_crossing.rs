macro_rules! impl_level_crossing_float {
    ($name:ident, $ty:ty) => {
        /// Level crossing detector — signals when a value crosses a threshold.
        ///
        /// Returns `true` on the sample where the signal crosses the threshold
        /// in either direction. Tracks total crossing count.
        ///
        /// # Use Cases
        /// - Zero-crossing detection
        /// - Alert when metric crosses a boundary
        /// - Counting oscillation frequency
        #[derive(Debug, Clone)]
        pub struct $name {
            threshold: $ty,
            was_above: bool,
            crossings: u64,
            initialized: bool,
        }

        impl $name {
            /// Creates a new level crossing detector at the given threshold.
            #[inline]
            #[must_use]
            pub fn new(threshold: $ty) -> Self {
                Self {
                    threshold,
                    was_above: false,
                    crossings: 0,
                    initialized: false,
                }
            }

            /// Feeds a sample. Returns `Ok(true)` if a crossing occurred.
            ///
            /// # Errors
            ///
            /// Returns `DataError::NotANumber` if the sample is NaN, or
            /// `DataError::Infinite` if the sample is infinite.
            #[inline]
            pub fn update(&mut self, sample: $ty) -> Result<bool, crate::DataError> {
                check_finite!(sample);
                let is_above = sample >= self.threshold;

                if !self.initialized {
                    self.was_above = is_above;
                    self.initialized = true;
                    return Ok(false);
                }

                if is_above != self.was_above {
                    self.was_above = is_above;
                    self.crossings += 1;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }

            /// Total number of crossings detected.
            #[inline]
            #[must_use]
            pub fn crossing_count(&self) -> u64 {
                self.crossings
            }

            /// Resets the detector.
            #[inline]
            pub fn reset(&mut self) {
                self.was_above = false;
                self.crossings = 0;
                self.initialized = false;
            }
        }
    };
}

macro_rules! impl_level_crossing_int {
    ($name:ident, $ty:ty) => {
        /// Level crossing detector — signals when a value crosses a threshold.
        #[derive(Debug, Clone)]
        pub struct $name {
            threshold: $ty,
            was_above: bool,
            crossings: u64,
            initialized: bool,
        }

        impl $name {
            /// Creates a new level crossing detector at the given threshold.
            #[inline]
            #[must_use]
            pub fn new(threshold: $ty) -> Self {
                Self {
                    threshold,
                    was_above: false,
                    crossings: 0,
                    initialized: false,
                }
            }

            /// Feeds a sample. Returns `true` if a crossing occurred.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> bool {
                let is_above = sample >= self.threshold;

                if !self.initialized {
                    self.was_above = is_above;
                    self.initialized = true;
                    return false;
                }

                if is_above != self.was_above {
                    self.was_above = is_above;
                    self.crossings += 1;
                    true
                } else {
                    false
                }
            }

            /// Total number of crossings detected.
            #[inline]
            #[must_use]
            pub fn crossing_count(&self) -> u64 {
                self.crossings
            }

            /// Resets the detector.
            #[inline]
            pub fn reset(&mut self) {
                self.was_above = false;
                self.crossings = 0;
                self.initialized = false;
            }
        }
    };
}

impl_level_crossing_float!(LevelCrossingF64, f64);
impl_level_crossing_float!(LevelCrossingF32, f32);
impl_level_crossing_int!(LevelCrossingI64, i64);
impl_level_crossing_int!(LevelCrossingI32, i32);
impl_level_crossing_int!(LevelCrossingI128, i128);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_no_crossing() {
        let mut lc = LevelCrossingF64::new(50.0);
        assert!(!lc.update(30.0).unwrap());
    }

    #[test]
    fn upward_crossing() {
        let mut lc = LevelCrossingF64::new(50.0);
        assert!(!lc.update(30.0).unwrap());
        assert!(lc.update(60.0).unwrap()); // crossed upward
        assert_eq!(lc.crossing_count(), 1);
    }

    #[test]
    fn downward_crossing() {
        let mut lc = LevelCrossingF64::new(50.0);
        assert!(!lc.update(60.0).unwrap());
        assert!(lc.update(40.0).unwrap()); // crossed downward
        assert_eq!(lc.crossing_count(), 1);
    }

    #[test]
    fn multiple_crossings() {
        let mut lc = LevelCrossingF64::new(50.0);
        let _ = lc.update(30.0).unwrap();
        let _ = lc.update(60.0).unwrap(); // cross 1
        let _ = lc.update(40.0).unwrap(); // cross 2
        let _ = lc.update(70.0).unwrap(); // cross 3
        assert_eq!(lc.crossing_count(), 3);
    }

    #[test]
    fn no_crossing_same_side() {
        let mut lc = LevelCrossingF64::new(50.0);
        let _ = lc.update(30.0).unwrap();
        assert!(!lc.update(40.0).unwrap()); // same side
        assert!(!lc.update(20.0).unwrap()); // same side
    }

    #[test]
    fn i64_basic() {
        let mut lc = LevelCrossingI64::new(100);
        assert!(!lc.update(50));
        assert!(lc.update(150)); // crossed
    }

    #[test]
    fn reset() {
        let mut lc = LevelCrossingF64::new(50.0);
        let _ = lc.update(30.0).unwrap();
        let _ = lc.update(60.0).unwrap();
        lc.reset();
        assert_eq!(lc.crossing_count(), 0);
    }

    #[test]
    fn i128_basic() {
        let mut lc = LevelCrossingI128::new(100);
        assert!(!lc.update(50));
        assert!(lc.update(150));
    }

    #[test]
    fn rejects_nan_and_inf() {
        let mut lc = LevelCrossingF64::new(50.0);
        assert_eq!(lc.update(f64::NAN), Err(crate::DataError::NotANumber));
        assert_eq!(lc.update(f64::INFINITY), Err(crate::DataError::Infinite));
        assert_eq!(
            lc.update(f64::NEG_INFINITY),
            Err(crate::DataError::Infinite)
        );
    }
}
