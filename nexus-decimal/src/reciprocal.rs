//! Reciprocal constants for future optimization of division by SCALE.
//!
//! **Current status:** NOT used in hot paths. Phase 1-3 use native i128
//! division for correctness. The Barrett reciprocal with a 64-bit multiplier
//! has errors > 1 ULP for i128 inputs with significant upper bits (the
//! approximation `hi * M + (lo * M >> 64)` diverges from the true
//! `(x * M) >> 64` when `hi` is non-trivial).
//!
//! **Phase 5 optimization:** Implement a proper 128-bit Barrett reduction
//! using a u128 multiplier with 128×128→256 bit intermediate (4 limb
//! multiplies). This gives correct results for all i128 inputs. Only
//! add when cargo-asm confirms __divti3 is a bottleneck.
//!
//! The constants are computed and tested here so they're ready for Phase 5.

/// Precomputed reciprocal for dividing by a power-of-10 scale factor.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Reciprocal {
    /// `ceil(2^64 / SCALE)` — 64-bit approximation.
    pub mul: u64,
}

/// Computes the reciprocal constant `ceil(2^64 / scale)`.
#[cfg_attr(not(test), allow(dead_code))]
///
/// For `scale == 1`, returns `mul: 0` as a sentinel.
pub(crate) const fn compute_reciprocal(scale: u64) -> Reciprocal {
    assert!(scale > 0, "scale must be positive");
    if scale == 1 {
        return Reciprocal { mul: 0 };
    }
    let pow2_64: u128 = 1u128 << 64;
    let s = scale as u128;
    let div = pow2_64 / s;
    let rem = pow2_64 % s;
    let mul = if rem == 0 {
        div as u64
    } else {
        (div + 1) as u64
    };
    Reciprocal { mul }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pow10::pow10_i64;

    #[test]
    fn reciprocal_for_d64_matches_fixdec() {
        let recip = compute_reciprocal(100_000_000);
        assert_eq!(recip.mul, 184_467_440_738);
    }

    #[test]
    fn reciprocal_d0_is_sentinel() {
        let recip = compute_reciprocal(1);
        assert_eq!(recip.mul, 0);
    }

    #[test]
    fn reciprocal_ceil_property() {
        for d in 1..=18u8 {
            let scale = pow10_i64(d) as u64;
            let recip = compute_reciprocal(scale);
            let product = (recip.mul as u128) * (scale as u128);
            assert!(
                product >= (1u128 << 64),
                "reciprocal for 10^{d} undershoots: M*S = {product}"
            );
        }
    }
}
