/// A detected peak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peak<T> {
    /// The peak value.
    pub value: T,
    /// Whether this is a local maximum (true) or local minimum (false).
    pub is_maximum: bool,
}

macro_rules! impl_peak_detector {
    ($name:ident, $ty:ty, $zero:expr) => {
        /// Peak detector — identifies local maxima and minima with prominence filtering.
        ///
        /// A peak is reported when the signal reverses direction by more than
        /// the prominence threshold. This filters out small oscillations.
        ///
        /// # Use Cases
        /// - Finding local highs/lows in price data
        /// - Cycle detection in oscillating signals
        /// - Inflection point identification
        #[derive(Debug, Clone)]
        pub struct $name {
            prominence: $ty,
            extreme: $ty,
            rising: bool,
            count: u64,
        }

        impl $name {
            /// Creates a new peak detector with the given prominence threshold.
            ///
            /// A reversal must exceed `prominence` to qualify as a peak.
            #[inline]
            pub fn new(prominence: $ty) -> Result<Self, crate::ConfigError> {
                #[allow(clippy::neg_cmp_op_on_partial_ord)]
                if !(prominence >= $zero) {
                    return Err(crate::ConfigError::Invalid("prominence must be non-negative"));
                }
                Ok(Self {
                    prominence,
                    extreme: $zero,
                    rising: true,
                    count: 0,
                })
            }

            /// Feeds a sample. Returns `Some(Peak)` when a peak is detected.
            #[inline]
            #[must_use]
            pub fn update(&mut self, sample: $ty) -> Option<Peak<$ty>> {
                self.count += 1;

                if self.count == 1 {
                    self.extreme = sample;
                    return Option::None;
                }

                if self.rising {
                    if sample > self.extreme {
                        self.extreme = sample;
                        Option::None
                    } else if self.extreme - sample >= self.prominence {
                        let peak = Peak { value: self.extreme, is_maximum: true };
                        self.extreme = sample;
                        self.rising = false;
                        Option::Some(peak)
                    } else {
                        Option::None
                    }
                } else if sample < self.extreme {
                    self.extreme = sample;
                    Option::None
                } else if sample - self.extreme >= self.prominence {
                    let peak = Peak { value: self.extreme, is_maximum: false };
                    self.extreme = sample;
                    self.rising = true;
                    Option::Some(peak)
                } else {
                    Option::None
                }
            }

            /// Resets the detector.
            #[inline]
            pub fn reset(&mut self) {
                self.extreme = $zero;
                self.rising = true;
                self.count = 0;
            }
        }
    };
}

impl_peak_detector!(PeakDetectorF64, f64, 0.0);
impl_peak_detector!(PeakDetectorF32, f32, 0.0);
impl_peak_detector!(PeakDetectorI64, i64, 0);
impl_peak_detector!(PeakDetectorI32, i32, 0);
impl_peak_detector!(PeakDetectorI128, i128, 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_maximum() {
        let mut pd = PeakDetectorF64::new(5.0).unwrap();
        let _ = pd.update(10.0);
        let _ = pd.update(20.0);
        let _ = pd.update(30.0); // rising
        let peak = pd.update(20.0); // dropped by 10 > prominence 5
        assert_eq!(peak, Some(Peak { value: 30.0, is_maximum: true }));
    }

    #[test]
    fn detects_minimum() {
        let mut pd = PeakDetectorF64::new(5.0).unwrap();
        let _ = pd.update(30.0);
        let _ = pd.update(20.0);
        let _ = pd.update(10.0); // found max at 30, now falling
        // need to trigger the max detection first
        let _ = pd.update(20.0); // reversal from 10 by 10 > 5, minimum at 10

        let mut pd2 = PeakDetectorF64::new(5.0).unwrap();
        let _ = pd2.update(10.0);
        let _ = pd2.update(20.0); // rising
        let _ = pd2.update(10.0); // max at 20, reversal
        let _ = pd2.update(5.0);  // falling
        let peak = pd2.update(15.0); // reversal from 5 by 10 > 5, minimum at 5
        assert_eq!(peak, Some(Peak { value: 5.0, is_maximum: false }));
    }

    #[test]
    fn small_oscillation_filtered() {
        let mut pd = PeakDetectorF64::new(10.0).unwrap();
        let _ = pd.update(100.0);
        let _ = pd.update(105.0);
        assert!(pd.update(102.0).is_none()); // only dropped 3, < prominence 10
    }

    #[test]
    fn i64_basic() {
        let mut pd = PeakDetectorI64::new(10).unwrap();
        let _ = pd.update(0);
        let _ = pd.update(50);
        let peak = pd.update(30); // dropped 20 > 10
        assert_eq!(peak, Some(Peak { value: 50, is_maximum: true }));
    }

    #[test]
    fn reset() {
        let mut pd = PeakDetectorF64::new(5.0).unwrap();
        let _ = pd.update(100.0);
        pd.reset();
        assert!(pd.update(50.0).is_none()); // re-initialized
    }

    #[test]
    fn rejects_negative_prominence() {
        assert!(matches!(PeakDetectorF64::new(-1.0), Err(crate::ConfigError::Invalid(_))));
    }

    #[test]
    fn i128_basic() {
        let mut pd = PeakDetectorI128::new(10).unwrap();
        let _ = pd.update(0);
        let _ = pd.update(50);
        let peak = pd.update(30); // dropped 20 > 10
        assert_eq!(peak, Some(Peak { value: 50, is_maximum: true }));
    }
}
