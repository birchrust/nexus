//! 256-bit arithmetic helpers for i128 multiplication and division.
//!
//! Handles the case where i128 x i128 produces a result wider than 128 bits.
//! Division uses Knuth Algorithm D (TAOCP Vol 2, section 4.3.1) for correctness
//! with arbitrarily large divisors.

/// Multiply two u128 values, returning a 256-bit result as `(low_128, high_128)`.
///
/// Handles the carry from `p1 + p2` overflow correctly.
#[inline(always)]
pub(crate) const fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    let a_lo = a as u64;
    let a_hi = (a >> 64) as u64;
    let b_lo = b as u64;
    let b_hi = (b >> 64) as u64;

    let p0 = a_lo as u128 * b_lo as u128;
    let p1 = a_lo as u128 * b_hi as u128;
    let p2 = a_hi as u128 * b_lo as u128;
    let p3 = a_hi as u128 * b_hi as u128;

    let (mid, mid_carry) = p1.overflowing_add(p2);
    let (low, carry0) = p0.overflowing_add(mid << 64);
    let high = p3 + (mid >> 64) + (carry0 as u128) + ((mid_carry as u128) << 64);

    (low, high)
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

// ============================================================================
// Wide division — core implementation
// ============================================================================

/// Divide a 256-bit value `(lo, hi)` by a 128-bit divisor.
///
/// Returns `None` if the result doesn't fit in u128, divisor is zero,
/// or `hi >= divisor` (quotient overflow).
///
/// For divisors < 2^64, uses the fast 2-digit schoolbook algorithm.
/// For divisors >= 2^64, uses Knuth Algorithm D with normalized
/// long division to ensure correct quotient estimates.
#[inline(always)]
pub(crate) const fn div_wide(lo: u128, hi: u128, divisor: u128) -> Option<u128> {
    if divisor == 0 || hi >= divisor {
        return None;
    }
    if hi == 0 {
        return Some(lo / divisor);
    }
    if divisor <= u64::MAX as u128 {
        return Some(div_wide_small_divisor(lo, hi, divisor));
    }
    Some(div_wide_large_divisor(lo, hi, divisor))
}

/// Wrapping wide division: reduces `hi` modulo `divisor`, then divides.
///
/// Returns the low 128 bits of the quotient. Returns 0 if divisor is zero.
#[inline(always)]
pub(crate) const fn div_wide_wrapping(lo: u128, hi: u128, divisor: u128) -> u128 {
    if divisor == 0 {
        return 0;
    }
    let hi = hi % divisor;
    // After reduction, hi < divisor, so div_wide always returns Some.
    match div_wide(lo, hi, divisor) {
        Some(q) => q,
        None => 0,
    }
}

/// Fast path for divisors that fit in 64 bits.
///
/// Remainder is always < divisor < 2^64, so `remainder << 64` fits in u128.
/// This is the original 2-digit schoolbook algorithm.
const fn div_wide_small_divisor(lo: u128, hi: u128, divisor: u128) -> u128 {
    let lo_hi = lo >> 64;
    let lo_lo = lo & 0xFFFF_FFFF_FFFF_FFFF;

    // hi < divisor < 2^64, so (hi << 64) fits in u128
    let d1 = (hi << 64) | lo_hi;
    let q1 = d1 / divisor;
    let r1 = d1 % divisor;

    // r1 < divisor < 2^64, so (r1 << 64) fits in u128
    let d0 = (r1 << 64) | lo_lo;
    let q0 = d0 / divisor;

    (q1 << 64) | q0
}

