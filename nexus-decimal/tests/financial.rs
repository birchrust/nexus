//! Financial methods, serde, num-traits, and type conversions.

use nexus_decimal::Decimal;

type D64 = Decimal<i64, 8>;
type D96 = Decimal<i128, 12>;

// ============================================================================
// Financial: midpoint
// ============================================================================

#[test]
fn midpoint_basic() {
    let bid = D64::new(100, 0);
    let ask = D64::new(101, 0);
    assert_eq!(bid.midpoint(ask), D64::new(100, 50_000_000));
}

#[test]
fn midpoint_same() {
    let p = D64::new(42, 0);
    assert_eq!(p.midpoint(p), p);
}

#[test]
fn midpoint_negative() {
    let a = D64::new(-10, 0);
    let b = D64::new(10, 0);
    assert_eq!(a.midpoint(b), D64::ZERO);
}

// ============================================================================
// Financial: spread
// ============================================================================

#[test]
fn spread_basic() {
    let ask = D64::new(101, 0);
    let bid = D64::new(100, 0);
    assert_eq!(ask.spread(bid).unwrap(), D64::new(1, 0));
}

#[test]
fn spread_crossed_returns_none() {
    // spread(self, other) returns None when self < other
    let low = D64::new(99, 0);
    let high = D64::new(100, 0);
    assert!(low.spread(high).is_none());
}

// ============================================================================
// Financial: tick rounding
// ============================================================================

#[test]
fn round_to_tick() {
    let price = D64::new(1, 23_700_000); // 1.237
    let tick = D64::new(0, 5_000_000); // 0.05
    assert_eq!(price.round_to_tick(tick), Some(D64::new(1, 25_000_000))); // 1.25
}

#[test]
fn floor_to_tick() {
    let price = D64::new(1, 23_700_000); // 1.237
    let tick = D64::new(0, 5_000_000); // 0.05
    assert_eq!(price.floor_to_tick(tick), Some(D64::new(1, 20_000_000))); // 1.20
}

#[test]
fn ceil_to_tick() {
    let price = D64::new(1, 23_700_000); // 1.237
    let tick = D64::new(0, 5_000_000); // 0.05
    assert_eq!(price.ceil_to_tick(tick), Some(D64::new(1, 25_000_000))); // 1.25
}

#[test]
fn floor_to_tick_exact() {
    let price = D64::new(1, 25_000_000); // exactly on tick
    let tick = D64::new(0, 5_000_000);
    assert_eq!(price.floor_to_tick(tick), Some(price));
}

// ============================================================================
// Financial: halve, div10, div100
// ============================================================================

#[test]
fn halve_basic() {
    assert_eq!(D64::new(10, 0).halve(), D64::new(5, 0));
    assert_eq!(D64::new(1, 0).halve(), D64::new(0, 50_000_000));
}

#[test]
fn halve_truncates_toward_zero() {
    // 3 / 2 = 1 (truncated)
    let three = D64::from_raw(3);
    assert_eq!(three.halve(), D64::from_raw(1));
    // -3 / 2 = -1 (truncated toward zero, not -2)
    let neg_three = D64::from_raw(-3);
    assert_eq!(neg_three.halve(), D64::from_raw(-1));
}

#[test]
fn div10_basic() {
    assert_eq!(D64::new(100, 0).div10(), D64::new(10, 0));
}

#[test]
fn div100_basic() {
    assert_eq!(D64::new(100, 0).div100(), D64::new(1, 0));
}

// ============================================================================
// Financial: basis points
// ============================================================================

#[test]
fn to_bps() {
    // 0.01 * 10000 = 100 bps
    let one_percent = D64::new(0, 1_000_000); // 0.01
    assert_eq!(one_percent.to_bps().unwrap(), D64::new(100, 0));
}

#[test]
fn from_bps() {
    // 100 bps = 0.01
    let result = D64::from_bps(100).unwrap();
    assert_eq!(result, D64::new(0, 1_000_000));
}

// ============================================================================
// Financial: mul_div
// ============================================================================

#[test]
fn mul_div_basic() {
    // 100 * 3 / 2 = 150
    let a = D64::new(100, 0);
    let b = D64::new(3, 0);
    let c = D64::new(2, 0);
    assert_eq!(a.mul_div(b, c).unwrap(), D64::new(150, 0));
}

#[test]
fn mul_div_zero_divisor() {
    assert!(D64::ONE.mul_div(D64::ONE, D64::ZERO).is_none());
}

// ============================================================================
// Financial: approx_eq, clamp_price
// ============================================================================

#[test]
fn approx_eq_within_tolerance() {
    let a = D64::new(100, 0);
    let b = D64::new(100, 1_000_000); // 100.01
    let tolerance = D64::new(0, 5_000_000); // 0.05
    assert!(a.approx_eq(b, tolerance));
}

#[test]
fn approx_eq_outside_tolerance() {
    let a = D64::new(100, 0);
    let b = D64::new(101, 0);
    let tolerance = D64::new(0, 50_000_000); // 0.5
    assert!(!a.approx_eq(b, tolerance));
}

#[test]
fn clamp_price() {
    let min = D64::new(90, 0);
    let max = D64::new(110, 0);
    assert_eq!(D64::new(100, 0).clamp_price(min, max), D64::new(100, 0));
    assert_eq!(D64::new(80, 0).clamp_price(min, max), min);
    assert_eq!(D64::new(120, 0).clamp_price(min, max), max);
}

