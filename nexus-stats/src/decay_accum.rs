/// Decaying accumulator — event-driven score with exponential decay.
///
/// Lazy evaluation: only computes decay when `accumulate()` or `score()` is called.
/// Between calls, no work is done.
///
/// # Use Cases
/// - Weighted event scoring with temporal decay
/// - "How active has this been recently?"
/// - Rate limiting with smooth backoff
#[derive(Debug, Clone)]
pub struct DecayAccumF64 {
    score: f64,
    last_time: f64,
    decay_constant: f64, // ln(2) / half_life
    initialized: bool,
}

impl DecayAccumF64 {
    /// Creates a new decaying accumulator with the given half-life.
    ///
    /// `half_life` is in the same time units as the timestamps passed to
    /// `accumulate()` and `score()`.
    #[inline]
    pub fn new(half_life: f64) -> Result<Self, crate::ConfigError> {
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(half_life > 0.0) {
            return Err(crate::ConfigError::Invalid("half_life must be positive"));
        }
        Ok(Self {
            score: 0.0,
            last_time: 0.0,
            decay_constant: core::f64::consts::LN_2 / half_life,
            initialized: false,
        })
    }

    /// Adds weight at the given timestamp.
    ///
    /// Applies decay from the last event/query before adding.
    #[inline]
    pub fn accumulate(&mut self, timestamp: f64, weight: f64) {
        self.apply_decay(timestamp);
        self.score += weight;
    }

    /// Queries the current decayed score at the given timestamp.
    #[inline]
    #[must_use]
    pub fn score(&mut self, now: f64) -> f64 {
        self.apply_decay(now);
        self.score
    }

    #[inline]
    fn apply_decay(&mut self, timestamp: f64) {
        if !self.initialized {
            self.last_time = timestamp;
            self.initialized = true;
            return;
        }

        let dt = timestamp - self.last_time;
        if dt > 0.0 {
            self.score *= crate::math::exp(-self.decay_constant * dt);
            self.last_time = timestamp;
        }
    }

    /// Resets to zero score.
    #[inline]
    pub fn reset(&mut self) {
        self.score = 0.0;
        self.initialized = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates() {
        let mut da = DecayAccumF64::new(10.0).unwrap();
        da.accumulate(0.0, 1.0);
        da.accumulate(0.0, 1.0);
        let s = da.score(0.0);
        assert!((s - 2.0).abs() < 1e-10);
    }

    #[test]
    fn decays_over_time() {
        let mut da = DecayAccumF64::new(10.0).unwrap();
        da.accumulate(0.0, 100.0);

        let s = da.score(10.0); // one half-life
        assert!((s - 50.0).abs() < 1.0, "should be ~50 after one half-life, got {s}");

        let s = da.score(20.0); // two half-lives
        assert!((s - 25.0).abs() < 1.0, "should be ~25 after two half-lives, got {s}");
    }

    #[test]
    fn lazy_evaluation() {
        let mut da = DecayAccumF64::new(10.0).unwrap();
        da.accumulate(0.0, 100.0);
        // No work done between calls
        da.accumulate(5.0, 50.0); // decays 100 by 5 time units, adds 50

        let s = da.score(5.0);
        // After 5 units: 100 * exp(-ln2/10 * 5) + 50 ≈ 100 * 0.707 + 50 ≈ 120.7
        assert!(s > 100.0 && s < 130.0, "score should be ~120, got {s}");
    }

    #[test]
    fn reset() {
        let mut da = DecayAccumF64::new(10.0).unwrap();
        da.accumulate(0.0, 100.0);
        da.reset();
        let s = da.score(0.0);
        assert!((s).abs() < 1e-10);
    }

    #[test]
    fn rejects_zero_half_life() {
        assert!(matches!(DecayAccumF64::new(0.0), Err(crate::ConfigError::Invalid(_))));
    }
}
