//! Display, FromStr, integer/float conversions, and byte serialization.

use core::str::FromStr;
use nexus_decimal::{Decimal, ParseError};

type D32 = Decimal<i32, 4>;
type D64 = Decimal<i64, 8>;
type D96 = Decimal<i128, 12>;
type D128 = Decimal<i128, 18>;

// ============================================================================
// Display
// ============================================================================

#[test]
fn display_zero() {
    assert_eq!(D64::ZERO.to_string(), "0");
    assert_eq!(D32::ZERO.to_string(), "0");
    assert_eq!(D96::ZERO.to_string(), "0");
}

#[test]
fn display_integer() {
    assert_eq!(D64::new(42, 0).to_string(), "42");
    assert_eq!(D64::new(-42, 0).to_string(), "-42");
}

#[test]
fn display_fractional() {
    assert_eq!(D64::new(1, 50_000_000).to_string(), "1.5");
    assert_eq!(D64::new(1, 23_456_789).to_string(), "1.23456789");
}

#[test]
fn display_trailing_zero_removal() {
    assert_eq!(D64::new(1, 10_000_000).to_string(), "1.1");
    assert_eq!(D64::new(1, 12_300_000).to_string(), "1.123");
    assert_eq!(D64::new(0, 50_000_000).to_string(), "0.5");
}

#[test]
fn display_leading_zeros_in_fraction() {
    // 1.001 → fractional part 00100000 → "1.001"
    assert_eq!(D64::new(1, 100_000).to_string(), "1.001");
    assert_eq!(D64::new(0, 1).to_string(), "0.00000001");
}

#[test]
fn display_negative_fraction() {
    assert_eq!(D64::new(-1, 50_000_000).to_string(), "-1.5");
    assert_eq!(D64::from_raw(-1).to_string(), "-0.00000001");
}

#[test]
fn display_max_min() {
    // Just verify they don't panic and produce reasonable output
    let max_str = D64::MAX.to_string();
    let min_str = D64::MIN.to_string();
    assert!(max_str.starts_with("92233720368"));
    assert!(min_str.starts_with("-92233720368"));
}

#[test]
fn display_d96() {
    assert_eq!(D96::new(123, 456_000_000_000).to_string(), "123.456");
}

#[test]
fn display_custom_precision() {
    type Usd = Decimal<i64, 2>;
    assert_eq!(Usd::new(19, 99).to_string(), "19.99");
    assert_eq!(Usd::new(100, 0).to_string(), "100");
}

// ============================================================================
// Debug
// ============================================================================

#[test]
fn debug_default_shows_formatted() {
    let d = D64::new(1, 50_000_000);
    assert_eq!(format!("{d:?}"), "1.5");
}

#[test]
fn debug_alternate_shows_raw() {
    let d = D64::new(1, 50_000_000);
    let debug = format!("{d:#?}");
    assert!(debug.contains("150000000"));
}

// ============================================================================
// FromStr / from_str_exact
// ============================================================================

#[test]
fn parse_integer() {
    assert_eq!(D64::from_str("42").unwrap(), D64::new(42, 0));
    assert_eq!(D64::from_str("-42").unwrap(), D64::new(-42, 0));
    assert_eq!(D64::from_str("+42").unwrap(), D64::new(42, 0));
}

#[test]
fn parse_decimal() {
    assert_eq!(D64::from_str("123.45").unwrap(), D64::new(123, 45_000_000));
    assert_eq!(D64::from_str("-99.99").unwrap(), D64::new(-99, 99_000_000));
}

#[test]
fn parse_leading_zeros_in_fraction() {
    assert_eq!(D64::from_str("1.001").unwrap(), D64::new(1, 100_000));
    assert_eq!(D64::from_str("0.00000001").unwrap(), D64::from_raw(1));
}

#[test]
fn parse_zero() {
    assert_eq!(D64::from_str("0").unwrap(), D64::ZERO);
    assert_eq!(D64::from_str("0.0").unwrap(), D64::ZERO);
    assert_eq!(D64::from_str("-0").unwrap(), D64::ZERO);
}

