//! Clock abstractions for event-driven runtimes.
//!
//! Three implementations:
//! - [`RealtimeClock`] — production, calibrated UTC from monotonic clock
//! - [`TestClock`] — deterministic, manually controlled
//! - [`HistoricalClock`] — replay, auto-advances per stamp
//!
//! The [`RealtimeClock`] calibration design is inspired by Agrona's
//! [`OffsetEpochNanoClock`](https://github.com/real-logic/agrona)
//! (Real Logic), which uses bracketed sampling with midpoint estimation
//! to achieve high-accuracy monotonic-to-UTC offset calibration.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Configuration error from clock construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A parameter value is invalid.
    Invalid(&'static str),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(msg) => write!(f, "clock configuration error: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Clock trait for stamping time in poll loops.
///
/// Called once per poll iteration with the user's `Instant`. Computes and
/// caches UTC nanoseconds from a calibrated offset. Handlers read the
/// cached values — no syscall on the hot path.
pub trait Clock {
    /// Stamp the current time. Called once per poll loop iteration.
    fn stamp(&mut self, now: Instant);

    /// Returns the cached UTC nanoseconds from the last stamp.
    fn nanos(&self) -> i128;

    /// Returns the Instant from the last stamp.
    fn instant(&self) -> Instant;
}

// =============================================================================
// RealtimeClock
// =============================================================================

/// Default calibration accuracy threshold.
///
/// Release: 250ns — matches Agrona's `DEFAULT_MEASUREMENT_THRESHOLD_NS`.
/// Achievable on x86-64 Linux with vDSO in <5 retries.
/// Debug: 1μs — relaxed for unoptimized builds and instrumentation.
#[cfg(debug_assertions)]
const DEFAULT_THRESHOLD: Duration = Duration::from_micros(1);
#[cfg(not(debug_assertions))]
const DEFAULT_THRESHOLD: Duration = Duration::from_nanos(250);

/// Minimum allowed threshold.
#[cfg(debug_assertions)]
const MIN_THRESHOLD: Duration = Duration::from_micros(1);
#[cfg(not(debug_assertions))]
const MIN_THRESHOLD: Duration = Duration::from_nanos(100);

/// Production clock — calibrated UTC from monotonic clock.
///
/// Uses Agrona-style offset calibration: brackets `SystemTime::now()` with
/// two `Instant::now()` reads, takes the midpoint for accuracy. Periodically
/// resyncs to correct for NTP drift.
///
/// On each `stamp()`, UTC nanos are computed from the monotonic instant
/// plus the calibrated offset — no syscall on the hot path.
pub struct RealtimeClock {
    base_instant: Instant,
    base_nanos: i128,
    last_resync: Instant,
    cached_instant: Instant,
    cached_nanos: i128,
    resync_interval: Duration,
    threshold: Duration,
    max_retries: u32,
    calibration_gap: Duration,
    accurate: bool,
}

/// Builder for [`RealtimeClock`].
pub struct RealtimeClockBuilder {
    threshold: Duration,
    max_retries: u32,
    resync_interval: Duration,
}

impl RealtimeClock {
    /// Creates a builder with sensible defaults.
    #[must_use]
    pub fn builder() -> RealtimeClockBuilder {
        RealtimeClockBuilder::default()
    }

    /// Force recalibration of the monotonic-to-UTC offset.
    pub fn resync(&mut self) {
        self.resync_at(Instant::now());
    }

    /// Internal resync anchored to the caller's instant.
    ///
    /// Anchors `base_instant` to `now` so that subsequent
    /// `now.duration_since(self.base_instant)` in `stamp()` cannot panic.
    fn resync_at(&mut self, now: Instant) {
        let (_, base_nanos, gap) =
            Self::calibrate(self.threshold, self.max_retries);
        self.base_instant = now;
        self.base_nanos = base_nanos;
        self.calibration_gap = gap;
        self.accurate = gap <= self.threshold;
        self.last_resync = now;
    }

    /// Whether calibration achieved the configured threshold.
    #[inline]
    #[must_use]
    pub fn is_accurate(&self) -> bool {
        self.accurate
    }

    /// Best measurement gap achieved during calibration.
    #[inline]
    #[must_use]
    pub fn calibration_gap(&self) -> Duration {
        self.calibration_gap
    }

    /// Calibrate the offset between monotonic and wall clock.
    fn calibrate(threshold: Duration, max_retries: u32) -> (Instant, i128, Duration) {
        let mut best_gap = Duration::MAX;
        let mut best_instant = Instant::now();
        let mut best_nanos = 0i128;

        for _ in 0..max_retries {
            let before = Instant::now();
            let wall = SystemTime::now();
            let after = Instant::now();

            let gap = after.duration_since(before);
            if gap < best_gap {
                best_gap = gap;
                best_instant = before + gap / 2;
                best_nanos = match wall.duration_since(UNIX_EPOCH) {
                    Ok(d) => d.as_nanos() as i128,
                    // Pre-epoch: represent as negative nanos
                    Err(e) => -(e.duration().as_nanos() as i128),
                };
            }

            if gap <= threshold {
                break;
            }
        }

        (best_instant, best_nanos, best_gap)
    }
}

impl Default for RealtimeClock {
    fn default() -> Self {
        Self::builder().build().expect("default RealtimeClock config is always valid")
    }
}

impl Clock for RealtimeClock {
    #[inline]
    fn stamp(&mut self, now: Instant) {
        if now.duration_since(self.last_resync) >= self.resync_interval {
            self.resync_at(now);
        }

        let elapsed = now.duration_since(self.base_instant);
        self.cached_instant = now;
        self.cached_nanos = self.base_nanos + elapsed.as_nanos() as i128;
    }

    #[inline]
    fn nanos(&self) -> i128 {
        self.cached_nanos
    }

    #[inline]
    fn instant(&self) -> Instant {
        self.cached_instant
    }
}

impl std::fmt::Debug for RealtimeClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealtimeClock")
            .field("cached_nanos", &self.cached_nanos)
            .field("calibration_gap", &self.calibration_gap)
            .field("accurate", &self.accurate)
            .field("resync_interval", &self.resync_interval)
            .finish()
    }
}

