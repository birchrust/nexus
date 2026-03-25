//! Integration tests: realistic rate limiting scenarios and edge cases.

use std::time::{Duration, Instant};

use nexus_rate::{ConfigError, local};

// =============================================================================
// Realistic scenarios
// =============================================================================

#[test]
fn binance_style_sliding_window() {
    let start = Instant::now();
    // 1200 requests per 60s, 10 sub-windows (6s each)
    // Using nanos = milliseconds * 1_000_000
    let mut sw = local::SlidingWindow::builder()
        .window(Duration::from_millis(60_000))
        .sub_windows(10)
        .limit(1200)
        .now(start)
        .build()
        .unwrap();

    // Steady rate just under limit: 1200 requests spread over 60s
    // = 20 per second = 1 per 50ms
    for i in 0..1200u64 {
        let now = start + Duration::from_millis(i * 50);
        assert!(
            sw.try_acquire(1, now),
            "request {i} at {}ms should be allowed",
            i * 50
        );
    }
    assert_eq!(sw.count(), 1200);

    // One more at the end of the window — should be rejected
    assert!(
        !sw.try_acquire(1, start + Duration::from_millis(59_999)),
        "should be at limit"
    );

    // After the window rolls, capacity frees up
    assert!(
        sw.try_acquire(1, start + Duration::from_millis(66_001)),
        "should be allowed after first sub-window expires"
    );
}

#[test]
fn multi_rate_composition() {
    let start = Instant::now();
    // Short-term: GCRA at 10 per 1000ns (burst 5)
    // Long-term: SlidingWindow at 100 per 60_000ns
    let mut gcra = local::Gcra::builder()
        .rate(10)
        .period(Duration::from_nanos(1000))
        .burst(5)
        .now(start)
        .build()
        .unwrap();

    let mut sw = local::SlidingWindow::builder()
        .window(Duration::from_nanos(60_000))
        .sub_windows(10)
        .limit(100)
        .now(start)
        .build()
        .unwrap();

    let mut total_allowed = 0u64;
    let mut total_rejected_gcra = 0u64;

    // Send at 1 request per 50ns for 10000ns
    // Check GCRA first (short-term), then SW (long-term)
    for i in 0..200u64 {
        let now = start + Duration::from_nanos(i * 50);
        let gcra_ok = gcra.try_acquire(1, now);
        if !gcra_ok {
            total_rejected_gcra += 1;
            continue;
        }
        if sw.try_acquire(1, now) {
            total_allowed += 1;
        }
    }

    // GCRA should have rejected some (rate = 10/1000ns, we send 20/1000ns)
    assert!(total_rejected_gcra > 0, "GCRA should reject some");
    // SW at 100/60000ns — allowed requests capped
    assert!(total_allowed <= 100, "SW should cap at 100");
}

#[test]
fn weighted_exchange_scenario() {
    let start = Instant::now();
    // Exchange rate limit: 50 weight per second, burst of 10 extra
    // emission_interval = 1000/50 = 20, tau = 20 * (10+1) = 220
    // Burst capacity = tau / emission_interval = 11 units of weight-1
    let mut gcra = local::Gcra::builder()
        .rate(50)
        .period(Duration::from_nanos(1000))
        .burst(10)
        .now(start)
        .build()
        .unwrap();

    // Send weighted requests: cancel=1, new=2, amend=3
    // TAT tracks: 0, 20, 60, 120, 160, 180 → all within tau=220
    let mut total_weight = 0u64;
    let ops = [
        (1u64, "cancel"),
        (2, "new"),
        (3, "amend"),
        (2, "new"),
        (1, "cancel"),
    ];
    for &(weight, _op) in &ops {
        assert!(gcra.try_acquire(weight, start));
        total_weight += weight;
    }
    assert_eq!(total_weight, 9); // TAT = 9 * 20 = 180

    // Next amend (weight 3): TAT = 180 + 60 = 240. Excess = 240, tau = 220. REJECTED.
    assert!(!gcra.try_acquire(3, start), "should exceed burst capacity");

    // But a lighter request (weight 1) fits: TAT = 180 + 20 = 200. Excess = 200 <= 220.
    assert!(gcra.try_acquire(1, start));
    // And another: TAT = 200 + 20 = 220. Excess = 220 <= 220. Exactly at limit.
    assert!(gcra.try_acquire(1, start));
    // One more: TAT = 220 + 20 = 240. Excess = 240 > 220. REJECTED.
    assert!(!gcra.try_acquire(1, start));
}

