//! Clock abstractions for event-driven runtimes.
//!
//! [`Clock`] is a pure data struct registered as a World resource. Handlers
//! read it via `Res<Clock>`. Sync sources ([`RealtimeClock`], [`TestClock`],
//! [`HistoricalClock`]) write into it once per poll loop iteration.
//!
//! The calibration design for [`RealtimeClock`] is inspired by Agrona's
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

// =============================================================================
// Clock — the World resource
// =============================================================================

/// The current time — registered as a World resource.
///
/// Synced once per poll loop iteration by a sync source. Handlers read
/// via `Res<Clock>`. Fields are public for direct access.
///
/// # Example
///
/// ```ignore
/// fn on_event(clock: Res<Clock>, event: SomeEvent) {
///     let timestamp = clock.unix_nanos;
///     let when = clock.instant;
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    /// Monotonic instant from this poll iteration.
    pub instant: Instant,
    /// UTC nanoseconds since Unix epoch (1970-01-01 00:00:00 UTC).
    pub unix_nanos: i128,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            instant: Instant::now(),
            unix_nanos: 0,
        }
    }
}

// =============================================================================
// RealtimeClock — sync source
// =============================================================================

/// Default calibration accuracy threshold.
///
/// Release: 250ns — matches Agrona's `DEFAULT_MEASUREMENT_THRESHOLD_NS`.
/// Debug: 1μs — relaxed for unoptimized builds.
#[cfg(debug_assertions)]
const DEFAULT_THRESHOLD: Duration = Duration::from_micros(1);
#[cfg(not(debug_assertions))]
const DEFAULT_THRESHOLD: Duration = Duration::from_nanos(250);

/// Minimum allowed threshold.
#[cfg(debug_assertions)]
const MIN_THRESHOLD: Duration = Duration::from_micros(1);
#[cfg(not(debug_assertions))]
const MIN_THRESHOLD: Duration = Duration::from_nanos(100);

/// Production sync source — calibrated UTC from monotonic clock.
///
/// Uses Agrona-style offset calibration: brackets `SystemTime::now()` with
/// two `Instant::now()` reads, takes the midpoint. Periodically resyncs
/// to correct for NTP drift.
///
/// # Usage
///
/// ```ignore
/// let mut realtime = RealtimeClock::default();
/// let mut clock = Clock::default();
///
/// // Poll loop:
/// let now = Instant::now();
/// realtime.sync(&mut clock, now);
/// ```
pub struct RealtimeClock {
    base_instant: Instant,
    base_nanos: i128,
    last_resync: Instant,
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

    /// Syncs the clock resource with the current time.
    ///
    /// Computes UTC nanos from the calibrated offset and writes both
    /// the instant and nanos into the `Clock` resource. Triggers resync
    /// if the resync interval has elapsed.
    #[inline]
    pub fn sync(&mut self, clock: &mut Clock, now: Instant) {
        if now.saturating_duration_since(self.last_resync) >= self.resync_interval {
            self.resync_at(now);
        }

        let elapsed = now.saturating_duration_since(self.base_instant);
        clock.instant = now;
        clock.unix_nanos = self.base_nanos + elapsed.as_nanos() as i128;
    }

    /// Force recalibration.
    pub fn resync(&mut self) {
        self.resync_at(Instant::now());
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

    fn resync_at(&mut self, now: Instant) {
        let (best_instant, base_nanos, gap) =
            Self::calibrate(self.threshold, self.max_retries);
        let adjustment = now.saturating_duration_since(best_instant);
        self.base_instant = now;
        self.base_nanos = base_nanos + adjustment.as_nanos() as i128;
        self.calibration_gap = gap;
        self.accurate = gap <= self.threshold;
        self.last_resync = now;
    }

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

impl std::fmt::Debug for RealtimeClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealtimeClock")
            .field("calibration_gap", &self.calibration_gap)
            .field("accurate", &self.accurate)
            .field("resync_interval", &self.resync_interval)
            .finish()
    }
}

impl RealtimeClockBuilder {
    /// Target accuracy threshold. Clamped to platform minimum (100ns release, 1μs debug).
    ///
    /// Release default: 250ns. Debug default: 1μs.
    #[must_use]
    pub fn threshold(mut self, threshold: Duration) -> Self {
        self.threshold = threshold.max(MIN_THRESHOLD);
        self
    }