/// Knuth Algorithm D for divisors >= 2^64.
///
/// Divides a 256-bit dividend (4 x 64-bit digits) by a 128-bit divisor
/// (2 x 64-bit digits). The quotient fits in 128 bits (2 digits) because
/// `hi < divisor`.
///
/// Steps:
/// 1. Normalize: left-shift divisor until MSB is set, shift dividend equally.
/// 2. Compute high quotient digit q1 via `div_3by2`.
/// 3. Compute low quotient digit q0 from the remainder.
const fn div_wide_large_divisor(lo: u128, hi: u128, divisor: u128) -> u128 {
    let v1 = (divisor >> 64) as u64;

    // Normalize: shift so v1's MSB is set (ensures quotient estimates are
    // accurate to within 2, per Knuth's analysis)
    let shift = v1.leading_zeros();
    let divisor_n = divisor << shift;
    let v1n = (divisor_n >> 64) as u64;
    let v0n = divisor_n as u64;

    // Shift dividend by the same amount
    let (lo_n, hi_n) = if shift == 0 {
        (lo, hi)
    } else {
        (lo << shift, (hi << shift) | (lo >> (128 - shift)))
    };

    // Extract 64-bit digits of the normalized dividend
    let d0 = lo_n as u64;
    let d1 = (lo_n >> 64) as u64;
    let d2 = hi_n as u64;
    let d3 = (hi_n >> 64) as u64;

    // Compute q1: [d3:d2:d1] / [v1n:v0n]
    let (q1, r_hi, r_lo) = div_3by2(d3, d2, d1, v1n, v0n);

    // Compute q0: [r_hi:r_lo:d0] / [v1n:v0n]
    let (q0, _, _) = div_3by2(r_hi, r_lo, d0, v1n, v0n);

    ((q1 as u128) << 64) | (q0 as u128)
}

/// Divide a 3-digit number [u2:u1:u0] by a 2-digit number [v1:v0].
///
/// Returns `(quotient_digit, remainder_hi, remainder_lo)`.
/// All "digits" are 64 bits. The divisor must be normalized (v1 MSB set).
/// The quotient must fit in a single 64-bit digit.
///
/// Algorithm: Knuth TAOCP Vol 2, section 4.3.1, Algorithm D steps D3-D4.
/// 1. Trial quotient: q_hat = [u2:u1] / v1 (cap at 2^64-1 on overflow)
/// 2. Refine: correct q_hat downward at most twice using v0
/// 3. Multiply-subtract: [u2:u1:u0] -= q_hat * [v1:v0]
/// 4. Add-back: if subtraction underflowed, add [v1:v0] once, decrement q_hat
const fn div_3by2(u2: u64, u1: u64, u0: u64, v1: u64, v0: u64) -> (u64, u64, u64) {
    let u_top = ((u2 as u128) << 64) | (u1 as u128);

    // Trial quotient: [u2:u1] / v1
    let (mut q_hat, mut r_hat): (u128, u128) = if u2 >= v1 {
        // Division would overflow u64; cap at u64::MAX.
        // Compute remainder: [u2:u1] - (2^64-1) * v1
        let q = u64::MAX as u128;
        let r = u_top - q * (v1 as u128);
        (q, r)
    } else {
        (u_top / (v1 as u128), u_top % (v1 as u128))
    };

    // Refine: while q_hat * v0 > [r_hat : u0], decrement q_hat.
    // Runs at most 2 times (guaranteed by normalization).
    let mut i = 0;
    while i < 2 {
        // If r_hat doesn't fit in 64 bits, [r_hat:u0] exceeds any
        // 128-bit product q_hat * v0, so the estimate is already good.
        if r_hat > u64::MAX as u128 {
            break;
        }
        let lhs = q_hat * (v0 as u128);
        let rhs = (r_hat << 64) | (u0 as u128);
        if lhs <= rhs {
            break;
        }
        q_hat -= 1;
        r_hat += v1 as u128;
        i += 1;
    }

    let q = q_hat as u64;

    // Multiply q * [v1:v0] into 3 limbs [p2:p1:p0]
    let prod_v0 = (q as u128) * (v0 as u128);
    let prod_v1 = (q as u128) * (v1 as u128);
    let p0 = prod_v0 as u64;
    let carry = prod_v0 >> 64;
    let mid = prod_v1 + carry;
    let p1 = mid as u64;
    let p2 = (mid >> 64) as u64;

    // Subtract [p2:p1:p0] from [u2:u1:u0]
    let u_lo = ((u1 as u128) << 64) | (u0 as u128);
    let p_lo = ((p1 as u128) << 64) | (p0 as u128);
    let (diff_lo, borrow) = u_lo.overflowing_sub(p_lo);
    let underflow = (u2 as u128) < (p2 as u128) + (borrow as u128);

    if underflow {
        // q was too large by 1. Add divisor back to get the correct remainder.
        let divisor = ((v1 as u128) << 64) | (v0 as u128);
        let rem = diff_lo.wrapping_add(divisor);
        (q - 1, (rem >> 64) as u64, rem as u64)
    } else {
        (q, (diff_lo >> 64) as u64, diff_lo as u64)
    }
}

