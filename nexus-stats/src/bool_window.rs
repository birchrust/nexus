/// Fixed-size sliding window of boolean outcomes.
///
/// Uses a ring buffer of `u64` words with popcount for O(1) rate queries.
/// The window size is `WORDS * 64` bits.
///
/// # Use Cases
/// - "What fraction of the last 64 requests failed?"
/// - Sliding window success/failure rate
/// - Recent error ratio for circuit breaker input
///
/// # Const Generic
///
/// `WORDS` is the number of `u64` words. The window holds `WORDS * 64` samples.
/// Common configurations:
/// - `BoolWindow<1>` — 64 samples (8 bytes)
/// - `BoolWindow<2>` — 128 samples (16 bytes)
/// - `BoolWindow<4>` — 256 samples (32 bytes)
pub struct BoolWindow<const WORDS: usize> {
    bits: [u64; WORDS],
    head: usize,
    capacity: usize,
    count: u64,
    failures: u32,
}

impl<const WORDS: usize> BoolWindow<WORDS> {
    /// Creates a new empty window.
    #[inline]
    pub fn new() -> Self {
        assert!(WORDS > 0, "BoolWindow must have at least 1 word");
        Self {
            bits: [0u64; WORDS],
            head: 0,
            capacity: WORDS * 64,
            count: 0,
            failures: 0,
        }
    }

    /// Total capacity of the window in samples.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Records an outcome. `true` = success, `false` = failure.
    #[inline]
    pub fn record(&mut self, success: bool) {
        let word = self.head / 64;
        let bit = self.head % 64;
        let mask = 1u64 << bit;

        // Evict old bit if window is full
        if self.count >= self.capacity as u64 && (self.bits[word] & mask) != 0 {
            // Old bit was 1 (failure) — decrement
            self.failures -= 1;
        }

        // Write new bit: 1 = failure, 0 = success
        if success {
            self.bits[word] &= !mask;
        } else {
            self.bits[word] |= mask;
            self.failures += 1;
        }

        self.head = (self.head + 1) % self.capacity;
        if self.count < self.capacity as u64 {
            self.count += 1;
        }
    }

    /// Failure rate over the window (0.0 to 1.0).
    ///
    /// Returns 0.0 if empty.
    #[inline]
    #[must_use]
    pub fn failure_rate(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.failures as f64 / self.count as f64
        }
    }

    /// Success rate over the window (0.0 to 1.0).
    ///
    /// Returns 1.0 if empty.
    #[inline]
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        1.0 - self.failure_rate()
    }

    /// Number of failures in the current window.
    #[inline]
    #[must_use]
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// Number of samples in the window (min of total recorded and capacity).
    #[inline]
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Whether the window is full.
    #[inline]
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.count >= self.capacity as u64
    }

    /// Resets to empty state.
    #[inline]
    pub fn reset(&mut self) {
        self.bits = [0u64; WORDS];
        self.head = 0;
        self.count = 0;
        self.failures = 0;
    }
}

impl<const WORDS: usize> Default for BoolWindow<WORDS> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<const WORDS: usize> core::fmt::Debug for BoolWindow<WORDS> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BoolWindow")
            .field("capacity", &self.capacity)
            .field("count", &self.count)
            .field("failures", &self.failures)
            .finish()
    }
}

impl<const WORDS: usize> Clone for BoolWindow<WORDS> {
    fn clone(&self) -> Self {
        Self {
            bits: self.bits,
            head: self.head,
            capacity: self.capacity,
            count: self.count,
            failures: self.failures,
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let bw = BoolWindow::<1>::new();
        assert_eq!(bw.count(), 0);
        assert_eq!(bw.capacity(), 64);
        assert_eq!(bw.failure_rate(), 0.0);
        assert_eq!(bw.success_rate(), 1.0);
    }

    #[test]
    fn all_success() {
        let mut bw = BoolWindow::<1>::new();
        for _ in 0..64 {
            bw.record(true);
        }
        assert_eq!(bw.failure_rate(), 0.0);
        assert_eq!(bw.failures(), 0);
    }

    #[test]
    fn all_failure() {
        let mut bw = BoolWindow::<1>::new();
        for _ in 0..64 {
            bw.record(false);
        }
        assert_eq!(bw.failure_rate(), 1.0);
        assert_eq!(bw.failures(), 64);
    }

    #[test]
    fn half_and_half() {
        let mut bw = BoolWindow::<1>::new();
        for i in 0..64 {
            bw.record(i % 2 == 0); // 32 success, 32 failure
        }
        assert!((bw.failure_rate() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn window_rolls() {
        let mut bw = BoolWindow::<1>::new();
        // Fill with failures
        for _ in 0..64 {
            bw.record(false);
        }
        assert_eq!(bw.failures(), 64);

        // Now push successes — old failures should be evicted
        for _ in 0..64 {
            bw.record(true);
        }
        assert_eq!(bw.failures(), 0);
    }

    #[test]
    fn multi_word() {
        let mut bw = BoolWindow::<2>::new(); // 128 samples
        assert_eq!(bw.capacity(), 128);

        for _ in 0..128 {
            bw.record(false);
        }
        assert_eq!(bw.failures(), 128);
        assert!(bw.is_full());

        for _ in 0..128 {
            bw.record(true);
        }
        assert_eq!(bw.failures(), 0);
    }

    #[test]
    fn priming() {
        let mut bw = BoolWindow::<1>::new();
        bw.record(false);
        assert_eq!(bw.count(), 1);
        assert!(!bw.is_full());
        assert_eq!(bw.failure_rate(), 1.0);
    }

    #[test]
    fn reset() {
        let mut bw = BoolWindow::<1>::new();
        for _ in 0..64 {
            bw.record(false);
        }
        bw.reset();
        assert_eq!(bw.count(), 0);
        assert_eq!(bw.failures(), 0);
    }

    #[test]
    fn default_is_empty() {
        let bw = BoolWindow::<1>::default();
        assert_eq!(bw.count(), 0);
    }
}
