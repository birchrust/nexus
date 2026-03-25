//! Arithmetic, rounding, constants, and edge cases.
//!
//! Covers add, sub, neg, abs, floor, ceil, trunc, round, round_dp
//! across all backing types (i32, i64, i128) and custom precisions.

use nexus_decimal::{Decimal, OverflowError};

type D32 = Decimal<i32, 4>;
type D64 = Decimal<i64, 8>;
type D96 = Decimal<i128, 12>;
type D128 = Decimal<i128, 18>;

// ============================================================================
// Constants
// ============================================================================

#[test]
fn zero_is_zero() {
    assert_eq!(D32::ZERO.to_raw(), 0);
    assert_eq!(D64::ZERO.to_raw(), 0);
    assert_eq!(D96::ZERO.to_raw(), 0);
    assert_eq!(D128::ZERO.to_raw(), 0);
}

#[test]
fn one_equals_scale() {
    assert_eq!(D32::ONE.to_raw(), 10_000);
    assert_eq!(D64::ONE.to_raw(), 100_000_000);
    assert_eq!(D96::ONE.to_raw(), 1_000_000_000_000);
    assert_eq!(D128::ONE.to_raw(), 1_000_000_000_000_000_000);
}

#[test]
fn neg_one() {
    assert_eq!(D64::NEG_ONE.to_raw(), -100_000_000);
    assert_eq!(D64::NEG_ONE, -D64::ONE);
}

#[test]
fn max_min_boundaries() {
    assert_eq!(D64::MAX.to_raw(), i64::MAX);
    assert_eq!(D64::MIN.to_raw(), i64::MIN);
    assert_eq!(D32::MAX.to_raw(), i32::MAX);
    assert_eq!(D32::MIN.to_raw(), i32::MIN);
    assert_eq!(D96::MAX.to_raw(), i128::MAX);
    assert_eq!(D128::MAX.to_raw(), i128::MAX);
}

#[test]
fn scale_values() {
    assert_eq!(D32::SCALE, 10_000);
    assert_eq!(D64::SCALE, 100_000_000);
    assert_eq!(D96::SCALE, 1_000_000_000_000);
    assert_eq!(D128::SCALE, 1_000_000_000_000_000_000);
}

#[test]
fn custom_precision() {
    type Usd = Decimal<i64, 2>;
    assert_eq!(Usd::SCALE, 100);
    assert_eq!(Usd::ONE.to_raw(), 100);
    assert_eq!(Usd::new(19, 99).to_raw(), 1999);
}

// ============================================================================
// Constructors
// ============================================================================

#[test]
fn from_raw_roundtrip() {
    let d = D64::from_raw(12345);
    assert_eq!(d.to_raw(), 12345);
}

#[test]
fn new_positive() {
    let d = D64::new(100, 50_000_000); // 100.50
    assert_eq!(d.to_raw(), 10_050_000_000);
}

#[test]
fn new_negative() {
    let d = D64::new(-50, 25_000_000); // -50.25
    assert_eq!(d.to_raw(), -5_025_000_000);
}

#[test]
fn new_zero() {
    let d = D64::new(0, 0);
    assert_eq!(d, D64::ZERO);
}

#[test]
#[should_panic(expected = "overflow")]
fn new_overflow_panics() {
    let _ = D64::new(i64::MAX, 0);
}

#[test]
fn default_is_zero() {
    assert_eq!(D64::default(), D64::ZERO);
    assert_eq!(D32::default(), D32::ZERO);
}

// ============================================================================
// Query methods
// ============================================================================

#[test]
fn is_zero_positive_negative() {
    assert!(D64::ZERO.is_zero());
    assert!(!D64::ONE.is_zero());

    assert!(D64::ONE.is_positive());
    assert!(!D64::ZERO.is_positive());
    assert!(!D64::NEG_ONE.is_positive());

    assert!(D64::NEG_ONE.is_negative());
    assert!(!D64::ZERO.is_negative());
    assert!(!D64::ONE.is_negative());
}

#[test]
fn signum_values() {
    assert_eq!(D64::ONE.signum(), 1);
    assert_eq!(D64::ZERO.signum(), 0);
    assert_eq!(D64::NEG_ONE.signum(), -1);
}

// ============================================================================
// Checked arithmetic
// ============================================================================