#[test]
fn parse_no_integer_part() {
    assert_eq!(D64::from_str(".5").unwrap(), D64::new(0, 50_000_000));
}

#[test]
fn parse_exact_rejects_excess_precision() {
    let result = D64::from_str_exact("1.123456789"); // 9 decimals, D=8
    assert_eq!(result, Err(ParseError::PrecisionLoss));
}

#[test]
fn parse_exact_accepts_max_precision() {
    let result = D64::from_str_exact("1.12345678"); // exactly 8 decimals
    assert!(result.is_ok());
}

#[test]
fn parse_invalid_format() {
    assert_eq!(D64::from_str(""), Err(ParseError::InvalidFormat));
    assert_eq!(D64::from_str("abc"), Err(ParseError::InvalidFormat));
    assert_eq!(D64::from_str("1.2.3"), Err(ParseError::InvalidFormat));
    assert_eq!(D64::from_str("-"), Err(ParseError::InvalidFormat));
}

#[test]
fn parse_overflow() {
    // i64::MAX is ~9.2e18, with SCALE=10^8 max integer is ~92 billion
    let result = D64::from_str("999999999999999");
    assert_eq!(result, Err(ParseError::Overflow));
}

// ============================================================================
// from_str_lossy (banker's rounding)
// ============================================================================

#[test]
fn lossy_rounds_up() {
    // 1.123456786 → round digit 6 > 5 → round up
    let result = D64::from_str_lossy("1.123456786").unwrap();
    assert_eq!(result, D64::from_raw(112_345_679));
}

#[test]
fn lossy_rounds_down() {
    // 1.123456784 → round digit 4 < 5 → truncate
    let result = D64::from_str_lossy("1.123456784").unwrap();
    assert_eq!(result, D64::from_raw(112_345_678));
}

#[test]
fn lossy_bankers_round_to_even() {
    // 1.123456785 → round digit 5, no trailing → 8 is even → keep
    let result = D64::from_str_lossy("1.123456785").unwrap();
    assert_eq!(result, D64::from_raw(112_345_678));
}

#[test]
fn lossy_bankers_round_to_even_odd() {
    // 1.123456795 → round digit 5, no trailing → 9 is odd → round up
    let result = D64::from_str_lossy("1.123456795").unwrap();
    assert_eq!(result, D64::from_raw(112_345_680));
}

#[test]
fn lossy_half_with_trailing() {
    // 1.1234567850001 → round digit 5, has trailing → round up
    let result = D64::from_str_lossy("1.1234567850001").unwrap();
    assert_eq!(result, D64::from_raw(112_345_679));
}

#[test]
fn lossy_within_precision_no_rounding() {
    // 1.12345678 → exactly 8 digits → no rounding
    let result = D64::from_str_lossy("1.12345678").unwrap();
    assert_eq!(result, D64::from_raw(112_345_678));
}

// ============================================================================
// Display → FromStr round-trip
// ============================================================================

#[test]
fn display_parse_roundtrip() {
    let values = [
        D64::ZERO,
        D64::ONE,
        D64::NEG_ONE,
        D64::new(123, 45_678_900),
        D64::new(-99, 1),
        D64::from_raw(1),
        D64::from_raw(-1),
        D64::new(42, 0),
    ];
    for v in values {
        let s = v.to_string();
        let parsed = D64::from_str(&s).unwrap_or_else(|e| {
            panic!("failed to parse '{s}' (from {v:?}): {e}");
        });
        assert_eq!(parsed, v, "roundtrip failed for '{s}'");
    }
}

#[test]
fn display_parse_roundtrip_d32() {
    let values = [D32::ZERO, D32::ONE, D32::new(100, 5000)];
    for v in values {
        let s = v.to_string();
        let parsed = D32::from_str(&s).unwrap();
        assert_eq!(parsed, v);
    }
}

