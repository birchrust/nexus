/// Token Bucket — lazy token computation (single-threaded).
///
/// Folly-style: stores a `zero_time` instead of a token count. Tokens are
/// computed lazily from elapsed time on each call. No timer needed.
///
/// # Use Cases
/// - Bandwidth limiting
/// - Request quota enforcement
/// - Metered resource access
#[derive(Debug, Clone)]
pub struct TokenBucket {
    zero_time: u64,
    rate: u64,
    period: u64,
    burst: u64,
}

/// Builder for [`TokenBucket`].
#[derive(Debug, Clone)]
pub struct TokenBucketBuilder {
    rate: Option<u64>,
    period: Option<u64>,
    burst: Option<u64>,
    now: Option<u64>,
}

impl TokenBucket {
    /// Creates a builder.
    #[inline]
    #[must_use]
    pub fn builder() -> TokenBucketBuilder {
        TokenBucketBuilder {
            rate: None,
            period: None,
            burst: None,
            now: None,
        }
    }

    /// Computes available tokens without consuming.
    #[inline]
    fn compute_available(&self, now: u64) -> u64 {
        let elapsed = now.saturating_sub(self.zero_time);
        // Use u128 to avoid overflow: elapsed * rate can exceed u64
        let tokens = elapsed as u128 * self.rate as u128 / self.period as u128;
        tokens.min(self.burst as u128) as u64
    }

    /// Attempts to consume `cost` tokens.
    ///
    /// Returns `true` if enough tokens are available. Tokens accumulate in
    /// whole units only — a token becomes available after `period / rate`
    /// ticks, not fractionally. This is standard for integer rate limiting.
    #[inline]
    #[must_use]
    pub fn try_acquire(&mut self, cost: u64, now: u64) -> bool {
        let available = self.compute_available(now);
        if available >= cost {
            // Consume by advancing zero_time (ceiling division to avoid fractional drift)
            let consume_ticks = (cost as u128 * self.period as u128).div_ceil(self.rate as u128) as u64;
            self.zero_time += consume_ticks;
            true
        } else {
            false
        }
    }

    /// Available tokens right now (without consuming).
    #[inline]
    #[must_use]
    pub fn available(&self, now: u64) -> u64 {
        self.compute_available(now)
    }

    /// Reconfigure rate and burst at runtime.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::Invalid` if rate or period is zero, or if
    /// `period / rate` rounds to zero.
    #[inline]
    pub fn reconfigure(&mut self, rate: u64, period: u64, burst: u64) -> Result<(), crate::ConfigError> {
        if rate == 0 { return Err(crate::ConfigError::Invalid("rate must be > 0")); }
        if period == 0 { return Err(crate::ConfigError::Invalid("period must be > 0")); }
        if period / rate == 0 { return Err(crate::ConfigError::Invalid("period / rate must be > 0")); }
        self.rate = rate;
        self.period = period;
        self.burst = burst;
        Ok(())
    }

    /// Resets to a fresh state at the given timestamp.
    #[inline]
    pub fn reset(&mut self, now: u64) {
        self.zero_time = now;
    }
}

impl TokenBucketBuilder {
    /// Token refill rate (tokens per period).
    #[inline]
    #[must_use]
    pub fn rate(mut self, tokens_per_period: u64) -> Self {
        self.rate = Some(tokens_per_period);
        self
    }

    /// Period length in timestamp units.
    #[inline]
    #[must_use]
    pub fn period(mut self, ticks: u64) -> Self {
        self.period = Some(ticks);
        self
    }

    /// Maximum burst size (bucket capacity).
    #[inline]
    #[must_use]
    pub fn burst(mut self, max_tokens: u64) -> Self {
        self.burst = Some(max_tokens);
        self
    }

    /// Initial timestamp.
    #[inline]
    #[must_use]
    pub fn now(mut self, now: u64) -> Self {
        self.now = Some(now);
        self
    }