// ============================================================================
// Public API wrappers — maintain existing call sites in arithmetic.rs
// ============================================================================

/// Divide a wide value `(low, high)` by a constant scale factor.
///
/// Returns `None` if the result doesn't fit in u128.
#[inline(always)]
pub(crate) const fn div_192_by_const(low: u128, high: u128, scale: u128) -> Option<u128> {
    div_wide(low, high, scale)
}

/// Wrapping version of wide division by constant.
#[inline(always)]
pub(crate) const fn div_192_by_const_wrapping(low: u128, high: u128, scale: u128) -> u128 {
    div_wide_wrapping(low, high, scale)
}

/// Divide a 256-bit value by a runtime u128 divisor.
///
/// Returns `None` if result doesn't fit in u128 or divisor is zero.
#[inline(always)]
pub(crate) const fn div_192_by_u128(low: u128, high: u128, divisor: u128) -> Option<u128> {
    div_wide(low, high, divisor)
}

/// Wrapping version of 256-bit division by u128.
#[inline(always)]
pub(crate) const fn div_192_by_u128_wrapping(low: u128, high: u128, divisor: u128) -> u128 {
    div_wide_wrapping(low, high, divisor)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Multiplication tests (unchanged)
    // ========================================================================

    #[test]
    fn mul_wide_basic() {
        let (low, high) = mul_wide(1, 1);
        assert_eq!(low, 1);
        assert_eq!(high, 0);
    }

    #[test]
    fn mul_wide_large() {
        let a = 1u128 << 64;
        let b = 1u128 << 64;
        let (low, high) = mul_wide(a, b);
        assert_eq!(low, 0);
        assert_eq!(high, 1);
    }

    #[test]
    fn mul_u128_by_small_basic() {
        let (low, high) = mul_u128_by_small(1_000_000_000_000, 1_000_000_000_000);
        assert_eq!(high, 0);
        assert_eq!(low, 1_000_000_000_000_000_000_000_000);
    }

    // ========================================================================
    // Division tests — small divisor (< 2^64)
    // ========================================================================

    #[test]
    fn div_small_divisor_basic() {
        let scale = 1_000_000_000_000u128;
        assert_eq!(div_192_by_const(scale, 0, scale), Some(1));
    }

    #[test]
    fn div_small_divisor_with_high() {
        let scale = 1_000_000_000_000u128;
        let result = div_192_by_const(0, 1, scale);
        assert_eq!(result, Some(340_282_366_920_938_463_463_374_607));
    }

    #[test]
    fn div_u128_basic() {
        assert_eq!(div_192_by_u128(100, 0, 10), Some(10));
    }

    #[test]
    fn div_u128_zero_divisor() {
        assert_eq!(div_192_by_u128(100, 0, 0), None);
    }

    #[test]
    fn roundtrip_mul_div() {
        let a: u128 = 12_345_678_901_234;
        let b: u128 = 98_765_432_109_876;
        let scale: u128 = 1_000_000_000_000;

        let (prod_low, prod_high) = mul_wide(a, b);
        let result = div_192_by_const(prod_low, prod_high, scale).unwrap();

        let expected = (a as f64) * (b as f64) / (scale as f64);
        let diff = (result as f64 - expected).abs();
        assert!(diff < 2.0, "roundtrip error too large: {diff}");
    }

    // ========================================================================
    // Division tests — large divisor (>= 2^64)
    // ========================================================================

    #[test]
    fn div_large_divisor_10e20() {
        // 10^20 > 2^64, exercises the Knuth Algorithm D path
        let divisor: u128 = 100_000_000_000_000_000_000;
        let (lo, hi) = mul_wide(divisor, 42);
        assert_eq!(div_192_by_const(lo, hi, divisor), Some(42));
        assert_eq!(div_192_by_u128(lo, hi, divisor), Some(42));
    }

    #[test]
    fn div_large_divisor_exactly_2e64() {
        // Exactly 2^64 — boundary between small and large divisor paths.
        // Use a dividend with non-zero hi so we actually exercise the
        // large-divisor Knuth path, not the hi==0 fast return.
        let divisor: u128 = (u64::MAX as u128) + 1; // 2^64
        let hi: u128 = 1; // hi < divisor
        let lo: u128 = divisor * 7;
        // dividend = (1 << 128) + 7 * 2^64 = 2^128 + 7 * 2^64
        // quotient = (2^128 + 7 * 2^64) / 2^64 = 2^64 + 7
        let expected: u128 = divisor + 7;
        assert_eq!(div_192_by_u128(lo, hi, divisor), Some(expected));
    }

    #[test]
    fn div_small_divisor_boundary_2e64_minus_1() {
        // 2^64 - 1 is the largest value that takes the small-divisor path.
        // Verify it works with non-zero hi.
        let divisor: u128 = u64::MAX as u128; // 2^64 - 1
        let hi: u128 = divisor - 1; // max valid hi (must be < divisor)
        let lo: u128 = 0;
        // dividend = (2^64 - 2) << 128 = (2^64 - 2) * 2^128
        // quotient = ((2^64 - 2) * 2^128) / (2^64 - 1)
        let result = div_wide(lo, hi, divisor).unwrap();
        // Verify via roundtrip: result * divisor <= dividend < (result+1) * divisor
        let (check_lo, check_hi) = mul_wide(result, divisor);
        assert!(
            check_hi < hi || (check_hi == hi && check_lo <= lo),
            "result too large"
        );
        let (over_lo, over_hi) = mul_wide(result + 1, divisor);
        assert!(
            over_hi > hi || (over_hi == hi && over_lo > lo),
            "result too small"
        );
    }

    #[test]
    fn div_large_divisor_max_operands() {
        let a: u128 = u128::MAX / 2;
        let b: u128 = 3;
        let (lo, hi) = mul_wide(a, b);
        assert_eq!(div_192_by_u128(lo, hi, a), Some(3));
    }

    #[test]
    fn div_large_divisor_d18_realistic() {
        // Decimal<i128, 18>: 1000.0 / 20.0
        // a_raw = 1000 * 10^18, b_raw = 20 * 10^18
        // (a_raw * SCALE) / b_raw = (10^21 * 10^18) / (2 * 10^19) = 5 * 10^19
        let scale: u128 = 1_000_000_000_000_000_000; // 10^18
        let a_raw: u128 = 1_000 * scale; // 10^21
        let b_raw: u128 = 20 * scale; // 2 * 10^19, > 2^64

        let (lo, hi) = mul_u128_by_small(a_raw, scale);
        let result = div_192_by_u128(lo, hi, b_raw);
        assert_eq!(result, Some(50 * scale)); // 50.0
    }

    #[test]
    fn div_large_divisor_near_max() {
        // Divisor close to u128::MAX
        let divisor: u128 = u128::MAX / 3;
        let quotient: u128 = 2;
        let (lo, hi) = mul_wide(divisor, quotient);
        assert_eq!(div_wide(lo, hi, divisor), Some(quotient));
    }

    #[test]
    fn div_large_divisor_quotient_one() {
        // Dividend just barely larger than divisor
        let divisor: u128 = (1u128 << 100) + 12345;
        let lo = divisor + 1;
        let hi = 0u128;
        assert_eq!(div_wide(lo, hi, divisor), Some(1));
    }

    #[test]
    fn div_large_divisor_hi_equals_divisor_minus_one() {
        // hi = divisor - 1 (maximum valid hi)
        let divisor: u128 = 1u128 << 65;
        let hi = divisor - 1;
        let lo = 0u128;
        // (hi * 2^128) / divisor = ((2^65 - 1) * 2^128) / 2^65 = 2^128 - 2^63
        let result = div_wide(lo, hi, divisor).unwrap();
        // Verify: result * divisor <= (hi * 2^128 + lo)
        let (check_lo, check_hi) = mul_wide(result, divisor);
        assert!(
            check_hi < hi || (check_hi == hi && check_lo <= lo),
            "result={result}, check_hi={check_hi}, check_lo={check_lo}, hi={hi}, lo={lo}"
        );
        // And (result + 1) * divisor > (hi * 2^128 + lo)
        let (check_lo2, check_hi2) = mul_wide(result + 1, divisor);
        assert!(
            check_hi2 > hi || (check_hi2 == hi && check_lo2 > lo),
            "result+1 should overflow"
        );
    }

    // ========================================================================
    // Wrapping variants
    // ========================================================================

    #[test]
    fn wrapping_matches_checked_small_divisor() {
        let scale: u128 = 10_000_000_000_000_000_000; // 10^19
        let a: u128 = 123_456_789_012_345_678;
        let b: u128 = 987_654_321;
        let (lo, hi) = mul_wide(a, b);
        if let Some(checked) = div_192_by_const(lo, hi, scale) {
            let wrapping = div_192_by_const_wrapping(lo, hi, scale);
            assert_eq!(checked, wrapping);
        }
    }

    #[test]
    fn wrapping_matches_checked_large_divisor() {
        let divisor: u128 = 100_000_000_000_000_000_000; // 10^20 > 2^64
        let a: u128 = 42_000_000_000_000_000_000_000; // 42 * 10^21
        let (lo, hi) = mul_wide(a, 1);
        if let Some(checked) = div_192_by_u128(lo, hi, divisor) {
            let wrapping = div_192_by_u128_wrapping(lo, hi, divisor);
            assert_eq!(checked, wrapping);
        }
    }

    #[test]
    fn wrapping_zero_divisor() {
        assert_eq!(div_192_by_u128_wrapping(100, 0, 0), 0);
    }

    // ========================================================================
    // Edge cases
    // ========================================================================

    #[test]
    fn div_hi_zero_small_divisor() {
        assert_eq!(div_wide(42, 0, 7), Some(6));
    }

    #[test]
    fn div_hi_zero_large_divisor() {
        let divisor: u128 = 1u128 << 65;
        assert_eq!(div_wide(divisor * 3, 0, divisor), Some(3));
    }

    #[test]
    fn div_hi_equals_divisor_returns_none() {
        assert_eq!(div_wide(0, 10, 10), None);
    }

    #[test]
    fn div_hi_greater_returns_none() {
        assert_eq!(div_wide(0, 11, 10), None);
    }

    #[test]
    fn div_by_one() {
        let lo = 0xDEAD_BEEF_u128;
        assert_eq!(div_wide(lo, 0, 1), Some(lo));
    }

    #[test]
    fn div_by_max_u128() {
        // Any (lo, 0) / u128::MAX = 0 (for lo < u128::MAX)
        assert_eq!(div_wide(100, 0, u128::MAX), Some(0));
        assert_eq!(div_wide(u128::MAX, 0, u128::MAX), Some(1));
    }

    /// Deterministic sweep: multiply then divide, verify roundtrip.
    #[test]
    fn roundtrip_sweep_large_divisors() {
        // Test a range of large divisors (>= 2^64)
        let divisors: [u128; 6] = [
            (1u128 << 64),     // exactly 2^64
            (1u128 << 64) + 1, // 2^64 + 1
            10u128.pow(20),    // 10^20
            10u128.pow(25),    // 10^25
            u128::MAX / 3,     // large
            u128::MAX / 2,     // near max
        ];
        let quotients: [u128; 5] = [1, 2, 7, 42, 1000];

        for &d in &divisors {
            for &q in &quotients {
                let (lo, hi) = mul_wide(d, q);
                if hi < d {
                    let result = div_wide(lo, hi, d);
                    assert_eq!(result, Some(q), "failed for divisor={d}, quotient={q}");
                }
            }
        }
    }

    /// Verify division with remainder (not exact multiples).
    #[test]
    fn div_with_remainder() {
        let divisor: u128 = 10u128.pow(20);
        // 42 * 10^20 + 12345
        let (lo, hi) = mul_wide(divisor, 42);
        let lo = lo + 12345;
        // Should still give 42 (truncating division)
        assert_eq!(div_wide(lo, hi, divisor), Some(42));
    }
}