// -- Builder --

impl RealtimeClockBuilder {
    /// Target accuracy threshold. Calibration stops early if the bracket
    /// gap is under this value. Clamped to platform minimum (100ns release,
    /// 1μs debug).
    ///
    /// Release default: 250ns. Debug default: 1μs.
    #[must_use]
    pub fn threshold(mut self, threshold: Duration) -> Self {
        self.threshold = threshold.max(MIN_THRESHOLD);
        self
    }

    /// Maximum calibration attempts per calibration cycle.
    /// Default: 20.
    #[must_use]
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// How often to recalibrate. Handles NTP drift.
    /// Default: 1 hour.
    #[must_use]
    pub fn resync_interval(mut self, interval: Duration) -> Self {
        self.resync_interval = interval;
        self
    }

    /// Builds and calibrates the clock.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::Invalid` if `max_retries` is 0.
    ///
    /// Calibration always uses best-effort — check [`RealtimeClock::is_accurate()`]
    /// to see if the threshold was achieved.
    pub fn build(self) -> Result<RealtimeClock, ConfigError> {
        if self.max_retries == 0 {
            return Err(ConfigError::Invalid("max_retries must be > 0"));
        }

        let (base_instant, base_nanos, gap) =
            RealtimeClock::calibrate(self.threshold, self.max_retries);
        let accurate = gap <= self.threshold;

        Ok(RealtimeClock {
            base_instant,
            base_nanos,
            last_resync: base_instant,
            cached_instant: base_instant,
            cached_nanos: base_nanos,
            resync_interval: self.resync_interval,
            threshold: self.threshold,
            max_retries: self.max_retries,
            calibration_gap: gap,
            accurate,
        })
    }
}

impl Default for RealtimeClockBuilder {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_THRESHOLD,
            max_retries: 20,
            resync_interval: Duration::from_secs(3600),
        }
    }
}

impl std::fmt::Debug for RealtimeClockBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealtimeClockBuilder")
            .field("threshold", &self.threshold)
            .field("max_retries", &self.max_retries)
            .field("resync_interval", &self.resync_interval)
            .finish()
    }
}

// =============================================================================
// TestClock
// =============================================================================