    /// Builds the token bucket.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::Missing` if rate, period, burst, or now not set.
    /// Returns `ConfigError::Invalid` if rate or period is zero.
    #[inline]
    pub fn build(self) -> Result<TokenBucket, crate::ConfigError> {
        let rate = self.rate.ok_or(crate::ConfigError::Missing("rate"))?;
        let period = self.period.ok_or(crate::ConfigError::Missing("period"))?;
        let burst = self.burst.ok_or(crate::ConfigError::Missing("burst"))?;
        let now = self.now.ok_or(crate::ConfigError::Missing("now"))?;
        if rate == 0 { return Err(crate::ConfigError::Invalid("rate must be > 0")); }
        if period == 0 { return Err(crate::ConfigError::Invalid("period must be > 0")); }
        if period / rate == 0 { return Err(crate::ConfigError::Invalid("period / rate must be > 0")); }

        Ok(TokenBucket {
            zero_time: now,
            rate,
            period,
            burst,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bucket() -> TokenBucket {
        // 10 tokens per 1000 ticks, burst of 20
        TokenBucket::builder().rate(10).period(1000).burst(20).now(0).build().unwrap()
    }

    #[test]
    fn initial_burst() {
        let mut tb = make_bucket();
        // At time 0, zero_time=0, elapsed=0, available=0
        // Need to advance time for tokens
        assert!(!tb.try_acquire(1, 0)); // no tokens yet

        // After 1000 ticks, 10 tokens available
        assert!(tb.try_acquire(10, 1000));
        assert!(!tb.try_acquire(1, 1000)); // exhausted
    }

    #[test]
    fn steady_rate() {
        let mut tb = make_bucket();
        // Consume 1 token every 100 ticks (= rate of 10/1000)
        for i in 1..=50 {
            assert!(tb.try_acquire(1, i * 100), "should be allowed at tick {}", i * 100);
        }
    }

    #[test]
    fn burst_cap() {
        let tb = make_bucket();
        // Wait long enough for burst to fill: burst=20, rate=10/1000
        // 20 tokens at 2000 ticks
        assert_eq!(tb.available(2000), 20);
        // Even at 10000 ticks, still capped at burst=20
        assert_eq!(tb.available(10000), 20);
    }

    #[test]
    fn token_replenishment() {
        let mut tb = make_bucket();
        // Consume all tokens
        assert!(tb.try_acquire(10, 1000));
        assert!(!tb.try_acquire(1, 1000));

        // Wait 500 ticks → 5 tokens
        assert_eq!(tb.available(1500), 5);
        assert!(tb.try_acquire(5, 1500));
    }

    #[test]
    fn weighted_cost() {
        let mut tb = make_bucket();
        // 10 tokens at 1000 ticks
        assert!(tb.try_acquire(5, 1000)); // consume 5
        assert_eq!(tb.available(1000), 5); // 5 left
        assert!(tb.try_acquire(5, 1000)); // consume remaining 5
        assert!(!tb.try_acquire(1, 1000)); // empty
    }

    #[test]
    fn available_does_not_consume() {
        let tb = make_bucket();
        let a1 = tb.available(1000);
        let a2 = tb.available(1000);
        assert_eq!(a1, a2);
    }

    #[test]
    fn reconfigure() {
        let mut tb = make_bucket();
        // Double the rate
        tb.reconfigure(20, 1000, 40).unwrap();
        // Now 20 tokens per 1000 ticks
        assert_eq!(tb.available(1000), 20);
    }

    #[test]
    fn reset() {
        let mut tb = make_bucket();
        let _ = tb.try_acquire(5, 1000);
        tb.reset(2000);
        // After reset at 2000, available at 2000 = 0 (just reset)
        assert_eq!(tb.available(2000), 0);
        // At 3000, 10 tokens
        assert_eq!(tb.available(3000), 10);
    }

    #[test]
    fn cost_zero_always_allowed() {
        let mut tb = make_bucket();
        assert!(tb.try_acquire(0, 0)); // zero cost, no tokens needed
    }

    #[test]
    fn timestamp_backward() {
        let mut tb = make_bucket();
        let _ = tb.try_acquire(5, 1000);
        // Go backward — saturating_sub produces elapsed=0, no new tokens
        let available = tb.available(500);
        assert_eq!(available, 0, "backward timestamp should produce 0 available");
    }

    #[test]
    fn missing_rate_returns_error() {
        let result = TokenBucket::builder().period(1000).burst(10).now(0).build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("rate"))));
    }

    #[test]
    fn zero_rate_returns_error() {
        let result = TokenBucket::builder().rate(0).period(1000).burst(10).now(0).build();
        assert!(matches!(result, Err(crate::ConfigError::Invalid(_))));
    }
}
