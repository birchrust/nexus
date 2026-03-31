use nexus_stats_core::math::MulAdd;

macro_rules! impl_kama {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// KAMA — Kaufman Adaptive Moving Average.
        ///
        /// EMA with an efficiency-ratio-driven alpha. In trending markets,
        /// the alpha increases (fast response). In noisy/choppy markets,
        /// the alpha decreases (slow response).
        ///
        /// The efficiency ratio = |direction| / volatility, where:
        /// - direction = price_now - price_N_ago
        /// - volatility = sum of |price_i - price_{i-1}| over N periods
        ///
        /// The window size is specified at runtime via the builder. The ring
        /// buffer is heap-allocated once during `build()` — no allocation
        /// after construction.
        ///
        /// # Use Cases
        /// - Adaptive smoothing that auto-tunes to market conditions
        /// - Noise-resistant trend following
        /// - Signal processing with variable noise levels
        pub struct $name {
            ring: *mut $ty,
            window: usize,
            head: usize,
            value: $ty,
            fast_sc: $ty,
            slow_sc: $ty,
            volatility_sum: $ty,
            count: u64,
            min_samples: u64,
        }

        // SAFETY: buffer is exclusively owned, T is Copy + Send
        unsafe impl Send for $name {}

        impl $name {
            #[inline]
            fn ring(&self) -> &[$ty] {
                // SAFETY: buffer allocated with capacity `window`, all elements initialized
                unsafe { core::slice::from_raw_parts(self.ring, self.window) }
            }

            #[inline]
            fn ring_mut(&mut self) -> &mut [$ty] {
                // SAFETY: buffer exclusively owned, all elements initialized
                unsafe { core::slice::from_raw_parts_mut(self.ring, self.window) }
            }
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            window: Option<usize>,
            fast_span: u64,
            slow_span: u64,
            min_samples: Option<u64>,
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    window: Option::None,
                    fast_span: 2,
                    slow_span: 30,
                    min_samples: Option::None,
                }
            }

            /// Feeds a sample. Returns the adaptive smoothed value once primed.
            ///
            /// # Errors
            ///
            /// Returns `DataError::NotANumber` if the sample is NaN, or
            /// `DataError::Infinite` if the sample is infinite.
            #[inline]
            pub fn update(&mut self, sample: $ty) -> Result<Option<$ty>, nexus_stats_core::DataError> {
                check_finite!(sample);
                let n = self.window;
                let idx = (self.count as usize) % n;
                // SAFETY: idx is in [0, window), buffer exclusively owned
                unsafe { *self.ring.add(idx) = sample; }
                self.count += 1;

                if self.count == 1 {
                    self.value = sample;
                    return if self.count >= self.min_samples { Ok(Option::Some(self.value)) } else { Ok(Option::None) };
                }

                if self.count <= n as u64 {
                    self.value = sample;
                    return if self.count >= self.min_samples { Ok(Option::Some(self.value)) } else { Ok(Option::None) };
                }

                // Window is full — compute ER from the ring buffer
                // SAFETY: buffer allocated with capacity `window`, all initialized
                let ring = unsafe { core::slice::from_raw_parts(self.ring, n) };

                // The ring is ordered: oldest at (idx+1)%n, newest at idx.
                // Split into two contiguous slices to avoid modular indexing
                // per iteration, enabling SIMD vectorization.
                let oldest = (idx + 1) % n;

                // Compute volatility: sum of |consecutive differences| in ring order
                let mut volatility = 0.0 as $ty;

                // Slice 1: oldest..end of buffer
                let s1 = &ring[oldest..];
                for w in s1.windows(2) {
                    volatility += (w[1] - w[0]).abs();
                }

                // Bridge: last element of s1 to first element of s2
                if oldest > 0 && !s1.is_empty() {
                    volatility += (ring[0] - s1[s1.len() - 1]).abs();
                }

                // Slice 2: start..oldest (the wrap-around portion)
                let s2 = &ring[..oldest];
                for w in s2.windows(2) {
                    volatility += (w[1] - w[0]).abs();
                }

                // Direction: |newest - oldest|
                let direction = (sample - ring[oldest]).abs();
                self.volatility_sum = volatility;
                let er = if volatility > 0.0 as $ty {
                    direction / volatility
                } else {
                    0.0 as $ty
                };

                // Smoothing constant: sc = (er * (fast - slow) + slow)^2
                let sc = er * (self.fast_sc - self.slow_sc) + self.slow_sc;
                let alpha = sc * sc;

                self.value = alpha.fma(sample - self.value, self.value);

                if self.count >= self.min_samples {
                    Ok(Option::Some(self.value))
                } else {
                    Ok(Option::None)
                }
            }

            /// Current adaptive smoothed value, or `None` if not primed.
            #[inline]
            #[must_use]
            pub fn value(&self) -> Option<$ty> {
                if self.count >= self.min_samples { Option::Some(self.value) } else { Option::None }
            }

            /// Current efficiency ratio (0 to 1), or `None` if < window samples.
            #[inline]
            #[must_use]
            pub fn efficiency_ratio(&self) -> Option<$ty> {
                let n = self.window;
                if self.count <= n as u64 {
                    return Option::None;
                }
                let newest_idx = ((self.count - 1) as usize) % n;
                let oldest_idx = (self.count as usize) % n;
                let ring = self.ring();
                let direction = (ring[newest_idx] - ring[oldest_idx]).abs();
                if self.volatility_sum > 0.0 as $ty {
                    Option::Some(direction / self.volatility_sum)
                } else {
                    Option::Some(0.0 as $ty)
                }
            }

            /// Window size.
            #[inline]
            #[must_use]
            pub fn window_size(&self) -> usize { self.window }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 { self.count }

            /// Whether the KAMA has reached `min_samples`.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool { self.count >= self.min_samples }

            /// Resets to uninitialized state.
            #[inline]
            pub fn reset(&mut self) {
                self.ring_mut().fill(0.0 as $ty);
                self.head = 0;
                self.value = 0.0 as $ty;
                self.volatility_sum = 0.0 as $ty;
                self.count = 0;
            }
        }

        impl $builder {
            /// Window size (number of samples in the ring buffer).
            #[inline]
            #[must_use]
            pub fn window_size(mut self, n: usize) -> Self {
                self.window = Option::Some(n);
                self
            }

            /// Fast EMA span (most reactive). Default: 2.
            #[inline]
            #[must_use]
            pub fn fast_span(mut self, n: u64) -> Self {
                self.fast_span = n;
                self
            }

            /// Slow EMA span (least reactive). Default: 30.
            #[inline]
            #[must_use]
            pub fn slow_span(mut self, n: u64) -> Self {
                self.slow_span = n;
                self
            }

            /// Minimum samples before value is valid. Default: window size.
            #[inline]
            #[must_use]
            pub fn min_samples(mut self, min: u64) -> Self {
                self.min_samples = Option::Some(min);
                self
            }

            /// Builds the KAMA.
            ///
            /// # Errors
            ///
            /// - Window size must have been set and > 0.
            /// - `fast_span` must be >= 1.
            /// - `slow_span` must be > `fast_span`.
            #[inline]
            pub fn build(self) -> Result<$name, nexus_stats_core::ConfigError> {
                let window = self.window.ok_or(nexus_stats_core::ConfigError::Missing("window_size"))?;
                if window == 0 {
                    return Err(nexus_stats_core::ConfigError::Invalid("window_size must be > 0"));
                }
                if self.fast_span < 1 {
                    return Err(nexus_stats_core::ConfigError::Invalid("fast_span must be >= 1"));
                }
                if self.slow_span <= self.fast_span {
                    return Err(nexus_stats_core::ConfigError::Invalid("slow_span must be > fast_span"));
                }
                let min_samples = self.min_samples.unwrap_or(window as u64);

                let mut vec = core::mem::ManuallyDrop::new(alloc::vec![0.0 as $ty; window]);
                let ring = vec.as_mut_ptr();

                Ok($name {
                    ring,
                    window,
                    head: 0,
                    value: 0.0 as $ty,
                    fast_sc: 2.0 as $ty / (self.fast_span as $ty + 1.0 as $ty),
                    slow_sc: 2.0 as $ty / (self.slow_span as $ty + 1.0 as $ty),
                    volatility_sum: 0.0 as $ty,
                    count: 0,
                    min_samples,
                })
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY: buffer was allocated by Vec with capacity `window`.
                // T is Copy so no element drops needed. Reclaim the allocation.
                unsafe {
                    let _ = alloc::vec::Vec::from_raw_parts(self.ring, 0, self.window);
                }
            }
        }

        impl Clone for $name {
            fn clone(&self) -> Self {
                let mut vec = alloc::vec![0.0 as $ty; self.window];
                vec.copy_from_slice(self.ring());
                let mut cloned = core::mem::ManuallyDrop::new(vec);
                let ring = cloned.as_mut_ptr();
                Self {
                    ring,
                    window: self.window,
                    head: self.head,
                    value: self.value,
                    fast_sc: self.fast_sc,
                    slow_sc: self.slow_sc,
                    volatility_sum: self.volatility_sum,
                    count: self.count,
                    min_samples: self.min_samples,
                }
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("window", &self.window)
                    .field("count", &self.count)
                    .field("value", &self.value)
                    .finish()
            }
        }
    };
}

