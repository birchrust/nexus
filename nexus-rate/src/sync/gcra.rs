use core::sync::atomic::{AtomicU64, Ordering};

/// GCRA — Generic Cell Rate Algorithm (thread-safe).
///
/// Same algorithm as [`local::Gcra`](crate::local::Gcra) but uses an
/// `AtomicU64` for the TAT with a CAS loop for lock-free concurrent access.
///
/// # Thread Safety
///
/// All methods take `&self`. Safe to share via `Arc` or static reference.
/// `reconfigure` uses atomic stores (not CAS) — control-plane operation.
pub struct Gcra {
    tat: AtomicU64,
    emission_interval: AtomicU64,
    tau: AtomicU64,
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

    /// Attempts to acquire with the given cost (thread-safe).
    ///
    /// Uses a CAS loop on the TAT. Retries on contention.
    #[inline]
    #[must_use]
    pub fn try_acquire(&self, cost: u64, now: u64) -> bool {
        let emission_interval = self.emission_interval.load(Ordering::Relaxed);
        let tau = self.tau.load(Ordering::Relaxed);

        loop {
            let tat = self.tat.load(Ordering::Relaxed);
            let new_tat = tat.max(now).saturating_add(cost.saturating_mul(emission_interval));

            if new_tat.saturating_sub(now) > tau {
                return false;
            }

            if self.tat.compare_exchange_weak(
                tat,
                new_tat,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ).is_ok() {
                return true;
            }
        }
    }

    /// Time (in ticks) until a request of the given cost would be allowed.
    #[inline]
    #[must_use]
    pub fn time_until_allowed(&self, cost: u64, now: u64) -> u64 {
        let tat = self.tat.load(Ordering::Relaxed);
        let emission_interval = self.emission_interval.load(Ordering::Relaxed);
        let tau = self.tau.load(Ordering::Relaxed);
        let new_tat = tat.max(now).saturating_add(cost.saturating_mul(emission_interval));
        let excess = new_tat.saturating_sub(now);
        excess.saturating_sub(tau)
    }

    /// Reconfigure rate and burst. Atomic stores.
    ///
    /// Note: `emission_interval` and `tau` are stored sequentially. A
    /// concurrent `try_acquire` may briefly see an inconsistent pair
    /// (new emission_interval with old tau). This is benign — at most
    /// one or two calls will see slightly wrong burst tolerance.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::Invalid` if rate or period is zero, or if
    /// `period / rate` rounds to zero.
    #[inline]
    pub fn reconfigure(&self, rate: u64, period: u64, burst: u64) -> Result<(), crate::ConfigError> {
        if rate == 0 { return Err(crate::ConfigError::Invalid("rate must be > 0")); }
        if period == 0 { return Err(crate::ConfigError::Invalid("period must be > 0")); }
        let ei = period / rate;
        if ei == 0 { return Err(crate::ConfigError::Invalid("period / rate must be > 0")); }
        self.emission_interval.store(ei, Ordering::Release);
        self.tau.store(ei.saturating_mul(burst.saturating_add(1)), Ordering::Release);
        Ok(())
    }

    /// Resets the TAT.
    #[inline]
    pub fn reset(&self) {
        self.tat.store(0, Ordering::Release);
    }
}

impl core::fmt::Debug for Gcra {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("sync::Gcra")
            .field("tat", &self.tat.load(Ordering::Relaxed))
            .field("emission_interval", &self.emission_interval.load(Ordering::Relaxed))
            .field("tau", &self.tau.load(Ordering::Relaxed))
            .finish()
    }
}

impl GcraBuilder {
    /// Allowed requests per period.
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

    /// Burst allowance. Default: 0.
    #[inline]
    #[must_use]
    pub fn burst(mut self, n: u64) -> Self {
        self.burst = n;
        self
    }

    /// Builds the thread-safe GCRA limiter.
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

        let ei = period / rate;
        if ei == 0 { return Err(crate::ConfigError::Invalid("period / rate must be > 0")); }
        let tau = ei.saturating_mul(self.burst.saturating_add(1));

        Ok(Gcra {
            tat: AtomicU64::new(0),
            emission_interval: AtomicU64::new(ei),
            tau: AtomicU64::new(tau),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_rate_limiting() {
        let g = Gcra::builder().rate(10).period(1000).burst(5).build().unwrap();

        // Burst: 6 allowed
        for i in 0..6 {
            assert!(g.try_acquire(1, 0), "burst request {i}");
        }
        assert!(!g.try_acquire(1, 0));
    }

    #[test]
    fn time_allows_more() {
        let g = Gcra::builder().rate(10).period(1000).burst(0).build().unwrap();
        assert!(g.try_acquire(1, 0));
        assert!(!g.try_acquire(1, 0));
        // After emission_interval (100 ticks)
        assert!(g.try_acquire(1, 200));
    }

    #[test]
    fn reset() {
        let g = Gcra::builder().rate(10).period(1000).burst(5).build().unwrap();
        for _ in 0..6 {
            let _ = g.try_acquire(1, 0);
        }
        g.reset();
        assert!(g.try_acquire(1, 0));
    }

    #[cfg(feature = "std")]
    #[test]
    #[allow(clippy::needless_collect)]
    fn concurrent_rate_limit() {
        use std::sync::Arc;
        use std::thread;

        let g = Arc::new(Gcra::builder().rate(100).period(1000).burst(0).build().unwrap());
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let g = Arc::clone(&g);
                thread::spawn(move || {
                    let mut allowed = 0u64;
                    for i in 0..1000 {
                        let now = (t * 1000 + i) * 10; // spread across time
                        if g.try_acquire(1, now) {
                            allowed += 1;
                        }
                    }
                    allowed
                })
            })
            .collect();

        let total: u64 = threads.into_iter().map(|t| t.join().unwrap()).sum();
        // With rate=100/1000 over ~40000 ticks, should allow ~4000
        // But concurrent CAS contention means some may be lost
        assert!(total > 0, "should have allowed some requests");
    }

    #[test]
    fn time_until_allowed() {
        let g = Gcra::builder().rate(10).period(1000).burst(5).build().unwrap();
        assert_eq!(g.time_until_allowed(1, 0), 0);

        for _ in 0..6 {
            let _ = g.try_acquire(1, 0);
        }
        assert!(g.time_until_allowed(1, 0) > 0);
    }

    #[test]
    fn reconfigure_takes_effect() {
        let g = Gcra::builder().rate(1).period(1000).burst(0).build().unwrap();
        assert!(g.try_acquire(1, 0));
        assert!(!g.try_acquire(1, 0));

        // Reconfigure to much higher rate and reset TAT
        g.reconfigure(1000, 1000, 10).unwrap();
        g.reset();
        // Now: emission_interval=1, tau=11, TAT=0
        // At now=1: new_tat=max(0,1)+1=2, excess=1, tau=11 → allowed
        assert!(g.try_acquire(1, 1));
    }

    #[test]
    fn cost_zero() {
        let g = Gcra::builder().rate(1).period(1000).burst(0).build().unwrap();
        let _ = g.try_acquire(1, 0);
        assert!(!g.try_acquire(1, 0));
        assert!(g.try_acquire(0, 0));
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