/// Deterministic clock for testing — manually controlled.
///
/// Time does not advance automatically. Use `advance()` or `set_elapsed()`
/// to control time progression. `stamp()` stores the passed Instant but
/// does NOT auto-advance the elapsed time.
///
/// Can be registered as a World resource for handler access via
/// `Res<TestClock>` / `ResMut<TestClock>`.
#[derive(Debug, Clone)]
pub struct TestClock {
    elapsed: Duration,
    base_nanos: i128,
    cached_instant: Instant,
}

impl TestClock {
    /// Creates a new test clock starting at epoch (nanos = 0).
    #[must_use]
    pub fn new() -> Self {
        Self {
            elapsed: Duration::ZERO,
            base_nanos: 0,
            cached_instant: Instant::now(),
        }
    }

    /// Creates a new test clock starting at the given UTC nanos.
    #[must_use]
    pub fn starting_at(nanos: i128) -> Self {
        Self {
            elapsed: Duration::ZERO,
            base_nanos: nanos,
            cached_instant: Instant::now(),
        }
    }

    /// Advances time by the given duration.
    #[inline]
    pub fn advance(&mut self, duration: Duration) {
        self.elapsed += duration;
    }

    /// Sets the elapsed time to an exact value.
    #[inline]
    pub fn set_elapsed(&mut self, elapsed: Duration) {
        self.elapsed = elapsed;
    }

    /// Sets the UTC nanos directly (resets elapsed to zero).
    #[inline]
    pub fn set_nanos(&mut self, nanos: i128) {
        self.base_nanos = nanos;
        self.elapsed = Duration::ZERO;
    }

    /// Returns the current elapsed duration.
    #[inline]
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Resets to zero elapsed, keeping the base nanos.
    #[inline]
    pub fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for TestClock {
    #[inline]
    fn stamp(&mut self, now: Instant) {
        // Store the passed Instant for consistency with callers who
        // pass clock.instant() for deterministic testing.
        self.cached_instant = now;
    }

    #[inline]
    fn nanos(&self) -> i128 {
        self.base_nanos + self.elapsed.as_nanos() as i128
    }

    #[inline]
    fn instant(&self) -> Instant {
        self.cached_instant
    }
}

// =============================================================================
// HistoricalClock
// =============================================================================

/// Replay clock — auto-advances by step on each `stamp()`.
///
/// Used for backtesting and replay. Starts at `start_nanos`, advances by
/// `step` on every `stamp()` call, and becomes exhausted when it reaches
/// `end_nanos`. Ignores the passed Instant — stores a synthetic instant
/// derived from the replay position.
#[derive(Debug, Clone)]
pub struct HistoricalClock {
    start_nanos: i128,
    end_nanos: i128,
    current_nanos: i128,
    step_nanos: i128,
    base_instant: Instant,
    exhausted: bool,
}

impl HistoricalClock {
    /// Creates a new historical clock.
    ///
    /// # Arguments
    /// - `start_nanos` — UTC nanos at replay start
    /// - `end_nanos` — UTC nanos at replay end
    /// - `step` — duration to advance per `stamp()` call
    ///
    /// # Errors
    ///
    /// Returns `ConfigError::Invalid` if `start_nanos >= end_nanos` or
    /// `step` is zero.
    pub fn new(start_nanos: i128, end_nanos: i128, step: Duration) -> Result<Self, ConfigError> {
        if start_nanos >= end_nanos {
            return Err(ConfigError::Invalid("start_nanos must be < end_nanos"));
        }
        if step.is_zero() {
            return Err(ConfigError::Invalid("step must be > 0"));
        }

        Ok(Self {
            start_nanos,
            end_nanos,
            current_nanos: start_nanos,
            step_nanos: step.as_nanos() as i128,
            base_instant: Instant::now(),
            exhausted: false,
        })
    }

    /// Whether the replay has reached or passed `end_nanos`.
    #[inline]
    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Current replay position in UTC nanos.
    #[inline]
    #[must_use]
    pub fn current_nanos(&self) -> i128 {
        self.current_nanos
    }

    /// Resets to start position.
    #[inline]
    pub fn reset(&mut self) {
        self.current_nanos = self.start_nanos;
        self.exhausted = false;
    }
}

impl Clock for HistoricalClock {
    #[inline]
    fn stamp(&mut self, _now: Instant) {
        if self.exhausted {
            return;
        }

        self.current_nanos += self.step_nanos;
        if self.current_nanos >= self.end_nanos {
            self.current_nanos = self.end_nanos;
            self.exhausted = true;
        }
    }

