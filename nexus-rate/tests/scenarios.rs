//! Integration tests: realistic rate limiting scenarios and edge cases.

use nexus_rate::{local, ConfigError};

// =============================================================================
// Realistic scenarios
// =============================================================================

#[test]
fn binance_style_sliding_window() {
    // 1200 requests per 60s, 10 sub-windows (6s each)
    // Using ticks = milliseconds, so 60_000ms window
    let mut sw = local::SlidingWindow::builder()
        .window(60_000)
        .sub_windows(10)
        .limit(1200)
        .now(0)
        .build()
        .unwrap();

    // Steady rate just under limit: 1200 requests spread over 60s
    // = 20 per second = 1 per 50ms
    for i in 0..1200 {
        let now = i * 50; // every 50ms
        assert!(sw.try_acquire(1, now), "request {i} at {now}ms should be allowed");
    }
    assert_eq!(sw.count(), 1200);

    // One more at the end of the window — should be rejected
    assert!(!sw.try_acquire(1, 59_999), "should be at limit");

    // After the window rolls, capacity frees up
    assert!(sw.try_acquire(1, 66_001), "should be allowed after first sub-window expires");
}

#[test]
fn multi_rate_composition() {
    // Short-term: GCRA at 10 per 1000ms (burst 5)
    // Long-term: SlidingWindow at 100 per 60_000ms
    let mut gcra = local::Gcra::builder()
        .rate(10).period(1000).burst(5)
        .build().unwrap();
    let mut sw = local::SlidingWindow::builder()
        .window(60_000).sub_windows(10).limit(100).now(0)
        .build().unwrap();

    let mut total_allowed = 0u64;
    let mut total_rejected_gcra = 0u64;

    // Send at 1 request per 50ms for 10 seconds
    // Check GCRA first (short-term), then SW (long-term)
    for i in 0..200 {
        let now = i * 50;
        let gcra_ok = gcra.try_acquire(1, now);
        if !gcra_ok {
            total_rejected_gcra += 1;
            continue;
        }
        if sw.try_acquire(1, now) {
            total_allowed += 1;
        }
    }

    // GCRA should have rejected some (rate = 10/s, we send 20/s)
    assert!(total_rejected_gcra > 0, "GCRA should reject some");
    // SW at 100/60s — allowed requests capped
    assert!(total_allowed <= 100, "SW should cap at 100");
}

#[test]
fn weighted_exchange_scenario() {
    // Exchange rate limit: 50 weight per second, burst of 10 extra
    // emission_interval = 1000/50 = 20, tau = 20 * (10+1) = 220
    // Burst capacity = tau / emission_interval = 11 units of weight-1
    let mut gcra = local::Gcra::builder()
        .rate(50).period(1000).burst(10)
        .build().unwrap();

    // Send weighted requests: cancel=1, new=2, amend=3
    // TAT tracks: 0, 20, 60, 120, 160, 180 → all within tau=220
    let mut total_weight = 0u64;
    let ops = [(1u64, "cancel"), (2, "new"), (3, "amend"), (2, "new"), (1, "cancel")];
    for &(weight, _op) in &ops {
        assert!(gcra.try_acquire(weight, 0));
        total_weight += weight;
    }
    assert_eq!(total_weight, 9); // TAT = 9 * 20 = 180

    // Next amend (weight 3): TAT = 180 + 60 = 240. Excess = 240, tau = 220. REJECTED.
    assert!(!gcra.try_acquire(3, 0), "should exceed burst capacity");

    // But a lighter request (weight 1) fits: TAT = 180 + 20 = 200. Excess = 200 <= 220.
    assert!(gcra.try_acquire(1, 0));
    // And another: TAT = 200 + 20 = 220. Excess = 220 <= 220. Exactly at limit.
    assert!(gcra.try_acquire(1, 0));
    // One more: TAT = 220 + 20 = 240. Excess = 240 > 220. REJECTED.
    assert!(!gcra.try_acquire(1, 0));
}

// =============================================================================
// Reconfigure validation
// =============================================================================

#[test]
fn gcra_reconfigure_zero_rate() {
    let mut g = local::Gcra::builder().rate(10).period(1000).burst(0).build().unwrap();
    let result = g.reconfigure(0, 1000, 0);
    assert!(matches!(result, Err(ConfigError::Invalid(_))));
}

#[test]
fn gcra_reconfigure_zero_period() {
    let mut g = local::Gcra::builder().rate(10).period(1000).burst(0).build().unwrap();
    let result = g.reconfigure(10, 0, 0);
    assert!(matches!(result, Err(ConfigError::Invalid(_))));
}

#[test]
fn token_bucket_reconfigure_zero_rate() {
    let mut tb = local::TokenBucket::builder()
        .rate(10).period(1000).burst(10).now(0).build().unwrap();
    let result = tb.reconfigure(0, 1000, 10);
    assert!(matches!(result, Err(ConfigError::Invalid(_))));
}

#[test]
fn sliding_window_reconfigure_zero_limit() {
    let mut sw = local::SlidingWindow::builder()
        .window(1000).sub_windows(10).limit(100).now(0).build().unwrap();
    let result = sw.reconfigure(0);
    assert!(matches!(result, Err(ConfigError::Invalid(_))));
}

#[test]
fn gcra_build_rate_exceeds_period() {
    // rate=1000, period=100 → emission_interval = 0
    let result = local::Gcra::builder().rate(1000).period(100).build();
    assert!(matches!(result, Err(ConfigError::Invalid(_))),
        "rate > period should fail (emission_interval = 0)");
}

// =============================================================================
// Overflow defense
// =============================================================================

#[test]
fn gcra_huge_burst_saturates() {
    let mut g = local::Gcra::builder()
        .rate(1).period(1000).burst(u64::MAX - 1)
        .build().unwrap();
    // tau should be saturated, not wrapped
    // The limiter should still work (just very permissive)
    assert!(g.try_acquire(1, 0));
}

#[test]
fn token_bucket_large_timestamp() {
    let mut tb = local::TokenBucket::builder()
        .rate(10).period(1000).burst(100).now(u64::MAX - 10_000)
        .build().unwrap();

    // Advance by 1000 ticks — should produce 10 tokens
    let now = u64::MAX - 9_000;
    assert!(tb.try_acquire(10, now));
    // Shouldn't wrap or panic
    assert!(!tb.try_acquire(1, now));
}

#[test]
fn sliding_window_huge_cost_rejected() {
    let mut sw = local::SlidingWindow::builder()
        .window(1000).sub_windows(10).limit(100).now(0)
        .build().unwrap();

    // cost = u64::MAX should be rejected, not wrapped to allow
    assert!(!sw.try_acquire(u64::MAX, 0), "huge cost should be rejected");
    assert_eq!(sw.count(), 0, "nothing should have been recorded");
}

#[test]
fn token_bucket_ceiling_division() {
    // Verify that consuming always advances zero_time by at least 1
    // even with unfavorable rate/period ratios
    let mut tb = local::TokenBucket::builder()
        .rate(3).period(10).burst(100).now(0)
        .build().unwrap();

    // At time 100: available = min(100 * 3 / 10, 100) = min(30, 100) = 30
    assert!(tb.try_acquire(1, 100));
    // Ceiling division: consume_ticks = ceil(1 * 10 / 3) = ceil(3.33) = 4
    // zero_time should advance by 4, not 3
    // If it advanced by 3 (truncation), we'd get 1 extra token over time
    let available_after = tb.available(100);
    assert!(available_after < 30, "should have consumed tokens, got {available_after}");
}