    /// Maximum calibration attempts. Default: 20.
    #[must_use]
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// Resync interval. Default: 1 hour.
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
// TestClock — sync source
// =============================================================================

/// Deterministic sync source for testing — manually controlled.
///
/// Use `advance()` to move time forward, then `sync()` to write into the
/// `Clock` resource. For simple tests, write `Clock` fields directly.
///
/// # Usage
///
/// ```ignore
/// let mut test_clock = TestClock::new();
/// let mut clock = Clock::default();
///
/// test_clock.advance(Duration::from_secs(1));
/// test_clock.sync(&mut clock);
/// assert_eq!(clock.unix_nanos, 1_000_000_000);
/// ```
#[derive(Debug, Clone)]
pub struct TestClock {
    elapsed: Duration,
    base_nanos: i128,
    base_instant: Instant,
}

impl TestClock {
    /// Creates a new test clock starting at epoch (nanos = 0).
    #[must_use]
    pub fn new() -> Self {
        Self {
            elapsed: Duration::ZERO,
            base_nanos: 0,
            base_instant: Instant::now(),
        }
    }

    /// Creates a new test clock starting at the given UTC nanos.
    #[must_use]
    pub fn starting_at(nanos: i128) -> Self {
        Self {
            elapsed: Duration::ZERO,
            base_nanos: nanos,
            base_instant: Instant::now(),
        }
    }