#[test]
fn checked_add_basic() {
    let a = D64::new(10, 0);
    let b = D64::new(20, 0);
    assert_eq!(a.checked_add(b).unwrap().to_raw(), D64::new(30, 0).to_raw());
}

#[test]
fn checked_add_overflow_returns_none() {
    assert!(D64::MAX.checked_add(D64::ONE).is_none());
}

#[test]
fn checked_sub_basic() {
    let a = D64::new(30, 0);
    let b = D64::new(10, 0);
    assert_eq!(a.checked_sub(b).unwrap().to_raw(), D64::new(20, 0).to_raw());
}

#[test]
fn checked_sub_overflow_returns_none() {
    assert!(D64::MIN.checked_sub(D64::ONE).is_none());
}

#[test]
fn checked_neg_basic() {
    assert_eq!(D64::ONE.checked_neg().unwrap(), D64::NEG_ONE);
    assert_eq!(D64::NEG_ONE.checked_neg().unwrap(), D64::ONE);
    assert_eq!(D64::ZERO.checked_neg().unwrap(), D64::ZERO);
}

#[test]
fn checked_neg_min_returns_none() {
    assert!(D64::MIN.checked_neg().is_none());
}

#[test]
fn checked_abs_basic() {
    assert_eq!(D64::ONE.checked_abs().unwrap(), D64::ONE);
    assert_eq!(D64::NEG_ONE.checked_abs().unwrap(), D64::ONE);
    assert_eq!(D64::ZERO.checked_abs().unwrap(), D64::ZERO);
}

#[test]
fn checked_abs_min_returns_none() {
    assert!(D64::MIN.checked_abs().is_none());
}

// ============================================================================
// Saturating arithmetic
// ============================================================================

#[test]
fn saturating_add_clamps() {
    assert_eq!(D64::MAX.saturating_add(D64::ONE), D64::MAX);
    assert_eq!(D64::MIN.saturating_add(D64::NEG_ONE), D64::MIN);
}

#[test]
fn saturating_sub_clamps() {
    assert_eq!(D64::MIN.saturating_sub(D64::ONE), D64::MIN);
    assert_eq!(D64::MAX.saturating_sub(D64::NEG_ONE), D64::MAX);
}

#[test]
fn saturating_neg_min() {
    assert_eq!(D64::MIN.saturating_neg(), D64::MAX);
}

#[test]
fn saturating_abs_min() {
    assert_eq!(D64::MIN.saturating_abs(), D64::MAX);
}

// ============================================================================
// Wrapping arithmetic
// ============================================================================

#[test]
fn wrapping_add_wraps() {
    let result = D64::MAX.wrapping_add(D64::from_raw(1));
    assert_eq!(result, D64::MIN);
}

#[test]
fn wrapping_neg_min() {
    // MIN.wrapping_neg() == MIN for two's complement
    assert_eq!(D64::MIN.wrapping_neg(), D64::MIN);
}

// ============================================================================
// Try variants
// ============================================================================

#[test]
fn try_add_ok() {
    let result = D64::ONE.try_add(D64::ONE);
    assert_eq!(result.unwrap().to_raw(), D64::new(2, 0).to_raw());
}

#[test]
fn try_add_overflow() {
    let result = D64::MAX.try_add(D64::ONE);
    assert_eq!(result, Err(OverflowError));
}

#[test]
fn try_neg_min() {
    assert_eq!(D64::MIN.try_neg(), Err(OverflowError));
}

// ============================================================================
// Operator traits
// ============================================================================

#[test]
fn add_operator() {
    let a = D64::new(10, 0);
    let b = D64::new(20, 0);
    assert_eq!((a + b).to_raw(), D64::new(30, 0).to_raw());
}

#[test]
fn sub_operator() {
    let a = D64::new(30, 0);
    let b = D64::new(10, 0);
    assert_eq!((a - b).to_raw(), D64::new(20, 0).to_raw());
}

#[test]
fn neg_operator() {
    assert_eq!(-D64::ONE, D64::NEG_ONE);
}

#[test]
fn add_assign() {
    let mut a = D64::new(10, 0);
    a += D64::new(5, 0);
    assert_eq!(a.to_raw(), D64::new(15, 0).to_raw());
}

#[test]
fn sub_assign() {
    let mut a = D64::new(10, 0);
    a -= D64::new(3, 0);
    assert_eq!(a.to_raw(), D64::new(7, 0).to_raw());
}