// =============================================================================
// Reconfigure validation
// =============================================================================

#[test]
fn gcra_reconfigure_zero_rate() {
    let mut g = local::Gcra::builder()
        .rate(10)
        .period(Duration::from_nanos(1000))
        .burst(0)
        .build()
        .unwrap();
    let result = g.reconfigure(0, Duration::from_nanos(1000), 0);
    assert!(matches!(result, Err(ConfigError::Invalid(_))));
}

#[test]
fn gcra_reconfigure_zero_period() {
    let mut g = local::Gcra::builder()
        .rate(10)
        .period(Duration::from_nanos(1000))
        .burst(0)
        .build()
        .unwrap();
    let result = g.reconfigure(10, Duration::ZERO, 0);
    assert!(matches!(result, Err(ConfigError::Invalid(_))));
}

#[test]
fn token_bucket_reconfigure_zero_rate() {
    let start = Instant::now();
    let mut tb = local::TokenBucket::builder()
        .rate(10)
        .period(Duration::from_nanos(1000))
        .burst(10)
        .now(start)
        .build()
        .unwrap();
    let result = tb.reconfigure(0, Duration::from_nanos(1000), 10);
    assert!(matches!(result, Err(ConfigError::Invalid(_))));
}

#[test]
fn sliding_window_reconfigure_zero_limit() {
    let start = Instant::now();
    let mut sw = local::SlidingWindow::builder()
        .window(Duration::from_nanos(1000))
        .sub_windows(10)
        .limit(100)
        .now(start)
        .build()
        .unwrap();
    let result = sw.reconfigure(0);
    assert!(matches!(result, Err(ConfigError::Invalid(_))));
}

#[test]
fn gcra_build_rate_exceeds_period() {
    // rate=1000, period=100ns → emission_interval = 0
    let result = local::Gcra::builder()
        .rate(1000)
        .period(Duration::from_nanos(100))
        .build();
    assert!(
        matches!(result, Err(ConfigError::Invalid(_))),
        "rate > period should fail (emission_interval = 0)"
    );
}

// =============================================================================
// Overflow defense
// =============================================================================

#[test]
fn gcra_huge_burst_saturates() {
    let start = Instant::now();
    let mut g = local::Gcra::builder()
        .rate(1)
        .period(Duration::from_nanos(1000))
        .burst(u64::MAX - 1)
        .now(start)
        .build()
        .unwrap();
    // tau should be saturated, not wrapped
    // The limiter should still work (just very permissive)
    assert!(g.try_acquire(1, start));
}

#[test]
fn token_bucket_large_timestamp() {
    let start = Instant::now();
    let mut tb = local::TokenBucket::builder()
        .rate(10)
        .period(Duration::from_nanos(1000))
        .burst(100)
        .now(start)
        .build()
        .unwrap();

    // Advance by 1000 nanos — should produce 10 tokens
    let now = start + Duration::from_nanos(1000);
    assert!(tb.try_acquire(10, now));
    // Shouldn't wrap or panic
    assert!(!tb.try_acquire(1, now));
}

#[test]
fn sliding_window_huge_cost_rejected() {
    let start = Instant::now();
    let mut sw = local::SlidingWindow::builder()
        .window(Duration::from_nanos(1000))
        .sub_windows(10)
        .limit(100)
        .now(start)
        .build()
        .unwrap();

    // cost = u64::MAX should be rejected, not wrapped to allow
    assert!(
        !sw.try_acquire(u64::MAX, start),
        "huge cost should be rejected"
    );
    assert_eq!(sw.count(), 0, "nothing should have been recorded");
}

#[test]
fn token_bucket_ceiling_division() {
    let start = Instant::now();
    // Verify that consuming always advances zero_time by at least 1
    // even with unfavorable rate/period ratios
    let mut tb = local::TokenBucket::builder()
        .rate(3)
        .period(Duration::from_nanos(10))
        .burst(100)
        .now(start)
        .build()
        .unwrap();

    // At time 100ns: available = min(100 * 3 / 10, 100) = min(30, 100) = 30
    assert!(tb.try_acquire(1, start + Duration::from_nanos(100)));
    // Ceiling division: consume_ticks = ceil(1 * 10 / 3) = ceil(3.33) = 4
    // zero_time should advance by 4, not 3
    // If it advanced by 3 (truncation), we'd get 1 extra token over time
    let available_after = tb.available(start + Duration::from_nanos(100));
    assert!(
        available_after < 30,
        "should have consumed tokens, got {available_after}"
    );
}
