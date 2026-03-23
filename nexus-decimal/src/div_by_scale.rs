//! Fast division of i128 by a compile-time SCALE constant.
//!
//! When SCALE < 2^32, splits the i128 into three overlapping u64
//! chunks and divides each by SCALE using native u64 division. LLVM
//! optimizes each u64 division to a magic multiply + shift (~4-5
//! cycles each). Total: ~14 cycles vs ~25 for `__divti3`.
//!
//! When SCALE ≥ 2^32, falls back to native i128 division.
//!
//! The const-evaluated branch in the callers ensures LLVM eliminates
//! the dead path entirely — zero runtime cost for type selection.

/// Threshold for the chunked division fast path.
/// SCALE must be < 2^32 for remainders to fit in 32 bits.
pub(crate) const CHUNK_THRESHOLD: u64 = 1u64 << 32;

/// Divides a u128 by a u64 scale using three chained u64 divisions.
///
/// # Precondition
///
/// `scale < 2^32` (caller must verify via const branch).
///
/// # Correctness
///
/// Each remainder `r` satisfies `r < scale < 2^32`. Shifting left by
/// 32 and OR-ing with the next 32-bit piece gives a value < 2^64,
/// so each intermediate fits in u64.
#[inline(always)]
pub(crate) const fn div_u128_chunked(abs: u128, scale: u64) -> u128 {
    let hi = (abs >> 64) as u64;
    let lo = abs as u64;

    // Chunk 1: top 64 bits
    let q_hi = hi / scale;
    let r_hi = hi % scale;

    // Chunk 2: remainder + upper 32 bits of lo
    let mid = (r_hi << 32) | (lo >> 32);
    let q_mid = mid / scale;
    let r_mid = mid % scale;

    // Chunk 3: remainder + lower 32 bits of lo
    let bot = (r_mid << 32) | (lo as u32 as u64);
    let q_bot = bot / scale;

    ((q_hi as u128) << 64) | ((q_mid as u128) << 32) | (q_bot as u128)
}

/// Divides an i128 value by SCALE, using the fast chunked path when
/// possible. Returns `None` if the result doesn't fit in i64.
///
/// Caller passes `scale` and `use_chunked` (a const bool that LLVM
/// eliminates at compile time).
#[inline(always)]
pub(crate) const fn div_i128_by_scale(
    value: i128,
    scale: i128,
    scale_u64: u64,
    use_chunked: bool,
) -> Option<i64> {
    if value == 0 {
        return Some(0);
    }

    let is_negative = value < 0;
    let abs_value = value.unsigned_abs();

    let quotient = if use_chunked {
        div_u128_chunked(abs_value, scale_u64)
    } else {
        abs_value / (scale as u128)
    };

    // Bounds check accounting for sign: |i64::MIN| = i64::MAX + 1
    if is_negative {
        if quotient > (i64::MAX as u128) + 1 {
            return None;
        }
        // Negate via two's complement to handle i64::MIN correctly.
        // quotient as i64 would give i64::MIN when quotient = i64::MAX+1,
        // then -(i64::MIN) overflows. Instead: -(quotient as i128) as i64.
        Some((-(quotient as i128)) as i64)
    } else {
        if quotient > i64::MAX as u128 {
            return None;
        }
        Some(quotient as i64)
    }
}

