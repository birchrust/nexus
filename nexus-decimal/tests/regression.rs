//! Regression tests for 1.0 bug fixes.
//! Each test would have caught the original bug.

use nexus_decimal::Decimal;

type D32 = Decimal<i32, 4>;
type D64 = Decimal<i64, 8>;
type D96 = Decimal<i128, 12>;

// ============================================================================
// Bug #1: percent_of returned SCALE× too large
// ============================================================================

#[test]
fn percent_of_basic() {
    // 100 * 50% = 50
    let result = D64::new(100, 0).percent_of(D64::new(50, 0)).unwrap();
    assert_eq!(result, D64::new(50, 0));
}

#[test]
fn percent_of_100_percent() {
    let result = D64::new(100, 0).percent_of(D64::new(100, 0)).unwrap();
    assert_eq!(result, D64::new(100, 0));
}

#[test]
fn percent_of_zero() {
    let result = D64::new(100, 0).percent_of(D64::ZERO).unwrap();
    assert_eq!(result, D64::ZERO);
}

#[test]
fn percent_of_one_percent() {
    // 1.0 * 1% = 0.01
    let result = D64::new(1, 0).percent_of(D64::new(1, 0)).unwrap();
    assert_eq!(result, D64::new(0, 1_000_000)); // 0.01
}

#[test]
fn percent_of_negative() {
    let result = D64::new(-100, 0).percent_of(D64::new(50, 0)).unwrap();
    assert_eq!(result, D64::new(-50, 0));
}

#[test]
fn percent_of_i32() {
    let result = D32::new(100, 0).percent_of(D32::new(50, 0)).unwrap();
    assert_eq!(result, D32::new(50, 0));
}

// ============================================================================
// Bug #2: mul_wide carry overflow
// ============================================================================

#[test]
fn mul_wide_large_inputs() {
    // D96 mul with values that exercise the wide multiply
    let a = D96::new(1_000_000_000, 0); // 1 billion
    let b = D96::new(1_000_000_000, 0);
    let result = a.checked_mul(b).unwrap();
    // 1e9 * 1e9 = 1e18
    assert_eq!(result.to_integer(), 1_000_000_000_000_000_000);
}

// ============================================================================
// Bug #3: rounding overflow near MIN/MAX
// ============================================================================

#[test]
fn floor_min_does_not_panic() {
    let _ = D64::MIN.floor(); // must not panic or wrap
}

#[test]
fn ceil_max_does_not_panic() {
    let _ = D64::MAX.ceil(); // must not panic or wrap
}

#[test]
fn round_near_max() {
    // MAX with fractional part — rounding should saturate, not overflow
    let _ = D64::MAX.round();
}

#[test]
fn floor_min_i32() {
    let _ = D32::MIN.floor();
}

#[test]
fn ceil_max_i32() {
    let _ = D32::MAX.ceil();
}

#[test]
fn round_dp_near_max() {
    let _ = D64::MAX.round_dp(0);
}

// ============================================================================
// Bug #4: tick rounding overflow → now returns Option
// ============================================================================

#[test]
fn tick_rounding_near_max() {
    let tick = D64::new(1, 0);
    // MAX rounded to tick=1.0 — this used to overflow; test ensures no panic
    let _ = D64::MAX.round_to_tick(tick);
}

#[test]
fn floor_to_tick_returns_option() {
    let tick = D64::new(1, 0);
    let result = D64::new(42, 50_000_000).floor_to_tick(tick);
    assert_eq!(result, Some(D64::new(42, 0)));
}

#[test]
fn ceil_to_tick_returns_option() {
    let tick = D64::new(1, 0);
    let result = D64::new(42, 50_000_000).ceil_to_tick(tick);
    assert_eq!(result, Some(D64::new(43, 0)));
}

// ============================================================================
// Bug #5: saturating_div / wrapping_div panic on zero
// ============================================================================

#[test]
#[should_panic(expected = "division by zero")]
fn saturating_div_zero_panics() {
    let _ = D64::ONE.saturating_div(D64::ZERO);
}

#[test]
#[should_panic(expected = "division by zero")]
fn wrapping_div_zero_panics() {
    let _ = D64::ONE.wrapping_div(D64::ZERO);
}

#[test]
#[should_panic(expected = "division by zero")]
fn saturating_div_zero_panics_i32() {
    let _ = D32::ONE.saturating_div(D32::ZERO);
}

// ============================================================================
// Bug #6: tick=0 panics in release mode
// ============================================================================

#[test]
#[should_panic(expected = "tick must be positive")]
fn round_to_tick_zero_panics() {
    let _ = D64::ONE.round_to_tick(D64::ZERO);
}

#[test]
#[should_panic(expected = "tick must be positive")]
fn floor_to_tick_zero_panics() {
    let _ = D64::ONE.floor_to_tick(D64::ZERO);
}

// ============================================================================
// Bug #7: from_bps i32 truncation → now returns Option
// ============================================================================

#[test]
fn from_bps_i32_basic() {
    let result = D32::from_bps(100);
    assert!(result.is_some());
    assert_eq!(result.unwrap(), D32::new(0, 100)); // 0.01 with SCALE=10000
}

#[test]
fn from_bps_i32_large() {
    // Very large bps that would overflow i32 after scaling
    let result = D32::from_bps(i32::MAX);
    // Should be None (overflow) or Some with correct value, not truncated garbage
    if let Some(d) = result {
        assert!(d.to_raw() > 0);
    }
}