#[test]
#[should_panic(expected = "overflow")]
fn add_operator_overflow_panics() {
    let _ = D64::MAX + D64::ONE;
}

// ============================================================================
// Rounding
// ============================================================================

#[test]
fn floor_positive() {
    assert_eq!(D64::new(1, 75_000_000).floor(), D64::new(1, 0));
    assert_eq!(D64::new(1, 0).floor(), D64::new(1, 0));
}

#[test]
fn floor_negative() {
    assert_eq!(D64::new(-1, 75_000_000).floor(), D64::new(-2, 0));
    assert_eq!(D64::new(-1, 0).floor(), D64::new(-1, 0));
}

#[test]
fn ceil_positive() {
    assert_eq!(D64::new(1, 25_000_000).ceil(), D64::new(2, 0));
    assert_eq!(D64::new(1, 0).ceil(), D64::new(1, 0));
}

#[test]
fn ceil_negative() {
    assert_eq!(D64::new(-1, 25_000_000).ceil(), D64::new(-1, 0));
    assert_eq!(D64::new(-1, 0).ceil(), D64::new(-1, 0));
}

#[test]
fn trunc_positive() {
    assert_eq!(D64::new(1, 99_000_000).trunc(), D64::new(1, 0));
}

#[test]
fn trunc_negative() {
    assert_eq!(D64::new(-1, 99_000_000).trunc(), D64::new(-1, 0));
}

#[test]
fn fract_positive() {
    let d = D64::new(1, 75_000_000); // 1.75
    assert_eq!(d.fract().to_raw(), 75_000_000);
}

#[test]
fn fract_negative() {
    let d = D64::new(-1, 75_000_000); // -1.75
    assert_eq!(d.fract().to_raw(), -75_000_000);
}

#[test]
fn trunc_plus_fract_identity() {
    let values = [
        D64::new(1, 75_000_000),
        D64::new(-1, 75_000_000),
        D64::ZERO,
        D64::new(99, 99_999_999),
    ];
    for d in values {
        assert_eq!(d.trunc() + d.fract(), d, "trunc + fract != self");
    }
}

#[test]
fn to_integer() {
    assert_eq!(D64::new(42, 75_000_000).to_integer(), 42);
    assert_eq!(D64::new(-42, 75_000_000).to_integer(), -42);
    assert_eq!(D64::ZERO.to_integer(), 0);
}

#[test]
fn round_bankers() {
    // 2.5 → 2 (round to even)
    assert_eq!(D64::new(2, 50_000_000).round(), D64::new(2, 0));
    // 3.5 → 4 (round to even)
    assert_eq!(D64::new(3, 50_000_000).round(), D64::new(4, 0));
    // 1.6 → 2
    assert_eq!(D64::new(1, 60_000_000).round(), D64::new(2, 0));
    // 1.4 → 1
    assert_eq!(D64::new(1, 40_000_000).round(), D64::new(1, 0));
}

#[test]
fn round_bankers_negative() {
    // -2.5 → -2 (round to even)
    assert_eq!(D64::new(-2, 50_000_000).round(), D64::new(-2, 0));
    // -3.5 → -4 (round to even)
    assert_eq!(D64::new(-3, 50_000_000).round(), D64::new(-4, 0));
}

#[test]
fn round_dp_basic() {
    let d = D64::new(1, 23_456_789); // 1.23456789
    assert_eq!(d.round_dp(2), D64::new(1, 23_000_000)); // 1.23
    assert_eq!(d.round_dp(4), D64::new(1, 23_460_000)); // 1.2346 (banker's: 5 rounds to even 6)
}

#[test]
fn round_dp_bankers_half() {
    // 1.235 rounded to 2dp: 5 is half, 3 is odd → round up to 1.24
    let d = D64::from_raw(123_500_000); // 1.235
    assert_eq!(d.round_dp(2).to_raw(), 124_000_000); // 1.24

    // 1.225 rounded to 2dp: 5 is half, 2 is even → stay at 1.22
    let d = D64::from_raw(122_500_000); // 1.225
    assert_eq!(d.round_dp(2).to_raw(), 122_000_000); // 1.22
}

#[test]
#[should_panic(expected = "round_dp")]
fn round_dp_panics_if_dp_equals_decimals() {
    let _ = D64::ONE.round_dp(8);
}

