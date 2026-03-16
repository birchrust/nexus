/// GCRA — Generic Cell Rate Algorithm (single-threaded).
///
/// From the ATM specification. Uses a single timestamp (Theoretical Arrival
/// Time) for O(1) rate limiting with no token tracking.
///
/// # Use Cases
/// - API request rate limiting
/// - Per-client message throttling
/// - Exchange order rate control
#[derive(Debug, Clone)]
pub struct Gcra {
    tat: u64,
    emission_interval: u64,
    tau: u64,
}

/// Builder for [`Gcra`].
#[derive(Debug, Clone)]
pub struct GcraBuilder {
    rate: Option<u64>,
    period: Option<u64>,
    burst: u64,
}

impl Gcra {
    /// Creates a builder.
    #[inline]
    #[must_use]
    pub fn builder() -> GcraBuilder {
        GcraBuilder {
            rate: None,
            period: None,
            burst: 0,
        }
    }

    /// Attempts to acquire with the given cost.
    ///
    /// Returns `true` if allowed, `false` if rate limited.
    /// `cost = 1` for a standard request. Higher for weighted operations.
    #[inline]
    #[must_use]
    pub fn try_acquire(&mut self, cost: u64, now: u64) -> bool {
        let new_tat = self.tat.max(now).saturating_add(cost.saturating_mul(self.emission_interval));
        if new_tat.saturating_sub(now) <= self.tau {
            self.tat = new_tat;
            true
        } else {
            false
        }
    }

    /// Time (in ticks) until a request of the given cost would be allowed.
    /// Returns 0 if allowed now.
    #[inline]
    #[must_use]
    pub fn time_until_allowed(&self, cost: u64, now: u64) -> u64 {
        let new_tat = self.tat.max(now).saturating_add(cost.saturating_mul(self.emission_interval));
        let excess = new_tat.saturating_sub(now);
        excess.saturating_sub(self.tau)
    }

    /// Reconfigure rate and burst at runtime. Takes effect immediately.
    #[inline]
    pub fn reconfigure(&mut self, rate: u64, period: u64, burst: u64) -> Result<(), crate::ConfigError> {
        if rate == 0 { return Err(crate::ConfigError::Invalid("rate must be > 0")); }
        if period == 0 { return Err(crate::ConfigError::Invalid("period must be > 0")); }
        let ei = period / rate;
        if ei == 0 { return Err(crate::ConfigError::Invalid("period / rate must be > 0")); }
        self.emission_interval = ei;
        self.tau = ei.saturating_mul(burst.saturating_add(1));
        Ok(())
    }

    /// Resets the limiter (clears TAT).
    #[inline]
    pub fn reset(&mut self) {
        self.tat = 0;
    }
}

impl GcraBuilder {
    /// Allowed requests per period (steady-state rate).
    #[inline]
    #[must_use]
    pub fn rate(mut self, n: u64) -> Self {
        self.rate = Some(n);
        self
    }

    /// Period length in timestamp units.
    #[inline]
    #[must_use]
    pub fn period(mut self, ticks: u64) -> Self {
        self.period = Some(ticks);
        self
    }

    /// Burst allowance above steady-state rate. Default: 0.
    #[inline]
    #[must_use]
    pub fn burst(mut self, n: u64) -> Self {
        self.burst = n;
        self
    }

