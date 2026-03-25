macro_rules! impl_debounce {
    ($name:ident, $ty:ty) => {
        /// Debounce filter — requires N consecutive active signals to trigger.
        ///
        /// Prevents spurious single-sample activations from triggering.
        /// Resets the consecutive counter on any inactive sample.
        ///
        /// # Use Cases
        /// - Confirming a condition persists before acting
        /// - Filtering noisy boolean signals
        /// - "Only alert if elevated for N consecutive checks"
        #[derive(Debug, Clone)]
        pub struct $name {
            threshold: $ty,
            consecutive: $ty,
        }

        impl $name {
            /// Creates a new debounce filter.
            ///
            /// `threshold` is the number of consecutive active samples required
            /// to trigger (return `true`).
            #[inline]
            pub fn new(threshold: $ty) -> Result<Self, crate::ConfigError> {
                if threshold == 0 {
                    return Err(crate::ConfigError::Invalid("threshold must be positive"));
                }
                Ok(Self {
                    threshold,
                    consecutive: 0,
                })
            }

            /// Feeds a boolean signal. Returns `true` once the threshold of
            /// consecutive active samples is reached.
            #[inline]
            #[must_use]
            pub fn update(&mut self, active: bool) -> bool {
                if active {
                    self.consecutive += 1;
                } else {
                    self.consecutive = 0;
                }
                self.consecutive >= self.threshold
            }

            /// Current consecutive count.
            #[inline]
            #[must_use]
            pub fn count(&self) -> $ty {
                self.consecutive
            }

            /// Resets the consecutive counter.
            #[inline]
            pub fn reset(&mut self) {
                self.consecutive = 0;
            }
        }
    };
}

impl_debounce!(DebounceU32, u32);
impl_debounce!(DebounceU64, u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_after_threshold() {
        let mut d = DebounceU32::new(3).unwrap();
        assert!(!d.update(true));
        assert!(!d.update(true));
        assert!(d.update(true)); // 3rd consecutive
    }

    #[test]
    fn resets_on_false() {
        let mut d = DebounceU32::new(3).unwrap();
        assert!(!d.update(true));
        assert!(!d.update(true));
        assert!(!d.update(false)); // reset
        assert!(!d.update(true)); // restart count
        assert!(!d.update(true));
        assert!(d.update(true)); // 3rd consecutive again
    }

    #[test]
    fn no_false_trigger_at_threshold_minus_one() {
        let mut d = DebounceU32::new(5).unwrap();
        for _ in 0..4 {
            assert!(!d.update(true));
        }
        // 4 is not enough
        assert!(!d.update(false)); // break
    }

    #[test]
    fn stays_triggered() {
        let mut d = DebounceU32::new(2).unwrap();
        assert!(!d.update(true));
        assert!(d.update(true));
        assert!(d.update(true)); // stays triggered
    }

    #[test]
    fn u64_basic() {
        let mut d = DebounceU64::new(2).unwrap();
        assert!(!d.update(true));
        assert!(d.update(true));
    }

    #[test]
    fn reset() {
        let mut d = DebounceU32::new(2).unwrap();
        let _ = d.update(true);
        d.reset();
        assert_eq!(d.count(), 0);
    }

    #[test]
    fn rejects_zero_threshold() {
        assert!(matches!(
            DebounceU32::new(0),
            Err(crate::ConfigError::Invalid(_))
        ));
        assert!(matches!(
            DebounceU64::new(0),
            Err(crate::ConfigError::Invalid(_))
        ));
    }
}