// ============================================================================
// Cross-backing-type tests
// ============================================================================

#[test]
fn d32_basic_arithmetic() {
    let a = D32::new(100, 5000); // 100.5
    let b = D32::new(50, 2500); // 50.25
    assert_eq!((a + b).to_raw(), D32::new(150, 7500).to_raw());
}

#[test]
fn d96_basic_arithmetic() {
    let a = D96::new(100, 500_000_000_000); // 100.5
    let b = D96::new(50, 250_000_000_000); // 50.25
    assert_eq!((a + b).to_raw(), D96::new(150, 750_000_000_000).to_raw());
}

#[test]
fn d128_basic_arithmetic() {
    let a = D128::new(100, 500_000_000_000_000_000); // 100.5
    let b = D128::new(50, 250_000_000_000_000_000); // 50.25
    assert_eq!(
        (a + b).to_raw(),
        D128::new(150, 750_000_000_000_000_000).to_raw()
    );
}

// ============================================================================
// Compile-time validation
// ============================================================================

#[test]
fn const_evaluation() {
    // Verify const fn works at compile time
    const A: D64 = D64::new(100, 0);
    const B: D64 = D64::new(50, 0);
    const SUM: D64 = match A.checked_add(B) {
        Some(v) => v,
        None => panic!("overflow"),
    };
    assert_eq!(SUM.to_raw(), D64::new(150, 0).to_raw());
}

#[test]
fn const_rounding() {
    const D: D64 = D64::new(1, 75_000_000);
    const FLOORED: D64 = D.floor();
    const CEILED: D64 = D.ceil();
    const TRUNCATED: D64 = D.trunc();
    assert_eq!(FLOORED, D64::new(1, 0));
    assert_eq!(CEILED, D64::new(2, 0));
    assert_eq!(TRUNCATED, D64::new(1, 0));
}

// ============================================================================
// Ordering and equality
// ============================================================================

#[test]
fn ordering() {
    assert!(D64::ONE > D64::ZERO);
    assert!(D64::ZERO > D64::NEG_ONE);
    assert!(D64::MIN < D64::MAX);
    assert!(D64::new(1, 50_000_000) > D64::new(1, 49_999_999));
}

#[test]
fn equality() {
    assert_eq!(D64::from_raw(100), D64::from_raw(100));
    assert_ne!(D64::from_raw(100), D64::from_raw(101));
}

// ============================================================================
// D96 (Decimal<i128, 12>) rounding
// ============================================================================

mod d96_rounding {
    type D96 = nexus_decimal::Decimal<i128, 12>;

    #[test]
    fn floor_positive() {
        // 1.75 → 1.0
        assert_eq!(D96::new(1, 750_000_000_000).floor(), D96::new(1, 0));
        // Already whole → unchanged
        assert_eq!(D96::new(3, 0).floor(), D96::new(3, 0));
        // 0.001 → 0
        assert_eq!(D96::from_raw(1_000_000_000).floor(), D96::ZERO);
    }

    #[test]
    fn floor_negative() {
        // -1.75 → -2.0
        assert_eq!(D96::new(-1, 750_000_000_000).floor(), D96::new(-2, 0));
        // Already whole → unchanged
        assert_eq!(D96::new(-3, 0).floor(), D96::new(-3, 0));
        // -0.001 → -1.0
        assert_eq!(D96::from_raw(-1_000_000_000).floor(), D96::NEG_ONE);
    }

    #[test]
    fn ceil_positive() {
        // 1.25 → 2.0
        assert_eq!(D96::new(1, 250_000_000_000).ceil(), D96::new(2, 0));
        // Already whole → unchanged
        assert_eq!(D96::new(3, 0).ceil(), D96::new(3, 0));
        // 0.001 → 1.0
        assert_eq!(D96::from_raw(1_000_000_000).ceil(), D96::ONE);
    }

    #[test]
    fn ceil_negative() {
        // -1.25 → -1.0
        assert_eq!(D96::new(-1, 250_000_000_000).ceil(), D96::new(-1, 0));
        // Already whole → unchanged
        assert_eq!(D96::new(-3, 0).ceil(), D96::new(-3, 0));
        // -0.001 → 0.0
        assert_eq!(D96::from_raw(-1_000_000_000).ceil(), D96::ZERO);
    }