    /// Builds the GCRA limiter.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::Missing` if rate or period not set.
    /// Returns `ConfigError::Invalid` if rate or period is zero.
    #[inline]
    pub fn build(self) -> Result<Gcra, crate::ConfigError> {
        let rate = self.rate.ok_or(crate::ConfigError::Missing("rate"))?;
        let period = self.period.ok_or(crate::ConfigError::Missing("period"))?;
        if rate == 0 { return Err(crate::ConfigError::Invalid("rate must be > 0")); }
        if period == 0 { return Err(crate::ConfigError::Invalid("period must be > 0")); }

        let emission_interval = period / rate;
        if emission_interval == 0 { return Err(crate::ConfigError::Invalid("period / rate must be > 0 (rate too high for period)")); }
        let tau = emission_interval.saturating_mul(self.burst.saturating_add(1));

        Ok(Gcra {
            tat: 0,
            emission_interval,
            tau,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_gcra() -> Gcra {
        // 10 requests per 1000 ticks, burst of 5
        Gcra::builder().rate(10).period(1000).burst(5).build().unwrap()
    }

    #[test]
    fn steady_rate_allowed() {
        let mut g = make_gcra();
        // emission_interval = 100, spacing at 100 ticks should always work
        for i in 0..100 {
            assert!(g.try_acquire(1, i * 100), "should be allowed at tick {}", i * 100);
        }
    }

    #[test]
    fn over_rate_rejected() {
        let mut g = make_gcra();
        // Burst: 5+1 = 6 requests allowed immediately
        for i in 0..6 {
            assert!(g.try_acquire(1, 0), "burst request {i} should be allowed");
        }
        // 7th should be rejected
        assert!(!g.try_acquire(1, 0), "should be rate limited");
    }

    #[test]
    fn burst_then_reject() {
        let mut g = Gcra::builder().rate(10).period(1000).burst(3).build().unwrap();
        // burst + 1 = 4 requests at time 0
        assert!(g.try_acquire(1, 0));
        assert!(g.try_acquire(1, 0));
        assert!(g.try_acquire(1, 0));
        assert!(g.try_acquire(1, 0));
        assert!(!g.try_acquire(1, 0)); // 5th rejected
    }

    #[test]
    fn time_passes_allows_again() {
        let mut g = make_gcra();
        // Exhaust burst
        for _ in 0..6 {
            let _ = g.try_acquire(1, 0);
        }
        assert!(!g.try_acquire(1, 0));

        // After enough time, should be allowed
        assert!(g.try_acquire(1, 200));
    }

    #[test]
    fn weighted_cost() {
        let mut g = Gcra::builder().rate(10).period(1000).burst(5).build().unwrap();
        // cost=3 consumes 3× the budget
        assert!(g.try_acquire(3, 0)); // uses 3 of 6 burst capacity
        assert!(g.try_acquire(3, 0)); // uses remaining 3
        assert!(!g.try_acquire(1, 0)); // exhausted
    }

    #[test]
    fn time_until_allowed_zero_when_ok() {
        let g = make_gcra();
        assert_eq!(g.time_until_allowed(1, 0), 0);
    }

    #[test]
    fn time_until_allowed_positive_when_limited() {
        let mut g = make_gcra();
        for _ in 0..6 {
            let _ = g.try_acquire(1, 0);
        }
        let wait = g.time_until_allowed(1, 0);
        assert!(wait > 0, "should need to wait, got {wait}");
    }

    #[test]
    fn reconfigure() {
        let mut g = make_gcra();
        // Exhaust at original rate
        for _ in 0..6 {
            let _ = g.try_acquire(1, 0);
        }
        assert!(!g.try_acquire(1, 0));

        // Double the rate — emission_interval halves, more burst
        g.reconfigure(20, 1000, 10).unwrap();
        // Need enough time for new config to allow: TAT=600, new tau=50*11=550
        // At now=100: new_tat=max(600,100)+50=650, excess=550, tau=550 → allowed
        assert!(g.try_acquire(1, 100));
    }

    #[test]
    fn reset_clears() {
        let mut g = make_gcra();
        for _ in 0..6 {
            let _ = g.try_acquire(1, 0);
        }
        g.reset();
        // Should be able to burst again
        assert!(g.try_acquire(1, 0));
    }

    #[test]
    fn cost_zero_always_allowed() {
        let mut g = make_gcra();
        // Exhaust burst
        for _ in 0..6 {
            let _ = g.try_acquire(1, 0);
        }
        assert!(!g.try_acquire(1, 0));
        // cost=0 should always succeed without consuming
        assert!(g.try_acquire(0, 0));
    }

    #[test]
    fn overflow_saturates() {
        let mut g = Gcra::builder().rate(1).period(100).burst(0).build().unwrap();
        // emission_interval = 100. cost * ei would overflow for huge cost
        assert!(!g.try_acquire(u64::MAX, 0)); // should saturate, not wrap
    }

    #[test]
    fn timestamp_backward() {
        let mut g = make_gcra();
        assert!(g.try_acquire(1, 100));
        // Going backward — should still work (max(tat, now) handles it)
        assert!(g.try_acquire(1, 50));
    }

    #[test]
    fn missing_rate_returns_error() {
        let result = Gcra::builder().period(1000).build();
        assert!(matches!(result, Err(crate::ConfigError::Missing("rate"))));
    }

    #[test]
    fn zero_rate_returns_error() {
        let result = Gcra::builder().rate(0).period(1000).build();
        assert!(matches!(result, Err(crate::ConfigError::Invalid(_))));
    }
}
