macro_rules! impl_max_gauge {
    ($name:ident, $ty:ty, $min_val:expr) => {
        /// Max gauge — tracks the maximum since last take.
        ///
        /// `take()` returns the maximum and resets the gauge. Useful for
        /// periodic reporting ("what was the peak since last report?").
        ///
        /// # Use Cases
        /// - Peak latency per reporting interval
        /// - High-water mark gauges (Prometheus-style)
        /// - Periodic max collection
        #[derive(Debug, Clone)]
        pub struct $name {
            max: $ty,
            has_value: bool,
        }

        impl $name {
            /// Creates a new empty gauge.
            #[inline]
            #[must_use]
            pub const fn new() -> Self {
                Self { max: $min_val, has_value: false }
            }

            /// Records a sample.
            #[inline]
            pub fn update(&mut self, sample: $ty) {
                if !self.has_value || sample > self.max {
                    self.max = sample;
                }
                self.has_value = true;
            }

            /// Returns the max since last take/reset, and resets the gauge.
            #[inline]
            pub fn take(&mut self) -> Option<$ty> {
                if self.has_value {
                    let val = self.max;
                    self.max = $min_val;
                    self.has_value = false;
                    Option::Some(val)
                } else {
                    Option::None
                }
            }

            /// Peeks at the current max without resetting.
            #[inline]
            #[must_use]
            pub fn peek(&self) -> Option<$ty> {
                if self.has_value { Option::Some(self.max) } else { Option::None }
            }

            /// Resets the gauge.
            #[inline]
            pub fn reset(&mut self) {
                self.max = $min_val;
                self.has_value = false;
            }
        }

        impl Default for $name {
            #[inline]
            fn default() -> Self { Self::new() }
        }
    };
}

impl_max_gauge!(MaxGaugeF64, f64, f64::MIN);
impl_max_gauge!(MaxGaugeF32, f32, f32::MIN);
impl_max_gauge!(MaxGaugeI64, i64, i64::MIN);
impl_max_gauge!(MaxGaugeI32, i32, i32::MIN);
impl_max_gauge!(MaxGaugeI128, i128, i128::MIN);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let mut g = MaxGaugeF64::new();
        assert!(g.peek().is_none());
        assert!(g.take().is_none());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn tracks_max() {
        let mut g = MaxGaugeF64::new();
        g.update(10.0);
        g.update(50.0);
        g.update(30.0);
        assert_eq!(g.peek(), Some(50.0));
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn take_returns_and_resets() {
        let mut g = MaxGaugeF64::new();
        g.update(50.0);
        assert_eq!(g.take(), Some(50.0));
        assert!(g.take().is_none()); // already taken

        g.update(20.0);
        assert_eq!(g.take(), Some(20.0));
    }

    #[test]
    fn i64_basic() {
        let mut g = MaxGaugeI64::new();
        g.update(100);
        g.update(200);
        assert_eq!(g.take(), Some(200));
    }

    #[test]
    fn reset() {
        let mut g = MaxGaugeF64::new();
        g.update(100.0);
        g.reset();
        assert!(g.peek().is_none());
    }

    #[test]
    fn default_is_empty() {
        let g = MaxGaugeI32::default();
        assert!(g.peek().is_none());
    }

    #[test]
    fn i128_basic() {
        let mut g = MaxGaugeI128::new();
        g.update(100);
        g.update(200);
        assert_eq!(g.take(), Some(200));
    }
}