#[test]
fn display_parse_roundtrip_d96() {
    let values = [D96::ZERO, D96::ONE, D96::new(100, 500_000_000_000)];
    for v in values {
        let s = v.to_string();
        let parsed = D96::from_str(&s).unwrap();
        assert_eq!(parsed, v);
    }
}

// ============================================================================
// Integer conversions
// ============================================================================

#[test]
fn from_i32_basic() {
    assert_eq!(D64::from_i32(42).unwrap(), D64::new(42, 0));
    assert_eq!(D64::from_i32(-1).unwrap(), D64::NEG_ONE);
    assert_eq!(D64::from_i32(0).unwrap(), D64::ZERO);
}

#[test]
fn from_i64_basic() {
    assert_eq!(D64::from_i64(100).unwrap(), D64::new(100, 0));
}

#[test]
fn from_i64_overflow() {
    // i64::MAX / SCALE = ~92 billion max integer
    assert!(D64::from_i64(i64::MAX).is_none());
}

#[test]
fn from_u32_basic() {
    assert_eq!(D64::from_u32(42).unwrap(), D64::new(42, 0));
}

#[test]
fn from_u64_basic() {
    assert_eq!(D64::from_u64(100).unwrap(), D64::new(100, 0));
}

// ============================================================================
// Float conversions
// ============================================================================

#[test]
fn to_f64_basic() {
    assert!((D64::new(1, 50_000_000).to_f64() - 1.5).abs() < 1e-10);
    assert!((D64::ZERO.to_f64()).abs() < 1e-10);
    assert!((D64::NEG_ONE.to_f64() + 1.0).abs() < 1e-10);
}

#[cfg(feature = "std")]
#[test]
fn from_f64_basic() {
    let d = D64::from_f64(1.5).unwrap();
    assert_eq!(d, D64::new(1, 50_000_000));
}

#[cfg(feature = "std")]
#[test]
fn from_f64_nan_errors() {
    assert!(D64::from_f64(f64::NAN).is_err());
    assert!(D64::from_f64(f64::INFINITY).is_err());
    assert!(D64::from_f64(f64::NEG_INFINITY).is_err());
}

#[cfg(feature = "std")]
#[test]
fn f64_roundtrip() {
    let original = 123.456;
    let d = D64::from_f64(original).unwrap();
    let recovered = d.to_f64();
    assert!((recovered - original).abs() < 1e-6);
}

// ============================================================================
// Byte serialization
// ============================================================================

#[test]
fn bytes_roundtrip_d64() {
    let d = D64::new(123, 45_678_900);
    assert_eq!(D64::from_le_bytes(d.to_le_bytes()), d);
    assert_eq!(D64::from_be_bytes(d.to_be_bytes()), d);
    assert_eq!(D64::from_ne_bytes(d.to_ne_bytes()), d);
}

#[test]
fn bytes_roundtrip_d32() {
    let d = D32::new(100, 5000);
    assert_eq!(D32::from_le_bytes(d.to_le_bytes()), d);
}

#[test]
fn bytes_roundtrip_d128() {
    let d = D128::new(42, 0);
    assert_eq!(D128::from_le_bytes(d.to_le_bytes()), d);
    assert_eq!(D128::from_be_bytes(d.to_be_bytes()), d);
}

#[test]
fn write_read_bytes_buf() {
    let d = D64::new(99, 99_000_000);
    let mut buf = [0u8; 16];
    d.write_le_bytes(&mut buf);
    let recovered = D64::read_le_bytes(&buf);
    assert_eq!(recovered, d);
}

#[test]
fn bytes_size_constants() {
    assert_eq!(D32::BYTES, 4);
    assert_eq!(D64::BYTES, 8);
    assert_eq!(D96::BYTES, 16);
    assert_eq!(D128::BYTES, 16);
}

// ============================================================================
// Cross-type Display/Parse
// ============================================================================

#[test]
fn d128_display_parse_roundtrip() {
    let d = D128::new(42, 123_456_789_000_000_000);
    let s = d.to_string();
    assert_eq!(s, "42.123456789");
    let parsed = D128::from_str(&s).unwrap();
    assert_eq!(parsed, d);
}
