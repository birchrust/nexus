// Shannon Entropy — Online Categorical Distribution Entropy
//
// H(X) = -Σ p_i * ln(p_i)  where p_i = count_i / total
//
// Maintains frequency counts over K categories, computes entropy on query.
// O(K) for entropy query, O(1) for observe.

macro_rules! impl_entropy {
    ($name:ident, $ty:ty) => {
        /// Shannon entropy over a categorical distribution with `K` categories.
        ///
        /// Maintains frequency counts and computes entropy on query.
        /// Entropy measures how "spread out" or unpredictable a distribution
        /// is — higher entropy means more uncertainty.
        ///
        /// # Use Cases
        /// - "How predictable is the distribution of order types?"
        /// - "Is the venue distribution concentrating or diversifying?"
        /// - Monitoring regime change via entropy shifts
        ///
        /// # Complexity
        /// - O(1) per observation, O(K) per entropy query.
        /// - `8*K + 8` bytes state, zero allocation.
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("use nexus_stats::", stringify!($name), ";")]
        ///
        /// // Uniform distribution over 4 categories → maximum entropy
        #[doc = concat!("let mut e = ", stringify!($name), "::<4>::new();")]
        /// for i in 0..400u32 { e.observe(i as usize % 4); }
        /// let h = e.entropy().unwrap();
        /// // ln(4) ≈ 1.386
        /// assert!((h - 1.386).abs() < 0.01);
        /// ```
        #[derive(Debug, Clone)]
        pub struct $name<const K: usize> {
            counts: [u64; K],
            total: u64,
        }

        impl<const K: usize> $name<K> {
            const _ASSERT_K: () = assert!(K >= 2, "K must be at least 2");

            /// Creates a new empty entropy tracker.
            #[inline]
            #[must_use]
            pub fn new() -> Self {
                #[allow(clippy::let_unit_value)]
                let () = Self::_ASSERT_K;
                Self {
                    counts: [0; K],
                    total: 0,
                }
            }

            /// Records an observation in the given category.
            ///
            /// # Panics
            ///
            /// Panics if `category >= K`.
            #[inline]
            pub fn observe(&mut self, category: usize) {
                assert!(category < K, "category {category} out of range (K={K})");
                self.counts[category] += 1;
                self.total += 1;
            }

            /// Shannon entropy in nats (natural logarithm base), or `None` if empty.
            ///
            /// Maximum entropy for K categories is ln(K) (uniform distribution).
            /// Minimum is 0 (all observations in one category).
            #[inline]
            #[must_use]
            pub fn entropy(&self) -> Option<$ty> {
                if self.total == 0 {
                    return Option::None;
                }
                let n = self.total as $ty;
                let mut h = 0.0 as $ty;
                for &c in &self.counts {
                    if c > 0 {
                        let p = c as $ty / n;
                        #[allow(clippy::cast_possible_truncation)]
                        {
                            h -= p * crate::math::ln(p as f64) as $ty;
                        }
                    }
                }
                Option::Some(h)
            }

            /// Entropy in bits (log base 2), or `None` if empty.
            ///
            /// `entropy_bits = entropy / ln(2)`.
            #[inline]
            #[must_use]
            pub fn entropy_bits(&self) -> Option<$ty> {
                self.entropy().map(|h| {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        h / crate::math::ln(2.0) as $ty
                    }
                })
            }

            /// Self-information of the given category: `-ln(p_i)`.
            ///
            /// High values indicate rare/surprising events.
            /// Returns `None` if empty or the category has never been observed.
            ///
            /// # Panics
            ///
            /// Panics if `category >= K`.
            #[inline]
            #[must_use]
            pub fn surprise(&self, category: usize) -> Option<$ty> {
                assert!(category < K, "category {category} out of range (K={K})");
                if self.total == 0 || self.counts[category] == 0 {
                    return Option::None;
                }
                let p = self.counts[category] as $ty / self.total as $ty;
                #[allow(clippy::cast_possible_truncation)]
                {
                    Option::Some(-(crate::math::ln(p as f64) as $ty))
                }
            }

            /// Probability estimate for a category, or `None` if empty.
            ///
            /// # Panics
            ///
            /// Panics if `category >= K`.
            #[inline]
            #[must_use]
            pub fn probability(&self, category: usize) -> Option<$ty> {
                assert!(category < K, "category {category} out of range (K={K})");
                if self.total == 0 {
                    return Option::None;
                }
                Option::Some(self.counts[category] as $ty / self.total as $ty)
            }

            /// Total observations across all categories.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.total
            }

            /// Whether any observations have been recorded.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool {
                self.total > 0
            }

            /// Observation count for a specific category.
            ///
            /// # Panics
            ///
            /// Panics if `category >= K`.
            #[inline]
            #[must_use]
            pub fn category_count(&self, category: usize) -> u64 {
                assert!(category < K, "category {category} out of range (K={K})");
                self.counts[category]
            }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.counts = [0; K];
                self.total = 0;
            }
        }

        impl<const K: usize> Default for $name<K> {
            #[inline]
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

impl_entropy!(EntropyF64, f64);
impl_entropy!(EntropyF32, f32);

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Basic correctness
    // =========================================================================

    #[test]
    fn uniform_entropy_equals_ln_k() {
        let mut e = EntropyF64::<4>::new();
        for i in 0..4000u32 {
            e.observe(i as usize % 4);
        }
        let h = e.entropy().unwrap();
        let expected = (4.0_f64).ln();
        assert!(
            (h - expected).abs() < 1e-10,
            "uniform entropy should be ln(4)={expected}, got {h}"
        );
    }

    #[test]
    fn concentrated_entropy_zero() {
        let mut e = EntropyF64::<4>::new();
        for _ in 0..1000 {
            e.observe(0);
        }
        let h = e.entropy().unwrap();
        assert!(h.abs() < 1e-10, "concentrated entropy should be 0, got {h}");
    }

    #[test]
    fn binary_50_50() {
        let mut e = EntropyF64::<2>::new();
        for i in 0..2000u32 {
            e.observe(i as usize % 2);
        }
        let h = e.entropy().unwrap();
        let expected = (2.0_f64).ln();
        assert!(
            (h - expected).abs() < 1e-10,
            "50/50 binary entropy should be ln(2)={expected}, got {h}"
        );
    }

    #[test]
    fn entropy_bits_conversion() {
        let mut e = EntropyF64::<2>::new();
        for i in 0..2000u32 {
            e.observe(i as usize % 2);
        }
        let h_bits = e.entropy_bits().unwrap();
        // ln(2) / ln(2) = 1.0 bit
        assert!(
            (h_bits - 1.0).abs() < 1e-10,
            "50/50 binary entropy should be 1 bit, got {h_bits}"
        );
    }

    // =========================================================================
    // Surprise (self-information)
    // =========================================================================

    #[test]
    fn surprise_rare_vs_common() {
        let mut e = EntropyF64::<2>::new();
        for _ in 0..990 {
            e.observe(0); // common
        }
        for _ in 0..10 {
            e.observe(1); // rare
        }
        let s_common = e.surprise(0).unwrap();
        let s_rare = e.surprise(1).unwrap();
        assert!(
            s_rare > s_common,
            "rare should be more surprising: common={s_common}, rare={s_rare}"
        );
    }

    #[test]
    fn surprise_unobserved_returns_none() {
        let mut e = EntropyF64::<4>::new();
        e.observe(0);
        assert!(e.surprise(1).is_none());
    }

    // =========================================================================
    // Probability
    // =========================================================================

    #[test]
    fn probability_matches_counts() {
        let mut e = EntropyF64::<3>::new();
        for _ in 0..30 {
            e.observe(0);
        }
        for _ in 0..50 {
            e.observe(1);
        }
        for _ in 0..20 {
            e.observe(2);
        }
        assert!((e.probability(0).unwrap() - 0.3).abs() < 1e-10);
        assert!((e.probability(1).unwrap() - 0.5).abs() < 1e-10);
        assert!((e.probability(2).unwrap() - 0.2).abs() < 1e-10);
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn empty_returns_none() {
        let e = EntropyF64::<4>::new();
        assert!(e.entropy().is_none());
        assert!(e.entropy_bits().is_none());
        assert!(e.probability(0).is_none());
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn observe_out_of_range_panics() {
        let mut e = EntropyF64::<4>::new();
        e.observe(4);
    }

    // =========================================================================
    // Category count
    // =========================================================================

    #[test]
    fn category_count_tracks() {
        let mut e = EntropyF64::<3>::new();
        e.observe(0);
        e.observe(0);
        e.observe(1);
        assert_eq!(e.category_count(0), 2);
        assert_eq!(e.category_count(1), 1);
        assert_eq!(e.category_count(2), 0);
        assert_eq!(e.count(), 3);
    }

    // =========================================================================
    // Reset
    // =========================================================================

    #[test]
    fn reset_clears_state() {
        let mut e = EntropyF64::<4>::new();
        for i in 0..100 {
            e.observe(i % 4);
        }
        e.reset();
        assert_eq!(e.count(), 0);
        assert!(e.entropy().is_none());
    }

    // =========================================================================
    // f32 variant
    // =========================================================================

    #[test]
    fn f32_basic() {
        let mut e = EntropyF32::<4>::new();
        for i in 0..400u32 {
            e.observe(i as usize % 4);
        }
        let h = e.entropy().unwrap();
        assert!((h - 1.386).abs() < 0.01, "f32 entropy = {h}");
    }

    // =========================================================================
    // Default
    // =========================================================================

    #[test]
    fn default_is_empty() {
        let e = EntropyF64::<4>::default();
        assert_eq!(e.count(), 0);
    }
}