    #[inline]
    fn nanos(&self) -> i128 {
        self.current_nanos
    }

    #[inline]
    fn instant(&self) -> Instant {
        let elapsed_nanos = (self.current_nanos - self.start_nanos).max(0) as u64;
        self.base_instant + Duration::from_nanos(elapsed_nanos)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // RealtimeClock
    // =========================================================================

    #[test]
    fn realtime_stamp_produces_valid_nanos() {
        let mut clock = RealtimeClock::default();
        let now = Instant::now();
        clock.stamp(now);
        let n = clock.nanos();
        // Verify nanos is a reasonable UTC timestamp — compare against current time
        let expected = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i128;
        let diff = (n - expected).unsigned_abs();
        assert!(diff < 1_000_000_000, "nanos should be close to current time, diff={diff}ns");
    }

    #[test]
    fn realtime_instant_matches_stamp() {
        let mut clock = RealtimeClock::default();
        let now = Instant::now();
        clock.stamp(now);
        assert_eq!(clock.instant(), now);
    }

    #[test]
    fn realtime_nanos_increases() {
        let mut clock = RealtimeClock::default();
        clock.stamp(Instant::now());
        let n1 = clock.nanos();
        std::thread::sleep(Duration::from_millis(1));
        clock.stamp(Instant::now());
        let n2 = clock.nanos();
        assert!(n2 > n1, "nanos should increase: {n1} → {n2}");
    }

    #[test]
    fn realtime_builder_custom() {
        let clock = RealtimeClock::builder()
            .threshold(Duration::from_micros(10))
            .max_retries(5)
            .resync_interval(Duration::from_secs(60))
            .build()
            .unwrap();
        assert!(clock.nanos() > 0);
    }

    #[test]
    fn realtime_calibration_gap_reported() {
        let clock = RealtimeClock::default();
        let gap = clock.calibration_gap();
        assert!(gap < Duration::from_secs(1), "gap too large: {gap:?}");
    }

    #[test]
    fn realtime_resync() {
        let mut clock = RealtimeClock::default();
        clock.resync();
        assert!(clock.calibration_gap() < Duration::from_secs(1));
    }

    #[test]
    fn realtime_stamp_after_resync_no_panic() {
        // Capture instant BEFORE resync, then stamp with it after.
        // Previously this panicked because resync set base_instant to
        // a fresh Instant::now() that was after the captured `now`.
        let mut clock = RealtimeClock::builder()
            .resync_interval(Duration::ZERO) // force resync on every stamp
            .build()
            .unwrap();

        let now = Instant::now();
        // This should not panic — resync_at anchors to `now`
        clock.stamp(now);
        assert!(clock.nanos() > 0);
    }

    #[test]
    fn realtime_threshold_clamped_to_minimum() {
        let clock = RealtimeClock::builder()
            .threshold(Duration::from_nanos(1))
            .build()
            .unwrap();
        assert!(clock.nanos() > 0);
    }

    #[test]
    fn realtime_zero_retries_rejected() {
        let result = RealtimeClock::builder().max_retries(0).build();
        assert!(matches!(result, Err(ConfigError::Invalid(_))));
    }

    // =========================================================================
    // TestClock
    // =========================================================================

    #[test]
    fn test_clock_starts_at_zero() {
        let clock = TestClock::new();
        assert_eq!(clock.nanos(), 0);
        assert_eq!(clock.elapsed(), Duration::ZERO);
    }

    #[test]
    fn test_clock_starting_at() {
        let clock = TestClock::starting_at(1_000_000_000);
        assert_eq!(clock.nanos(), 1_000_000_000);
    }

    #[test]
    fn test_clock_advance() {
        let mut clock = TestClock::new();
        clock.advance(Duration::from_millis(100));
        assert_eq!(clock.nanos(), 100_000_000);
        clock.advance(Duration::from_millis(50));
        assert_eq!(clock.nanos(), 150_000_000);
    }

    #[test]
    fn test_clock_set_elapsed() {
        let mut clock = TestClock::new();
        clock.advance(Duration::from_secs(10));
        clock.set_elapsed(Duration::from_secs(5));
        assert_eq!(clock.elapsed(), Duration::from_secs(5));
    }

    #[test]
    fn test_clock_set_nanos() {
        let mut clock = TestClock::new();
        clock.advance(Duration::from_secs(10));
        clock.set_nanos(42);
        assert_eq!(clock.nanos(), 42);
    }

    #[test]
    fn test_clock_reset() {
        let mut clock = TestClock::new();
        clock.advance(Duration::from_secs(10));
        clock.reset();
        assert_eq!(clock.nanos(), 0);
    }

    #[test]
    fn test_clock_stamp_stores_instant() {
        let mut clock = TestClock::new();
        let now = Instant::now();
        clock.stamp(now);
        assert_eq!(clock.instant(), now);
        // But nanos don't change — stamp doesn't advance
        assert_eq!(clock.nanos(), 0);
    }

    #[test]
    fn test_clock_default() {
        let clock = TestClock::default();
        assert_eq!(clock.nanos(), 0);
    }

    // =========================================================================
    // HistoricalClock
    // =========================================================================

    #[test]
    fn historical_starts_at_start() {
        let clock = HistoricalClock::new(1000, 2000, Duration::from_nanos(100)).unwrap();
        assert_eq!(clock.current_nanos(), 1000);
        assert!(!clock.is_exhausted());
    }

    #[test]
    fn historical_advances_on_stamp() {
        let mut clock = HistoricalClock::new(0, 1000, Duration::from_nanos(100)).unwrap();
        clock.stamp(Instant::now());
        assert_eq!(clock.current_nanos(), 100);
        clock.stamp(Instant::now());
        assert_eq!(clock.current_nanos(), 200);
    }

    #[test]
    fn historical_exhausts() {
        let mut clock = HistoricalClock::new(0, 300, Duration::from_nanos(100)).unwrap();
        clock.stamp(Instant::now());
        clock.stamp(Instant::now());
        clock.stamp(Instant::now());
        assert!(clock.is_exhausted());
        assert_eq!(clock.current_nanos(), 300);
        clock.stamp(Instant::now());
        assert_eq!(clock.current_nanos(), 300);
    }

    #[test]
    fn historical_reset() {
        let mut clock = HistoricalClock::new(100, 500, Duration::from_nanos(100)).unwrap();
        for _ in 0..10 {
            clock.stamp(Instant::now());
        }
        assert!(clock.is_exhausted());
        clock.reset();
        assert!(!clock.is_exhausted());
        assert_eq!(clock.current_nanos(), 100);
    }

    #[test]
    fn historical_nanos_via_trait() {
        let mut clock = HistoricalClock::new(1000, 2000, Duration::from_nanos(50)).unwrap();
        clock.stamp(Instant::now());
        assert_eq!(clock.nanos(), 1050);
    }

    #[test]
    fn historical_instant_advances() {
        let mut clock = HistoricalClock::new(0, 1_000_000, Duration::from_nanos(1000)).unwrap();
        let i1 = clock.instant();
        clock.stamp(Instant::now());
        let i2 = clock.instant();
        assert!(i2 > i1);
    }

    #[test]
    fn historical_rejects_start_ge_end() {
        let result = HistoricalClock::new(1000, 1000, Duration::from_nanos(100));
        assert!(matches!(result, Err(ConfigError::Invalid(_))));

        let result = HistoricalClock::new(2000, 1000, Duration::from_nanos(100));
        assert!(matches!(result, Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn historical_rejects_zero_step() {
        let result = HistoricalClock::new(0, 1000, Duration::ZERO);
        assert!(matches!(result, Err(ConfigError::Invalid(_))));
    }

    // =========================================================================
    // Generic over Clock trait
    // =========================================================================

    #[test]
    fn generic_over_clock() {
        fn use_clock(clock: &mut dyn Clock) {
            clock.stamp(Instant::now());
            let _ = clock.nanos();
            let _ = clock.instant();
        }

        let mut rt = RealtimeClock::default();
        let mut tc = TestClock::new();
        let mut hc = HistoricalClock::new(0, 1000, Duration::from_nanos(100)).unwrap();

        use_clock(&mut rt);
        use_clock(&mut tc);
        use_clock(&mut hc);
    }
}
