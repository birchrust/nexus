//! 192-bit arithmetic helpers for i128 multiplication and division.
//!
//! Ported from fixdec. These handle the case where i128 × i128
//! produces a result wider than 128 bits.

/// Multiply two u128 values, returning a 192-bit result as `(low_128, high_64)`.
///
/// Both inputs should be at most ~96 bits for the result to fit in 192 bits.
/// For larger inputs the high part may exceed u64 — callers must verify.
#[inline(always)]
pub(crate) const fn mul_wide(a: u128, b: u128) -> (u128, u64) {
    let a_lo = a as u64;
    let a_hi = (a >> 64) as u64;
    let b_lo = b as u64;
    let b_hi = (b >> 64) as u64;

    let p0 = a_lo as u128 * b_lo as u128;
    let p1 = a_lo as u128 * b_hi as u128;
    let p2 = a_hi as u128 * b_lo as u128;
    let p3 = a_hi as u128 * b_hi as u128;

    let mid = p1 + p2;
    let (low, carry) = p0.overflowing_add(mid << 64);
    let high = p3 + (mid >> 64) + (carry as u128);

    (low, high as u64)
}

/// Multiply a u128 by a smaller u128 (e.g., SCALE), returning `(low_128, high_128)`.
///
/// Used for division: `a * SCALE` where a is the dividend and SCALE is ~40 bits.
#[inline(always)]
pub(crate) const fn mul_u128_by_small(a: u128, b: u128) -> (u128, u128) {
    let a_lo = a as u64 as u128;
    let a_hi = a >> 64;

    let p0 = a_lo * b;
    let p1 = a_hi * b;

    let (low, carry) = p0.overflowing_add(p1 << 64);
    let high = (p1 >> 64) + (carry as u128);

    (low, high)
}

/// Divide a 192-bit value `(low, high)` by a constant scale factor.
///
/// Returns `None` if the result doesn't fit in u128.
#[inline(always)]
pub(crate) const fn div_192_by_const(low: u128, high: u64, scale: u128) -> Option<u128> {
    let high = high as u128;

    if high >= scale {
        return None;
    }

    // Fast path: high is zero
    if high == 0 {
        return Some(low / scale);
    }

    // Long division on 64-bit chunks
    let low_hi = low >> 64;
    let low_lo = low & 0xFFFF_FFFF_FFFF_FFFF;

    let dividend_mid = (high << 64) | low_hi;
    let q_mid = dividend_mid / scale;
    let r_mid = dividend_mid % scale;

    let dividend_low = (r_mid << 64) | low_lo;
    let q_low = dividend_low / scale;

    Some((q_mid << 64) | q_low)
}

/// Wrapping version of 192-bit division by constant.
#[inline(always)]
pub(crate) const fn div_192_by_const_wrapping(low: u128, high: u64, scale: u128) -> u128 {
    let r_high = (high as u128) % scale;

    let low_hi = low >> 64;
    let low_lo = low & 0xFFFF_FFFF_FFFF_FFFF;

    let dividend_mid = (r_high << 64) | low_hi;
    let q_mid = dividend_mid / scale;
    let r_mid = dividend_mid % scale;

    let dividend_low = (r_mid << 64) | low_lo;
    let q_low = dividend_low / scale;

    (q_mid << 64) | q_low
}

/// Divide a 192-bit value by a runtime u128 divisor.
///
/// Used for `Decimal<i128, D> / Decimal<i128, D>`.
/// Returns `None` if result doesn't fit in u128 or divisor is zero.
#[inline(always)]
pub(crate) const fn div_192_by_u128(low: u128, high: u128, divisor: u128) -> Option<u128> {
    if divisor == 0 || high >= divisor {
        return None;
    }

    let r_high = high % divisor;

    let low_hi = low >> 64;
    let low_lo = low & 0xFFFF_FFFF_FFFF_FFFF;

    let dividend_mid = (r_high << 64) | low_hi;
    let q_mid = dividend_mid / divisor;
    let r_mid = dividend_mid % divisor;

    let dividend_low = (r_mid << 64) | low_lo;
    let q_low = dividend_low / divisor;

    Some((q_mid << 64) | q_low)
}

/// Wrapping version of 192-bit division by u128.
#[inline(always)]
pub(crate) const fn div_192_by_u128_wrapping(low: u128, high: u128, divisor: u128) -> u128 {
    if divisor == 0 {
        return 0;
    }

    let r_high = high % divisor;

    let low_hi = low >> 64;
    let low_lo = low & 0xFFFF_FFFF_FFFF_FFFF;

    let dividend_mid = (r_high << 64) | low_hi;
    let q_mid = dividend_mid / divisor;
    let r_mid = dividend_mid % divisor;

    let dividend_low = (r_mid << 64) | low_lo;
    let q_low = dividend_low / divisor;

    (q_mid << 64) | q_low
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_wide_basic() {
        // 1 * 1 = 1
        let (low, high) = mul_wide(1, 1);
        assert_eq!(low, 1);
        assert_eq!(high, 0);
    }

    #[test]
    fn mul_wide_large() {
        // (2^64) * (2^64) = 2^128 → low=0, high=1
        let a = 1u128 << 64;
        let b = 1u128 << 64;
        let (low, high) = mul_wide(a, b);
        assert_eq!(low, 0);
        assert_eq!(high, 1);
    }

    #[test]
    fn div_192_by_const_basic() {
        // 1_000_000_000_000 / 1_000_000_000_000 = 1
        let scale = 1_000_000_000_000u128;
        let result = div_192_by_const(scale, 0, scale);
        assert_eq!(result, Some(1));
    }

    #[test]
    fn div_192_by_const_with_high() {
        let scale = 1_000_000_000_000u128;
        // high=1 means the 192-bit value is 1 * 2^128 + 0 = 2^128
        // 2^128 / 10^12 = 340282366920938463463374607.431768211456
        // floor = 340282366920938463463374607
        let result = div_192_by_const(0, 1, scale);
        assert_eq!(result, Some(340_282_366_920_938_463_463_374_607));
    }

    #[test]
    fn mul_u128_by_small_basic() {
        let (low, high) = mul_u128_by_small(1_000_000_000_000, 1_000_000_000_000);
        assert_eq!(high, 0);
        assert_eq!(low, 1_000_000_000_000_000_000_000_000);
    }

    #[test]
    fn div_192_by_u128_basic() {
        let result = div_192_by_u128(100, 0, 10);
        assert_eq!(result, Some(10));
    }

    #[test]
    fn div_192_by_u128_zero_divisor() {
        assert_eq!(div_192_by_u128(100, 0, 0), None);
    }

    #[test]
    fn roundtrip_mul_div() {
        // a * b / SCALE should give the multiplication result
        let a: u128 = 12_345_678_901_234;
        let b: u128 = 98_765_432_109_876;
        let scale: u128 = 1_000_000_000_000; // 10^12

        let (prod_low, prod_high) = mul_wide(a, b);
        let result = div_192_by_const(prod_low, prod_high, scale).unwrap();

        // Verify against f64 approximation
        let expected = (a as f64) * (b as f64) / (scale as f64);
        let diff = (result as f64 - expected).abs();
        assert!(diff < 2.0, "roundtrip error too large: {diff}");
    }
}