// ============================================================================
// Sum and Product iterators
// ============================================================================

#[test]
fn sum_iterator() {
    let values = vec![D64::new(1, 0), D64::new(2, 0), D64::new(3, 0)];
    let total: D64 = values.into_iter().sum();
    assert_eq!(total, D64::new(6, 0));
}

#[test]
fn sum_ref_iterator() {
    let values = [D64::new(1, 0), D64::new(2, 0), D64::new(3, 0)];
    let total: D64 = values.iter().sum();
    assert_eq!(total, D64::new(6, 0));
}

#[test]
fn product_iterator() {
    let values = vec![D64::new(2, 0), D64::new(3, 0), D64::new(4, 0)];
    let total: D64 = values.into_iter().product();
    assert_eq!(total, D64::new(24, 0));
}

// ============================================================================
// TryFrom conversions
// ============================================================================

#[test]
fn try_from_i64() {
    let d: D64 = 42i64.try_into().unwrap();
    assert_eq!(d, D64::new(42, 0));
}

#[cfg(feature = "std")]
#[test]
fn try_from_f64() {
    let d: D64 = 1.5f64.try_into().unwrap();
    assert_eq!(d, D64::new(1, 50_000_000));
}

// ============================================================================
// Rem operator
// ============================================================================

#[test]
fn rem_basic() {
    let a = D64::new(10, 0);
    let b = D64::new(3, 0);
    // 10.0 % 3.0 = 1.0 (on raw values: 1000000000 % 300000000 = 100000000)
    assert_eq!(a % b, D64::new(1, 0));
}

// ============================================================================
// Serde (feature-gated)
// ============================================================================

#[cfg(feature = "serde")]
mod serde_tests {
    use super::*;

    #[test]
    fn json_roundtrip_d64() {
        let original = D64::new(123, 45_678_900);
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"123.456789\"");
        let parsed: D64 = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn json_zero() {
        let json = serde_json::to_string(&D64::ZERO).unwrap();
        assert_eq!(json, "\"0\"");
        let parsed: D64 = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, D64::ZERO);
    }

    #[test]
    fn json_negative() {
        let d = D64::new(-42, 50_000_000);
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, "\"-42.5\"");
        let parsed: D64 = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, d);
    }

    #[test]
    fn json_d96() {
        let original = D96::new(42, 123_000_000_000);
        let json = serde_json::to_string(&original).unwrap();
        let parsed: D96 = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }
}

// ============================================================================
// num-traits (feature-gated)
// ============================================================================

#[cfg(feature = "num-traits")]
mod num_traits_tests {
    use super::*;
    use num_traits::{Bounded, CheckedAdd, FromPrimitive, Num, One, Signed, ToPrimitive, Zero};

    #[test]
    fn zero_one() {
        assert!(D64::zero().is_zero());
        assert!(D64::one().is_one());
    }

    #[test]
    fn bounded() {
        assert_eq!(D64::min_value(), D64::MIN);
        assert_eq!(D64::max_value(), D64::MAX);
    }

    #[test]
    fn signed_abs() {
        let neg = D64::new(-42, 0);
        assert_eq!(neg.abs(), D64::new(42, 0));
        assert!(neg.is_negative());
        assert!(!neg.is_positive());
    }

    #[test]
    fn signed_signum() {
        // num-traits Signed::signum returns Decimal, not backing type
        let pos: D64 = Signed::signum(&D64::new(42, 0));
        let zero: D64 = Signed::signum(&D64::ZERO);
        let neg: D64 = Signed::signum(&D64::new(-42, 0));
        assert_eq!(pos, D64::ONE);
        assert_eq!(zero, D64::ZERO);
        assert_eq!(neg, D64::NEG_ONE);
    }

    #[test]
    fn checked_add_trait() {
        let a = D64::new(1, 0);
        let b = D64::new(2, 0);
        assert_eq!(CheckedAdd::checked_add(&a, &b), Some(D64::new(3, 0)));
    }

    #[test]
    fn from_str_radix_10() {
        let d = D64::from_str_radix("42.5", 10).unwrap();
        assert_eq!(d, D64::new(42, 50_000_000));
    }

    #[test]
    fn from_str_radix_non_10_errors() {
        assert!(D64::from_str_radix("42", 16).is_err());
    }

    #[test]
    fn to_primitive() {
        let d = D64::new(42, 0);
        assert_eq!(ToPrimitive::to_i64(&d), Some(42));
        assert_eq!(ToPrimitive::to_u64(&d), Some(42));
        assert!((ToPrimitive::to_f64(&d).unwrap() - 42.0).abs() < 1e-10);
    }

    #[test]
    fn from_primitive() {
        let d: D64 = FromPrimitive::from_i64(42).unwrap();
        assert_eq!(d, D64::new(42, 0));
    }

    #[test]
    fn generic_sum() {
        fn sum_generic<T: Num + Copy>(values: &[T]) -> T {
            values.iter().fold(T::zero(), |acc, &x| acc + x)
        }
        let values = [D64::new(1, 0), D64::new(2, 0), D64::new(3, 0)];
        assert_eq!(sum_generic(&values), D64::new(6, 0));
    }
}
