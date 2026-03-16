use crate::Condition;
use crate::windowed::{WindowedMinI32, WindowedMinI64};

macro_rules! impl_queue_delay {
    ($name:ident, $builder:ident, $ty:ty, $windowed_min:ty) => {
        /// CoDel-inspired sojourn time monitor.
        ///
        /// Composes a windowed minimum of sojourn times with a threshold.
        /// Reports `Elevated` when even the minimum sojourn time in the
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
            window: Option<u64>,
            min_samples: u64,
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
                }
            }

            /// Feeds a sojourn time at the given timestamp.
            ///
            /// Returns `Some(Condition)` once primed, `None` before.
            #[inline]
            #[must_use]
            pub fn update(&mut self, timestamp: u64, sojourn: $ty) -> Option<Condition> {
                let min = self.windowed_min.update(timestamp, sojourn);

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
            pub fn window(mut self, window: u64) -> Self {
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

            /// Builds the queue delay monitor.
            ///
            /// # Errors
            ///
            /// - Target must have been set.
            /// - Window must have been set and be positive.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let target = self.target.ok_or(crate::ConfigError::Missing("target"))?;
                let window = self.window.ok_or(crate::ConfigError::Missing("window"))?;
                if window == 0 {
                    return Err(crate::ConfigError::Invalid("QueueDelay window must be positive"));
                }

                Ok($name {
                    windowed_min: <$windowed_min>::new(window),
                    target,
                    min_samples: self.min_samples,
                })
            }
        }
    };
}

impl_queue_delay!(QueueDelayI64, QueueDelayI64Builder, i64, WindowedMinI64);
impl_queue_delay!(QueueDelayI32, QueueDelayI32Builder, i32, WindowedMinI32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_queue() {
        let mut qd = QueueDelayI64::builder()
            .target(100)
            .window(1000)
            .build().unwrap();

        // All sojourn times below target
        for t in 0..100 {
            let result = qd.update(t * 10, 50);
            assert_eq!(result, Some(Condition::Normal));
        }
        assert!(!qd.is_elevated());
    }

    #[test]
    fn elevated_detection() {
        let mut qd = QueueDelayI64::builder()
            .target(100)
            .window(1000)
            .build().unwrap();

        // Sojourn times above target
        for t in 0..100 {
            let _ = qd.update(t * 10, 200);
        }
        assert!(qd.is_elevated());
    }

    #[test]
    fn recovery_after_drain() {
        let mut qd = QueueDelayI64::builder()
            .target(100)
            .window(10)
            .build().unwrap();

        // Build up
        for t in 0..10 {
            let _ = qd.update(t, 200);
        }
        assert!(qd.is_elevated());

        // Drain — low sojourn times should eventually make min drop below target
        for t in 10..30 {
            let _ = qd.update(t, 10);
        }
        assert!(!qd.is_elevated(), "should recover after drain");
    }

    #[test]
    fn burst_vs_standing_queue() {
        let mut qd = QueueDelayI64::builder()
            .target(100)
            .window(10)
            .build().unwrap();

        // Single burst sample among low values — min stays low, not elevated
        for t in 0..10 {
            let _ = qd.update(t, 10);
        }
        let _ = qd.update(10, 500); // single burst
        assert!(!qd.is_elevated(), "single burst should not trigger — min is still low");
    }

    #[test]
    fn priming() {
        let mut qd = QueueDelayI64::builder()
            .target(100)
            .window(1000)
            .min_samples(5)
            .build().unwrap();

        for t in 0..4 {
            assert_eq!(qd.update(t, 200), None);
        }
        assert!(qd.update(4, 200).is_some());
    }

    #[test]
    fn reset_clears() {
        let mut qd = QueueDelayI64::builder()
            .target(100)
            .window(1000)
            .build().unwrap();

        for t in 0..10 {
            let _ = qd.update(t, 200);
        }
        qd.reset();
        assert_eq!(qd.count(), 0);
        assert!(qd.min_sojourn().is_none());
    }

    #[test]
    fn i32_basic() {
        let mut qd = QueueDelayI32::builder()
            .target(50)
            .window(100)
            .build().unwrap();

        let result = qd.update(0, 30);
        assert_eq!(result, Some(Condition::Normal));
    }

    #[test]
    fn errors_without_target() {
        let result = QueueDelayI64::builder().window(100).build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("target"))));
    }

    #[test]
    fn errors_without_window() {
        let result = QueueDelayI64::builder().target(100).build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("window"))));
    }
}