impl_kama!(KamaF64, KamaF64Builder, f64);
impl_kama!(KamaF32, KamaF32Builder, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trending_signal_fast_response() {
        let mut kama = KamaF64::builder().window_size(10).build().unwrap();

        // Linear trend — ER should be high, KAMA should track closely
        for i in 0..50 {
            kama.update(i as f64).unwrap();
        }

        let er = kama.efficiency_ratio().unwrap();
        assert!(er > 0.5, "trending signal should have high ER, got {er}");
    }

    #[test]
    fn noisy_signal_slow_response() {
        let mut kama = KamaF64::builder().window_size(10).build().unwrap();

        // Oscillating — ER should be low
        for i in 0..50 {
            let v = if i % 2 == 0 { 100.0 } else { 0.0 };
            kama.update(v).unwrap();
        }

        let er = kama.efficiency_ratio().unwrap();
        assert!(er < 0.3, "noisy signal should have low ER, got {er}");
    }

    #[test]
    fn er_bounds() {
        let mut kama = KamaF64::builder().window_size(10).build().unwrap();
        for i in 0..20 {
            kama.update(i as f64).unwrap();
        }
        let er = kama.efficiency_ratio().unwrap();
        assert!(
            (0.0..=1.0).contains(&er),
            "ER should be in [0, 1], got {er}"
        );
    }

    #[test]
    fn priming() {
        let mut kama = KamaF64::builder().window_size(10).build().unwrap();
        for i in 0..9 {
            assert!(kama.update(i as f64).unwrap().is_none());
        }
        assert!(kama.update(9.0).unwrap().is_some());
    }

    #[test]
    fn reset() {
        let mut kama = KamaF64::builder().window_size(10).build().unwrap();
        for i in 0..20 {
            kama.update(i as f64).unwrap();
        }
        kama.reset();
        assert_eq!(kama.count(), 0);
    }

    #[test]
    fn f32_basic() {
        let mut kama = KamaF32::builder().window_size(5).build().unwrap();
        for i in 0..10 {
            kama.update(i as f32).unwrap();
        }
        assert!(kama.value().is_some());
    }

    #[test]
    fn window_size_accessor() {
        let kama = KamaF64::builder().window_size(10).build().unwrap();
        assert_eq!(kama.window_size(), 10);
    }

    #[test]
    fn rejects_nan_and_inf() {
        let mut kama = KamaF64::builder().window_size(10).build().unwrap();
        assert!(matches!(
            kama.update(f64::NAN),
            Err(nexus_stats_core::DataError::NotANumber)
        ));
        assert!(matches!(
            kama.update(f64::INFINITY),
            Err(nexus_stats_core::DataError::Infinite)
        ));
        assert!(matches!(
            kama.update(f64::NEG_INFINITY),
            Err(nexus_stats_core::DataError::Infinite)
        ));
        assert_eq!(kama.count(), 0);
    }
}
