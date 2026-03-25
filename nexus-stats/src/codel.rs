use std::time::{Duration, Instant};

use crate::Condition;
use crate::windowed::{
    WindowedMinF32, WindowedMinF64, WindowedMinI32, WindowedMinI64, WindowedMinI128,
};

macro_rules! impl_codel {
    ($name:ident, $builder:ident, $ty:ty, $windowed_min:ty) => {
        /// CoDel — Controlled Delay queue monitor (Nichols & Jacobson, 2012).
        ///
        /// Composes a windowed minimum of sojourn times with a threshold.
        /// Reports `Degraded` when even the minimum sojourn time in the
        /// observation window exceeds the target — indicating a standing
        /// queue rather than a transient burst.
        ///
        /// # Use Cases
        /// - Internal queue health monitoring
        /// - Backpressure detection
        /// - "Is this queue draining or building up?"
        #[derive(Debug, Clone)]
        pub struct $name {
            windowed_min: $windowed_min,
            target: $ty,
            min_samples: u64,
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            target: Option<$ty>,
            window: Option<Duration>,
            min_samples: u64,
            base: Option<Instant>,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    target: Option::None,
                    window: Option::None,
                    min_samples: 1,
                    base: Option::None,
                }
            }

            /// Feeds a sojourn time at the given timestamp.
            ///
            /// Returns `Some(Condition)` once primed, `None` before.
            #[inline]
            #[must_use]
            pub fn update(&mut self, now: Instant, sojourn: $ty) -> Option<Condition> {
                let min = self.windowed_min.update(now, sojourn);

                if self.windowed_min.count() < self.min_samples {
                    return Option::None;
                }

                if min > self.target {
                    Option::Some(Condition::Degraded)
                } else {
                    Option::Some(Condition::Normal)
                }
            }

            /// Current windowed minimum sojourn time, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn min_sojourn(&self) -> Option<$ty> {
                self.windowed_min.min()
            }

            /// Whether the queue is currently elevated.
            #[inline]
            #[must_use]
            pub fn is_elevated(&self) -> bool {
                if let Some(min) = self.windowed_min.min() {
                    min > self.target
                } else {
                    false
                }
            }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.windowed_min.count()
            }

            /// Whether the monitor has reached `min_samples`.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.windowed_min.count() >= self.min_samples
            }

            /// Resets to empty state. Parameters unchanged.
            #[inline]
            pub fn reset(&mut self) {
                self.windowed_min.reset();
            }
        }

        impl $builder {
            /// Target sojourn time. Elevated when minimum exceeds this.
            #[inline]
            #[must_use]
            pub fn target(mut self, target: $ty) -> Self {
                self.target = Option::Some(target);
                self
            }

            /// Observation window size (in timestamp units).
            #[inline]
            #[must_use]
            pub fn window(mut self, window: Duration) -> Self {
                self.window = Option::Some(window);
                self
            }

            /// Minimum samples before monitoring activates. Default: 1.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = min;
                self
            }

            /// Base instant for timestamp conversion. Default: `Instant::now()`.
            #[inline]
            #[must_use]
            pub fn base(mut self, base: Instant) -> Self {
                self.base = Option::Some(base);
                self
            }

            /// Builds the CoDel monitor.
            ///
            /// # Errors
            ///
            /// - Target must have been set.
            /// - Window must have been set and be positive.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let target = self.target.ok_or(crate::ConfigError::Missing("target"))?;
                let window = self.window.ok_or(crate::ConfigError::Missing("window"))?;
                if window.is_zero() {
                    return Err(crate::ConfigError::Invalid("CoDel window must be positive"));
                }

                let base = self.base.unwrap_or_else(Instant::now);
                Ok($name {
                    windowed_min: <$windowed_min>::with_base(window, base)?,
                    target,
                    min_samples: self.min_samples,
                })
            }
        }
    };
}

