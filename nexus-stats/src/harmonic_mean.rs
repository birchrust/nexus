macro_rules! impl_harmonic_mean {
    ($name:ident, $ty:ty) => {
        /// Online harmonic mean.
        ///
        /// Tracks the sum of reciprocals for computing the harmonic mean
        /// incrementally. Useful for averaging rates (e.g., throughput in
        /// requests/second across multiple servers).
        ///
        /// # Use Cases
        /// - Average throughput across heterogeneous servers
        /// - Mean speed over equal-distance segments
        /// - Any rate where arithmetic mean would be misleading
        #[derive(Debug, Clone)]
        pub struct $name {
            reciprocal_sum: $ty,
            count: u64,
        }

        impl $name {
            /// Creates a new empty accumulator.
            #[inline]
            #[must_use]
            pub const fn new() -> Self {
                Self {
                    reciprocal_sum: 0.0 as $ty,
                    count: 0,
                }
            }

            /// Feeds a sample. Must be positive and non-zero.
            ///
            /// # Panics
            ///
            /// Panics if sample is zero or negative.
            #[inline]
            pub fn update(&mut self, sample: $ty) {
                assert!(
                    sample > 0.0 as $ty,
                    "harmonic mean requires positive values"
                );
                self.count += 1;
                self.reciprocal_sum += 1.0 as $ty / sample;
            }

            /// Harmonic mean, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn mean(&self) -> Option<$ty> {
                if self.count == 0 {
                    Option::None
                } else {
                    Option::Some(self.count as $ty / self.reciprocal_sum)
                }
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.reciprocal_sum = 0.0 as $ty;
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

impl_harmonic_mean!(HarmonicMeanF64, f64);
impl_harmonic_mean!(HarmonicMeanF32, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let hm = HarmonicMeanF64::new();
        assert!(hm.mean().is_none());
    }

    #[test]
    fn known_values() {
        let mut hm = HarmonicMeanF64::new();
        // HM(1, 4) = 2 / (1/1 + 1/4) = 2 / 1.25 = 1.6
        hm.update(1.0);
        hm.update(4.0);
        let m = hm.mean().unwrap();
        assert!((m - 1.6).abs() < 1e-10, "expected 1.6, got {m}");
    }

    #[test]
    fn harmonic_leq_arithmetic() {
        let mut hm = HarmonicMeanF64::new();
        let vals = [2.0, 4.0, 8.0];
        let mut sum = 0.0;
        for &v in &vals {
            hm.update(v);
            sum += v;
        }
        let arithmetic = sum / vals.len() as f64;
        let harmonic = hm.mean().unwrap();
        assert!(
            harmonic <= arithmetic,
            "HM ({harmonic}) should be <= AM ({arithmetic})"
        );
    }

    #[test]
    fn equal_values() {
        let mut hm = HarmonicMeanF64::new();
        for _ in 0..100 {
            hm.update(5.0);
        }
        let m = hm.mean().unwrap();
        assert!(
            (m - 5.0).abs() < 1e-10,
            "HM of equal values should equal that value"
        );
    }

    #[test]
    fn reset() {
        let mut hm = HarmonicMeanF64::new();
        hm.update(1.0);
        hm.reset();
        assert_eq!(hm.count(), 0);
        assert!(hm.mean().is_none());
    }

    #[test]
    fn f32_basic() {
        let mut hm = HarmonicMeanF32::new();
        hm.update(2.0);
        hm.update(4.0);
        assert!(hm.mean().is_some());
    }

    #[test]
    fn default_is_empty() {
        let hm = HarmonicMeanF64::default();
        assert_eq!(hm.count(), 0);
    }

    #[test]
    #[should_panic(expected = "positive values")]
    fn panics_on_zero() {
        let mut hm = HarmonicMeanF64::new();
        hm.update(0.0);
    }
}
