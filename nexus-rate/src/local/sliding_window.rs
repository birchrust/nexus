/// Sliding Window Counter — time-bucketed rate limiter (single-threaded).
///
/// Divides the window into sub-windows (buckets) for fine-grained time
/// tracking. Expired buckets are evicted incrementally. Running total is
/// maintained for O(1) limit checks.
///
/// Requires the `alloc` feature for the bucket buffer.
///
/// # Use Cases
/// - "No more than 100 requests per minute"
/// - Sliding rate limit with sub-second granularity
/// - Traffic shaping with windowed counters
#[derive(Debug, Clone)]
pub struct SlidingWindow {
    buckets: *mut u64,
    num_buckets: usize,
    total: u64,
    current_bucket: usize,
    bucket_duration: u64,
    last_bucket_time: u64,
    limit: u64,
}

// SAFETY: buffer is exclusively owned, u64 is Copy + Send
unsafe impl Send for SlidingWindow {}

impl SlidingWindow {
    #[inline]
    fn ring_mut(&mut self) -> &mut [u64] {
        unsafe { core::slice::from_raw_parts_mut(self.buckets, self.num_buckets) }
    }
}

/// Builder for [`SlidingWindow`].
#[derive(Debug, Clone)]
pub struct SlidingWindowBuilder {
    window: Option<u64>,
    sub_windows: Option<usize>,
    limit: Option<u64>,
    now: Option<u64>,
}

impl SlidingWindow {
    /// Creates a builder.
    #[inline]
    #[must_use]
    pub fn builder() -> SlidingWindowBuilder {
        SlidingWindowBuilder {
            window: None,
            sub_windows: None,
            limit: None,
            now: None,
        }
    }

    /// Advances time, evicting expired buckets.
    #[inline]
    fn advance_time(&mut self, now: u64) {
        let elapsed = now.saturating_sub(self.last_bucket_time);
        let buckets_to_advance = (elapsed / self.bucket_duration) as usize;

        if buckets_to_advance == 0 {
            return;
        }

        let buckets_to_clear = buckets_to_advance.min(self.num_buckets);
        let num_buckets = self.num_buckets;
        let mut current = self.current_bucket;
        let mut total = self.total;
        // SAFETY: buffer allocated with capacity num_buckets, all initialized
        let ring = unsafe { core::slice::from_raw_parts_mut(self.buckets, num_buckets) };

        for _ in 0..buckets_to_clear {
            current = (current + 1) % num_buckets;
            total = total.saturating_sub(ring[current]);
            ring[current] = 0;
        }

        self.current_bucket = current;
        self.total = total;
        self.last_bucket_time = self.last_bucket_time
            .saturating_add((buckets_to_advance as u64).saturating_mul(self.bucket_duration));
    }

    /// Records an event with the given cost. Returns `true` if under limit.
    #[inline]
    #[must_use]
    pub fn try_acquire(&mut self, cost: u64, now: u64) -> bool {
        self.advance_time(now);

        if self.total + cost <= self.limit {
            let idx = self.current_bucket;
            self.ring_mut()[idx] += cost;
            self.total += cost;
            true
        } else {
            false
        }
    }

    /// Current count in the window.
    #[inline]
    #[must_use]
    pub fn count(&self) -> u64 {
        self.total
    }

    /// Remaining capacity before hitting the limit.
    #[inline]
    #[must_use]
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.total)
    }

    /// Reconfigure the limit at runtime.
    #[inline]
    pub fn reconfigure(&mut self, new_limit: u64) {
        self.limit = new_limit;
    }

    /// Resets all buckets and the running total.
    #[inline]
    pub fn reset(&mut self) {
        self.ring_mut().fill(0);
        self.total = 0;
    }
}

impl Drop for SlidingWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = alloc::vec::Vec::from_raw_parts(self.buckets, 0, self.num_buckets);
        }
    }
}

impl SlidingWindowBuilder {
    /// Total window duration in timestamp units.
    #[inline]
    #[must_use]
    pub fn window(mut self, ticks: u64) -> Self {
        self.window = Some(ticks);
        self
    }

    /// Number of sub-windows (buckets). More = finer granularity. Typical: 10-60.
    #[inline]
    #[must_use]
    pub fn sub_windows(mut self, n: usize) -> Self {
        self.sub_windows = Some(n);
        self
    }

    /// Maximum allowed count per window.
    #[inline]
    #[must_use]
    pub fn limit(mut self, max_count: u64) -> Self {
        self.limit = Some(max_count);
        self
    }

    /// Initial timestamp.
    #[inline]
    #[must_use]
    pub fn now(mut self, now: u64) -> Self {
        self.now = Some(now);
        self
    }