impl_codel!(CoDelI64, CoDelI64Builder, i64, WindowedMinI64);
impl_codel!(CoDelI32, CoDelI32Builder, i32, WindowedMinI32);
impl_codel!(CoDelI128, CoDelI128Builder, i128, WindowedMinI128);
impl_codel!(CoDelF64, CoDelF64Builder, f64, WindowedMinF64);
impl_codel!(CoDelF32, CoDelF32Builder, f32, WindowedMinF32);

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn t(base: Instant, ns: u64) -> Instant {
        base + Duration::from_nanos(ns)
    }

    #[test]
    fn healthy_queue() {
        let base = Instant::now();
        let mut qd = CoDelI64::builder()
            .target(100)
            .window(Duration::from_nanos(1000))
            .base(base)
            .build()
            .unwrap();

        // All sojourn times below target
        for ts in 0..100 {
            let result = qd.update(t(base, ts * 10), 50);
            assert_eq!(result, Some(Condition::Normal));
        }
        assert!(!qd.is_elevated());
    }

    #[test]
    fn elevated_detection() {
        let base = Instant::now();
        let mut qd = CoDelI64::builder()
            .target(100)
            .window(Duration::from_nanos(1000))
            .base(base)
            .build()
            .unwrap();

        // Sojourn times above target
        for ts in 0..100 {
            let _ = qd.update(t(base, ts * 10), 200);
        }
        assert!(qd.is_elevated());
    }

    #[test]
    fn recovery_after_drain() {
        let base = Instant::now();
        let mut qd = CoDelI64::builder()
            .target(100)
            .window(Duration::from_nanos(10))
            .base(base)
            .build()
            .unwrap();

        // Build up
        for ts in 0..10 {
            let _ = qd.update(t(base, ts), 200);
        }
        assert!(qd.is_elevated());

        // Drain — low sojourn times should eventually make min drop below target
        for ts in 10..30 {
            let _ = qd.update(t(base, ts), 10);
        }
        assert!(!qd.is_elevated(), "should recover after drain");
    }

    #[test]
    fn burst_vs_standing_queue() {
        let base = Instant::now();
        let mut qd = CoDelI64::builder()
            .target(100)
            .window(Duration::from_nanos(10))
            .base(base)
            .build()
            .unwrap();

        // Single burst sample among low values — min stays low, not elevated
        for ts in 0..10 {
            let _ = qd.update(t(base, ts), 10);
        }
        let _ = qd.update(t(base, 10), 500); // single burst
        assert!(
            !qd.is_elevated(),
            "single burst should not trigger — min is still low"
        );
    }

    #[test]
    fn priming() {
        let base = Instant::now();
        let mut qd = CoDelI64::builder()
            .target(100)
            .window(Duration::from_nanos(1000))
            .min_samples(5)
            .base(base)
            .build()
            .unwrap();

        for ts in 0..4 {
            assert_eq!(qd.update(t(base, ts), 200), None);
        }
        assert!(qd.update(t(base, 4), 200).is_some());
    }

    #[test]
    fn reset_clears() {
        let base = Instant::now();
        let mut qd = CoDelI64::builder()
            .target(100)
            .window(Duration::from_nanos(1000))
            .base(base)
            .build()
            .unwrap();

        for ts in 0..10 {
            let _ = qd.update(t(base, ts), 200);
        }
        qd.reset();
        assert_eq!(qd.count(), 0);
        assert!(qd.min_sojourn().is_none());
    }

    #[test]
    fn i32_basic() {
        let base = Instant::now();
        let mut qd = CoDelI32::builder()
            .target(50)
            .window(Duration::from_nanos(100))
            .base(base)
            .build()
            .unwrap();

        let result = qd.update(t(base, 0), 30);
        assert_eq!(result, Some(Condition::Normal));
    }

    #[test]
    fn errors_without_target() {
        let base = Instant::now();
        let result = CoDelI64::builder()
            .window(Duration::from_nanos(100))
            .base(base)
            .build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("target"))));
    }

    #[test]
    fn errors_without_window() {
        let base = Instant::now();
        let result = CoDelI64::builder().target(100).base(base).build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("window"))));
    }

    #[test]
    fn i128_basic() {
        let base = Instant::now();
        let mut qd = CoDelI128::builder()
            .target(50)
            .window(Duration::from_nanos(100))
            .base(base)
            .build()
            .unwrap();

        let result = qd.update(t(base, 0), 30);
        assert_eq!(result, Some(Condition::Normal));
    }
}