    #[test]
    fn trunc_positive() {
        // 1.99 → 1.0
        assert_eq!(D96::new(1, 990_000_000_000).trunc(), D96::new(1, 0));
        // 0.999 → 0.0
        assert_eq!(D96::from_raw(999_000_000_000).trunc(), D96::ZERO);
    }

    #[test]
    fn trunc_negative() {
        // -1.99 → -1.0 (towards zero)
        assert_eq!(D96::new(-1, 990_000_000_000).trunc(), D96::new(-1, 0));
        // -0.999 → 0.0
        assert_eq!(D96::from_raw(-999_000_000_000).trunc(), D96::ZERO);
    }

    #[test]
    fn fract_values() {
        // 1.75 → 0.75
        let d = D96::new(1, 750_000_000_000);
        assert_eq!(d.fract().to_raw(), 750_000_000_000);

        // -1.75 → -0.75
        let d = D96::new(-1, 750_000_000_000);
        assert_eq!(d.fract().to_raw(), -750_000_000_000);

        // Whole number → 0
        assert_eq!(D96::new(5, 0).fract(), D96::ZERO);

        // Zero → 0
        assert_eq!(D96::ZERO.fract(), D96::ZERO);
    }

    #[test]
    fn round_bankers_positive() {
        // 2.5 → 2 (half, even integer → stay)
        assert_eq!(D96::new(2, 500_000_000_000).round(), D96::new(2, 0));
        // 3.5 → 4 (half, odd integer → round up)
        assert_eq!(D96::new(3, 500_000_000_000).round(), D96::new(4, 0));
        // 1.6 → 2 (above half → round up)
        assert_eq!(D96::new(1, 600_000_000_000).round(), D96::new(2, 0));
        // 1.4 → 1 (below half → round down)
        assert_eq!(D96::new(1, 400_000_000_000).round(), D96::new(1, 0));
        // 0.5 → 0 (half, 0 is even → stay)
        assert_eq!(D96::new(0, 500_000_000_000).round(), D96::ZERO);
        // 1.5 → 2 (half, 1 is odd → round up)
        assert_eq!(D96::new(1, 500_000_000_000).round(), D96::new(2, 0));
    }

    #[test]
    fn round_bankers_negative() {
        // -2.5 → -2 (half, even → stay)
        assert_eq!(D96::new(-2, 500_000_000_000).round(), D96::new(-2, 0));
        // -3.5 → -4 (half, odd → round away)
        assert_eq!(D96::new(-3, 500_000_000_000).round(), D96::new(-4, 0));
        // -1.6 → -2 (above half magnitude → round away)
        assert_eq!(D96::new(-1, 600_000_000_000).round(), D96::new(-2, 0));
        // -1.4 → -1 (below half → truncate)
        assert_eq!(D96::new(-1, 400_000_000_000).round(), D96::new(-1, 0));
    }

    #[test]
    fn round_dp_basic() {
        // 1.234567890123 rounded to 2dp → 1.23
        let d = D96::from_raw(1_234_567_890_123);
        assert_eq!(d.round_dp(2), D96::from_raw(1_230_000_000_000));

        // 1.235 rounded to 2dp: half, 3 is odd → round up to 1.24
        let d = D96::from_raw(1_235_000_000_000);
        assert_eq!(d.round_dp(2), D96::from_raw(1_240_000_000_000));

        // 1.225 rounded to 2dp: half, 2 is even → stay at 1.22
        let d = D96::from_raw(1_225_000_000_000);
        assert_eq!(d.round_dp(2), D96::from_raw(1_220_000_000_000));

        // round to 6dp
        let d = D96::from_raw(1_234_567_890_123); // 1.234567890123
        assert_eq!(d.round_dp(6), D96::from_raw(1_234_568_000_000)); // 1.234568
    }

    #[test]
    fn trunc_plus_fract_identity() {
        let values = [
            D96::new(1, 750_000_000_000),
            D96::new(-1, 750_000_000_000),
            D96::ZERO,
            D96::new(99, 999_999_999_999),
            D96::new(-99, 999_999_999_999),
            D96::from_raw(1), // smallest positive fraction
        ];
        for d in values {
            assert_eq!(
                d.trunc() + d.fract(),
                d,
                "trunc + fract != self for {:?}",
                d.to_raw()
            );
        }
    }
}
