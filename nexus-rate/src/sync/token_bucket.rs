use core::sync::atomic::{AtomicU64, Ordering};

/// Token Bucket — lazy token computation (thread-safe).
///
/// Same Folly-style algorithm as [`local::TokenBucket`](crate::local::TokenBucket)
/// but uses an `AtomicU64` for `zero_time` with a CAS loop.
///
/// # Thread Safety
///
/// `try_acquire` and `available` take `&self`. Safe to share via `Arc`.
/// `reconfigure` uses `&mut self` (control-plane operation).
pub struct TokenBucket {
    zero_time: AtomicU64,
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

    /// Attempts to consume `cost` tokens (thread-safe).
    ///
    /// Uses a CAS loop on `zero_time`.
    #[inline]
    #[must_use]
    pub fn try_acquire(&self, cost: u64, now: u64) -> bool {
        loop {
            let zero_time = self.zero_time.load(Ordering::Relaxed);
            let elapsed = now.saturating_sub(zero_time);
            let available = ((elapsed as u128 * self.rate as u128 / self.period as u128) as u64)
                .min(self.burst);

            if available < cost {
                return false;
            }

            let consume_ticks = (cost as u128 * self.period as u128 / self.rate as u128) as u64;
            let new_zero_time = zero_time + consume_ticks;

            if self.zero_time.compare_exchange_weak(
                zero_time,
                new_zero_time,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ).is_ok() {
                return true;
            }
        }
    }

    /// Available tokens right now (without consuming).
    #[inline]
    #[must_use]
    pub fn available(&self, now: u64) -> u64 {
        let zero_time = self.zero_time.load(Ordering::Relaxed);
        let elapsed = now.saturating_sub(zero_time);
        ((elapsed as u128 * self.rate as u128 / self.period as u128) as u64).min(self.burst)
    }

    /// Reconfigure rate and burst. Control-plane operation.
    #[inline]
    pub fn reconfigure(&mut self, rate: u64, period: u64, burst: u64) {
        self.rate = rate;
        self.period = period;
        self.burst = burst;
    }

    /// Resets to fresh state at the given timestamp.
    #[inline]
    pub fn reset(&self, now: u64) {
        self.zero_time.store(now, Ordering::Release);
    }
}

impl core::fmt::Debug for TokenBucket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("sync::TokenBucket")
            .field("zero_time", &self.zero_time.load(Ordering::Relaxed))
            .field("rate", &self.rate)
            .field("period", &self.period)
            .field("burst", &self.burst)
            .finish()
    }
}

impl TokenBucketBuilder {
    /// Token refill rate.
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

    /// Maximum burst size.
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

    /// Builds the thread-safe token bucket.
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

        Ok(TokenBucket {
            zero_time: AtomicU64::new(now),
            rate,
            period,
            burst,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        let tb = TokenBucket::builder().rate(10).period(1000).burst(20).now(0).build().unwrap();
        assert_eq!(tb.available(1000), 10);
        assert!(tb.try_acquire(10, 1000));
        assert!(!tb.try_acquire(1, 1000));
    }

    #[test]
    fn burst_cap() {
        let tb = TokenBucket::builder().rate(10).period(1000).burst(20).now(0).build().unwrap();
        assert_eq!(tb.available(10000), 20); // capped at burst
    }

    #[test]
    fn reset() {
        let tb = TokenBucket::builder().rate(10).period(1000).burst(20).now(0).build().unwrap();
        let _ = tb.try_acquire(10, 1000);
        tb.reset(5000);
        assert_eq!(tb.available(5000), 0);
    }

    #[cfg(feature = "std")]
    #[test]
    #[allow(clippy::needless_collect)]
    fn concurrent_consumption() {
        use std::sync::Arc;
        use std::thread;

        let tb = Arc::new(
            TokenBucket::builder().rate(1000).period(1000).burst(100).now(0).build().unwrap(),
        );

        let threads: Vec<_> = (0..4)
            .map(|_| {
                let tb = Arc::clone(&tb);
                thread::spawn(move || {
                    let mut allowed = 0u64;
                    for t in 1..=100 {
                        if tb.try_acquire(1, t * 10) {
                            allowed += 1;
                        }
                    }
                    allowed
                })
            })
            .collect();

        let total: u64 = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert!(total > 0);
    }

    #[test]
    fn available_query() {
        let tb = TokenBucket::builder().rate(10).period(1000).burst(20).now(0).build().unwrap();
        assert_eq!(tb.available(1000), 10);
        assert_eq!(tb.available(1000), 10); // doesn't consume
    }

    #[test]
    fn cost_zero() {
        let tb = TokenBucket::builder().rate(10).period(1000).burst(10).now(0).build().unwrap();
        assert!(tb.try_acquire(0, 0));
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