/// Wrapping version — truncates the quotient to i64.
#[inline(always)]
pub(crate) const fn div_i128_by_scale_wrapping(
    value: i128,
    scale: i128,
    scale_u64: u64,
    use_chunked: bool,
) -> i64 {
    if value == 0 {
        return 0;
    }

    let is_negative = value < 0;
    let abs_value = value.unsigned_abs();

    let quotient = if use_chunked {
        div_u128_chunked(abs_value, scale_u64)
    } else {
        abs_value / (scale as u128)
    };

    let quotient = quotient as i64;

    if is_negative {
        quotient.wrapping_neg()
    } else {
        quotient
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;

    use super::*;

    /// Verify chunked division matches native for D64 scale (10^8).
    #[test]
    fn chunked_matches_native_d64() {
        let scale: u64 = 100_000_000;

        // Deterministic sweep of i64*i64 products
        let mut rng = 42u64;
        for _ in 0..100_000 {
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let a = rng as i64;
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let b = rng as i64;

            let product = (a as i128) * (b as i128);
            let abs = product.unsigned_abs();

            let native = abs / (scale as u128);
            let chunked = div_u128_chunked(abs, scale);

            assert_eq!(
                chunked, native,
                "mismatch for a={a}, b={b}, product={product}"
            );
        }
    }

    /// Verify chunked division for all valid i64 scales where SCALE < 2^32.
    #[test]
    fn chunked_matches_native_all_small_scales() {
        // D=1..9 for i64 (all have SCALE < 2^32)
        let scales: Vec<u64> = (1..=9)
            .map(|d| {
                let mut s: u64 = 1;
                for _ in 0..d {
                    s *= 10;
                }
                s
            })
            .collect();

        for &scale in &scales {
            assert!(scale < CHUNK_THRESHOLD, "scale {scale} >= 2^32");

            let mut rng = scale.wrapping_mul(7919);
            for _ in 0..10_000 {
                rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                let a = rng as i64;
                rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                let b = rng as i64;

                let product = (a as i128) * (b as i128);
                let abs = product.unsigned_abs();

                let native = abs / (scale as u128);
                let chunked = div_u128_chunked(abs, scale);

                assert_eq!(chunked, native, "mismatch for scale={scale}, a={a}, b={b}");
            }
        }
    }

    /// Edge cases: zero, one, MAX products.
    #[test]
    fn chunked_edge_cases() {
        let scale: u64 = 100_000_000;

        assert_eq!(div_u128_chunked(0, scale), 0);
        assert_eq!(div_u128_chunked(1, scale), 0);
        assert_eq!(div_u128_chunked(scale as u128, scale), 1);
        assert_eq!(div_u128_chunked(scale as u128 - 1, scale), 0);

        // Max i64 * i64 product
        let max_product = (i64::MAX as u128) * (i64::MAX as u128);
        let expected = max_product / (scale as u128);
        assert_eq!(div_u128_chunked(max_product, scale), expected);

        // i64::MAX * SCALE (should give i64::MAX)
        let exact_product = (i64::MAX as u128) * (scale as u128);
        assert_eq!(div_u128_chunked(exact_product, scale), i64::MAX as u128);
    }

    /// Test the signed wrapper with negative values.
    #[test]
    fn signed_wrapper_negative() {
        let scale = 100_000_000i128;
        let scale_u64 = 100_000_000u64;

        // Positive
        let result = div_i128_by_scale(300_000_000, scale, scale_u64, true);
        assert_eq!(result, Some(3));

        // Negative
        let result = div_i128_by_scale(-300_000_000, scale, scale_u64, true);
        assert_eq!(result, Some(-3));

        // Zero
        let result = div_i128_by_scale(0, scale, scale_u64, true);
        assert_eq!(result, Some(0));

        // Overflow
        let huge = (i64::MAX as i128) * (i64::MAX as i128);
        assert!(div_i128_by_scale(huge, scale, scale_u64, true).is_none());
    }

    /// Verify chunked path and native path give same result through the wrapper.
    #[test]
    fn chunked_vs_native_through_wrapper() {
        let scale = 100_000_000i128;
        let scale_u64 = 100_000_000u64;

        let mut rng = 99u64;
        for _ in 0..50_000 {
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let a = rng as i64;
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let b = rng as i64;

            let product = (a as i128) * (b as i128);

            let chunked = div_i128_by_scale(product, scale, scale_u64, true);
            let native = div_i128_by_scale(product, scale, scale_u64, false);

            assert_eq!(chunked, native, "mismatch for a={a}, b={b}");
        }
    }
}
