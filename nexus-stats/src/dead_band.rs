macro_rules! impl_dead_band {
    ($name:ident, $ty:ty, $zero:expr) => {
        /// Dead band filter — suppresses small changes, reports significant ones.
        ///
        /// Only emits a new value when the sample deviates from the last reported
        /// value by more than the threshold. Prevents noisy oscillation around
        /// a stable value from generating unnecessary updates.
        ///
        /// # Use Cases
        /// - Reducing update frequency for slowly-changing metrics
        /// - Hysteresis-free change suppression
        /// - Sensor noise filtering
        #[derive(Debug, Clone)]
        pub struct $name {
            threshold: $ty,
            last_reported: $ty,
            initialized: bool,
        }

        impl $name {
            /// Creates a new dead band filter with the given threshold.
            #[inline]
            #[must_use]
            pub fn new(threshold: $ty) -> Self {
                Self { threshold, last_reported: $zero, initialized: false }
            }

            /// Feeds a sample. Returns `Some(value)` if the change exceeds
            /// the threshold, `None` if suppressed.
            ///
            /// The first sample is always reported.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<$ty> {
                if !self.initialized {
                    self.last_reported = sample;
                    self.initialized = true;
                    return Option::Some(sample);
                }

                let delta = sample - self.last_reported;
                let abs_delta = if delta < $zero { $zero - delta } else { delta };

                if abs_delta > self.threshold {
                    self.last_reported = sample;
                    Option::Some(sample)
                } else {
                    Option::None
                }
            }

            /// Last reported value, or `None` if no sample has been processed.
            #[inline]
            #[must_use]
            pub fn last_reported(&self) -> Option<$ty> {
                if self.initialized { Option::Some(self.last_reported) } else { Option::None }
            }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.last_reported = $zero;
                self.initialized = false;
            }
        }
    };
}

impl_dead_band!(DeadBandF64, f64, 0.0);
impl_dead_band!(DeadBandF32, f32, 0.0);
impl_dead_band!(DeadBandI64, i64, 0);
impl_dead_band!(DeadBandI32, i32, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)]
    fn first_sample_always_reported() {
        let mut db = DeadBandF64::new(5.0);
        assert_eq!(db.update(100.0), Some(100.0));
    }

    #[test]
    fn small_changes_suppressed() {
        let mut db = DeadBandF64::new(5.0);
        let _ = db.update(100.0);
        assert_eq!(db.update(103.0), None); // within threshold
        assert_eq!(db.update(99.0), None);  // within threshold
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn large_changes_reported() {
        let mut db = DeadBandF64::new(5.0);
        let _ = db.update(100.0);
        assert_eq!(db.update(110.0), Some(110.0)); // exceeds threshold
    }

    #[test]
    fn i64_basic() {
        let mut db = DeadBandI64::new(10);
        assert_eq!(db.update(100), Some(100));
        assert_eq!(db.update(105), None);
        assert_eq!(db.update(115), Some(115));
    }

    #[test]
    fn reset() {
        let mut db = DeadBandF64::new(5.0);
        let _ = db.update(100.0);
        db.reset();
        assert!(db.last_reported().is_none());
    }
}