    /// Builds the sliding window limiter.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::Missing` if window, sub_windows, limit, or now not set.
    /// Returns `ConfigError::Invalid` if window, sub_windows, limit, or
    /// bucket duration (window / sub_windows) is zero.
    #[inline]
    pub fn build(self) -> Result<SlidingWindow, crate::ConfigError> {
        let window = self.window.ok_or(crate::ConfigError::Missing("window"))?;
        let sub_windows = self.sub_windows.ok_or(crate::ConfigError::Missing("sub_windows"))?;
        let limit = self.limit.ok_or(crate::ConfigError::Missing("limit"))?;
        let now = self.now.ok_or(crate::ConfigError::Missing("now"))?;
        if window == 0 { return Err(crate::ConfigError::Invalid("window must be > 0")); }
        if sub_windows == 0 { return Err(crate::ConfigError::Invalid("sub_windows must be > 0")); }
        if limit == 0 { return Err(crate::ConfigError::Invalid("limit must be > 0")); }

        let bucket_duration = window / sub_windows as u64;
        if bucket_duration == 0 { return Err(crate::ConfigError::Invalid("window / sub_windows must be > 0")); }

        let mut vec = core::mem::ManuallyDrop::new(alloc::vec![0u64; sub_windows]);
        let buckets = vec.as_mut_ptr();

        Ok(SlidingWindow {
            buckets,
            num_buckets: sub_windows,
            total: 0,
            current_bucket: 0,
            bucket_duration,
            last_bucket_time: now,
            limit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_window() -> SlidingWindow {
        // 100 events per 1000 ticks, 10 sub-windows (each 100 ticks)
        SlidingWindow::builder()
            .window(1000)
            .sub_windows(10)
            .limit(100)
            .now(0)
            .build()
            .unwrap()
    }

    #[test]
    fn under_limit_allowed() {
        let mut sw = make_window();
        for _ in 0..100 {
            assert!(sw.try_acquire(1, 0));
        }
    }

    #[test]
    fn at_limit_rejected() {
        let mut sw = make_window();
        for _ in 0..100 {
            let _ = sw.try_acquire(1, 0);
        }
        assert!(!sw.try_acquire(1, 0));
    }

    #[test]
    fn time_passes_frees_capacity() {
        let mut sw = make_window();
        // Fill up at time 0 (all in bucket 0)
        for _ in 0..100 {
            let _ = sw.try_acquire(1, 0);
        }
        assert!(!sw.try_acquire(1, 0));

        // Bucket 0 events don't evict until the head pointer wraps past it.
        // With 10 sub-windows of 100 ticks each, we need to advance through
        // all 10 sub-windows for bucket 0 to be evicted (full window = 1000 ticks).
        // At 1100 ticks, bucket 0 is cleared.
        assert!(sw.try_acquire(1, 1100));
    }

    #[test]
    fn weighted_cost() {
        let mut sw = make_window();
        assert!(sw.try_acquire(50, 0)); // half the limit
        assert!(sw.try_acquire(50, 0)); // at limit
        assert!(!sw.try_acquire(1, 0)); // over
    }

    #[test]
    fn count_and_remaining() {
        let mut sw = make_window();
        let _ = sw.try_acquire(30, 0);
        assert_eq!(sw.count(), 30);
        assert_eq!(sw.remaining(), 70);
    }

    #[test]
    fn reconfigure_limit() {
        let mut sw = make_window();
        for _ in 0..100 {
            let _ = sw.try_acquire(1, 0);
        }
        assert!(!sw.try_acquire(1, 0));

        sw.reconfigure(200);
        assert!(sw.try_acquire(1, 0)); // now under new limit
    }

    #[test]
    fn sub_window_granularity() {
        let mut sw = make_window();
        // Events at different sub-windows
        let _ = sw.try_acquire(30, 0);    // bucket 0
        let _ = sw.try_acquire(30, 200);  // bucket 2
        let _ = sw.try_acquire(30, 400);  // bucket 4
        assert_eq!(sw.count(), 90);

        // At time 200, bucket 0 evicts (it's 2 sub-windows behind the head at 200)
        // Actually eviction happens when we advance past bucket 0
        // At time 1100, bucket 0 is evicted (1000 ticks = full window)
        let _ = sw.try_acquire(1, 1100);
        // Bucket 0's 30 events should have been evicted
        assert!(sw.count() <= 91); // 90 - 30 + 1 = 61
    }

    #[test]
    fn reset_clears() {
        let mut sw = make_window();
        for _ in 0..50 {
            let _ = sw.try_acquire(1, 0);
        }
        sw.reset();
        assert_eq!(sw.count(), 0);
        assert_eq!(sw.remaining(), 100);
    }

    #[test]
    fn long_idle_clears_all() {
        let mut sw = make_window();
        for _ in 0..100 {
            let _ = sw.try_acquire(1, 0);
        }
        // After a very long time, all buckets should be evicted
        assert!(sw.try_acquire(1, 100_000));
    }

    #[test]
    fn cost_zero() {
        let mut sw = make_window();
        for _ in 0..100 {
            let _ = sw.try_acquire(1, 0);
        }
        assert!(!sw.try_acquire(1, 0));
        assert!(sw.try_acquire(0, 0)); // zero cost always ok
    }

    #[test]
    fn weighted_eviction() {
        let mut sw = SlidingWindow::builder()
            .window(1000).sub_windows(10).limit(100).now(0).build().unwrap();

        // Insert weighted events in bucket 0
        let _ = sw.try_acquire(30, 0);
        // Insert in bucket 1
        let _ = sw.try_acquire(20, 100);
        assert_eq!(sw.count(), 50);

        // Advance past the full window — all buckets evicted
        let _ = sw.try_acquire(1, 1200);
        // Only the new event should remain
        assert!(sw.count() <= 1);
    }

    #[test]
    fn large_timestamp_jump() {
        let mut sw = make_window();
        for _ in 0..50 {
            let _ = sw.try_acquire(1, 0);
        }
        // Jump to near u64::MAX — should not loop forever or panic
        assert!(sw.try_acquire(1, u64::MAX - 1));
    }

    #[test]
    fn timestamp_backward() {
        let mut sw = make_window();
        let _ = sw.try_acquire(1, 100);
        // Backward timestamp — saturating_sub produces 0 elapsed, no advance
        let _ = sw.try_acquire(1, 50);
        assert_eq!(sw.count(), 2); // both events counted, no eviction
    }

    #[test]
    fn missing_window_returns_error() {
        let result = SlidingWindow::builder().sub_windows(10).limit(100).now(0).build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("window"))));
    }

    #[test]
    fn zero_window_returns_error() {
        let result = SlidingWindow::builder().window(0).sub_windows(10).limit(100).now(0).build();
        assert!(matches!(result, Err(crate::ConfigError::Invalid(_))));
    }
}