    /// Syncs the clock resource with the test clock's current state.
    #[inline]
    pub fn sync(&self, clock: &mut Clock) {
        clock.instant = self.base_instant + self.elapsed;
        clock.unix_nanos = self.base_nanos + self.elapsed.as_nanos() as i128;
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

// =============================================================================
// HistoricalClock — sync source
// =============================================================================

/// Replay sync source — auto-advances by step on each `sync()`.
///
/// Starts at `start_nanos`, advances by `step` on every `sync()` call,
/// and becomes exhausted when it reaches `end_nanos`.
///
/// # Usage
///
/// ```ignore
/// let mut historical = HistoricalClock::new(start, end, step).unwrap();
/// let mut clock = Clock::default();
///
/// while !historical.is_exhausted() {
///     historical.sync(&mut clock);
///     // process at clock.unix_nanos
/// }
/// ```
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

    /// Syncs the clock resource and auto-advances the replay position.
    ///
    /// Writes current position into `clock`, then advances by one step.
    /// After exhaustion, writes `end_nanos` and stops advancing.
    #[inline]
    pub fn sync(&mut self, clock: &mut Clock) {
        let elapsed = (self.current_nanos - self.start_nanos).max(0);
        let elapsed_nanos = u64::try_from(elapsed).unwrap_or(u64::MAX);
        clock.instant = self.base_instant + Duration::from_nanos(elapsed_nanos);
        clock.unix_nanos = self.current_nanos;

        if !self.exhausted {
            self.current_nanos += self.step_nanos;
            if self.current_nanos >= self.end_nanos {
                self.current_nanos = self.end_nanos;
                self.exhausted = true;
            }
        }
    }

    /// Whether the replay has reached `end_nanos`.
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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Clock struct
    // =========================================================================

    #[test]
    fn clock_default() {
        let clock = Clock::default();
        assert_eq!(clock.unix_nanos, 0);
    }

    #[test]
    fn clock_fields_writable() {
        let mut clock = Clock::default();
        clock.unix_nanos = 42;
        clock.instant = Instant::now();
        assert_eq!(clock.unix_nanos, 42);
    }

    // =========================================================================
    // RealtimeClock
    // =========================================================================

    #[test]
    fn realtime_sync_produces_valid_nanos() {
        let mut rt = RealtimeClock::default();
        let mut clock = Clock::default();
        rt.sync(&mut clock, Instant::now());

        let expected = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i128;
        let diff = (clock.unix_nanos - expected).unsigned_abs();
        assert!(diff < 1_000_000_000, "nanos should be close to now, diff={diff}ns");
    }

    #[test]
    fn realtime_sync_sets_instant() {
        let mut rt = RealtimeClock::default();
        let mut clock = Clock::default();
        let now = Instant::now();
        rt.sync(&mut clock, now);
        assert_eq!(clock.instant, now);
    }

    #[test]
    fn realtime_nanos_increase() {
        let mut rt = RealtimeClock::default();
        let mut clock = Clock::default();
        rt.sync(&mut clock, Instant::now());
        let n1 = clock.unix_nanos;
        std::thread::sleep(Duration::from_millis(1));
        rt.sync(&mut clock, Instant::now());
        assert!(clock.unix_nanos > n1);
    }

    #[test]
    fn realtime_builder() {
        let rt = RealtimeClock::builder()
            .threshold(Duration::from_micros(10))
            .max_retries(5)
            .resync_interval(Duration::from_secs(60))
            .build()
            .unwrap();
        assert!(rt.calibration_gap() < Duration::from_secs(1));
    }

    #[test]
    fn realtime_resync_no_panic() {
        let mut rt = RealtimeClock::builder()
            .resync_interval(Duration::ZERO)
            .build()
            .unwrap();
        let mut clock = Clock::default();
        let now = Instant::now();
        rt.sync(&mut clock, now);
        assert!(clock.unix_nanos > 0);
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
        let tc = TestClock::new();
        let mut clock = Clock::default();
        tc.sync(&mut clock);
        assert_eq!(clock.unix_nanos, 0);
    }

    #[test]
    fn test_clock_starting_at() {
        let tc = TestClock::starting_at(1_000_000_000);
        let mut clock = Clock::default();
        tc.sync(&mut clock);
        assert_eq!(clock.unix_nanos, 1_000_000_000);
    }

    #[test]
    fn test_clock_advance() {
        let mut tc = TestClock::new();
        let mut clock = Clock::default();
        tc.advance(Duration::from_millis(100));
        tc.sync(&mut clock);
        assert_eq!(clock.unix_nanos, 100_000_000);
    }

    #[test]
    fn test_clock_set_nanos() {
        let mut tc = TestClock::new();
        let mut clock = Clock::default();
        tc.set_nanos(42);
        tc.sync(&mut clock);
        assert_eq!(clock.unix_nanos, 42);
    }

    #[test]
    fn test_clock_reset() {
        let mut tc = TestClock::new();
        let mut clock = Clock::default();
        tc.advance(Duration::from_secs(10));
        tc.reset();
        tc.sync(&mut clock);
        assert_eq!(clock.unix_nanos, 0);
    }

    #[test]
    fn test_clock_instant_advances() {
        let mut tc = TestClock::new();
        let mut clock = Clock::default();
        tc.sync(&mut clock);
        let i1 = clock.instant;
        tc.advance(Duration::from_millis(100));
        tc.sync(&mut clock);
        assert_eq!(clock.instant.duration_since(i1), Duration::from_millis(100));
    }

    #[test]
    fn test_clock_direct_write() {
        let mut clock = Clock::default();
        clock.unix_nanos = 999;
        assert_eq!(clock.unix_nanos, 999);
    }

    // =========================================================================
    // HistoricalClock
    // =========================================================================

    #[test]
    fn historical_starts_at_start() {
        let hc = HistoricalClock::new(1000, 2000, Duration::from_nanos(100)).unwrap();
        assert_eq!(hc.current_nanos(), 1000);
        assert!(!hc.is_exhausted());
    }

    #[test]
    fn historical_sync_writes_and_advances() {
        let mut hc = HistoricalClock::new(0, 1000, Duration::from_nanos(100)).unwrap();
        let mut clock = Clock::default();

        hc.sync(&mut clock);
        assert_eq!(clock.unix_nanos, 0); // writes current BEFORE advancing
        assert_eq!(hc.current_nanos(), 100); // advanced after sync

        hc.sync(&mut clock);
        assert_eq!(clock.unix_nanos, 100);
        assert_eq!(hc.current_nanos(), 200);
    }

    #[test]
    fn historical_exhausts() {
        let mut hc = HistoricalClock::new(0, 200, Duration::from_nanos(100)).unwrap();
        let mut clock = Clock::default();

        hc.sync(&mut clock); // writes 0, advances to 100
        hc.sync(&mut clock); // writes 100, advances to 200 → exhausted

        assert!(hc.is_exhausted());
        assert_eq!(hc.current_nanos(), 200);

        hc.sync(&mut clock); // writes 200, no further advance
        assert_eq!(clock.unix_nanos, 200);
    }

    #[test]
    fn historical_reset() {
        let mut hc = HistoricalClock::new(100, 500, Duration::from_nanos(100)).unwrap();
        let mut clock = Clock::default();
        for _ in 0..10 {
            hc.sync(&mut clock);
        }
        assert!(hc.is_exhausted());
        hc.reset();
        assert!(!hc.is_exhausted());
        assert_eq!(hc.current_nanos(), 100);
    }

    #[test]
    fn historical_rejects_bad_config() {
        assert!(HistoricalClock::new(1000, 1000, Duration::from_nanos(100)).is_err());
        assert!(HistoricalClock::new(2000, 1000, Duration::from_nanos(100)).is_err());
        assert!(HistoricalClock::new(0, 1000, Duration::ZERO).is_err());
    }
}
