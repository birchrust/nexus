//! Hash function for AsciiString.
//!
//! Provides a single `hash<CAP>` function that automatically selects the best
//! XXH3 implementation based on compile-time target features:
//!
//! - AVX-512 if `target_feature = "avx512f"` is enabled
//! - AVX2 if `target_feature = "avx2"` is enabled
//! - SSE2 on x86_64 (always available)
//! - Scalar on other architectures (ARM, etc.)
//!
//! ## Usage
//!
//! ```bash
//! # Default (SSE2 on x86_64, scalar elsewhere)
//! cargo build --release
//!
//! # AVX2
//! RUSTFLAGS="-C target-feature=+avx2" cargo build --release
//!
//! # AVX-512
//! RUSTFLAGS="-C target-feature=+avx512f" cargo build --release
//!
//! # Native (auto-detect CPU features)
//! RUSTFLAGS="-C target-cpu=native" cargo build --release
//! ```

// =============================================================================
// Implementations (internal)
// =============================================================================

mod xxh3;

#[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
mod xxh3_sse2;

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
mod xxh3_avx2;

#[cfg(all(target_arch = "x86_64", target_feature = "avx512f"))]
mod xxh3_avx512;

// =============================================================================
// Main entry point
// =============================================================================

/// Hash bytes using XXH3 with compile-time capacity bound.
///
/// The capacity bound allows the compiler to eliminate unreachable code paths.
/// For example, `hash::<32>(data)` will only generate code for inputs up to
/// 32 bytes, eliminating medium (129-240) and large (>240) paths entirely.
///
/// Returns a 64-bit hash. Caller truncates to 48 bits as needed.
#[inline]
pub fn hash<const CAP: usize>(data: &[u8]) -> u64 {
    hash_with_seed::<CAP>(data, 0)
}

/// Hash bytes with a seed.
#[inline]
pub fn hash_with_seed<const CAP: usize>(data: &[u8], seed: u64) -> u64 {
    // AVX-512: best for large inputs
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512f"))]
    {
        xxh3_avx512::hash_bounded_with_seed::<CAP>(data, seed)
    }

    // AVX2: great for large inputs
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2", not(target_feature = "avx512f")))]
    {
        xxh3_avx2::hash_bounded_with_seed::<CAP>(data, seed)
    }

    // SSE2: baseline x86_64
    #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
    {
        xxh3_sse2::hash_bounded_with_seed::<CAP>(data, seed)
    }

    // Scalar: all other architectures
    #[cfg(not(target_arch = "x86_64"))]
    {
        xxh3::hash_bounded_with_seed::<CAP>(data, seed)
    }
}

// =============================================================================
// Const hash (for compile-time evaluation)
// =============================================================================

/// Const-compatible hash for compile-time evaluation.
///
/// This function can be used in const contexts (e.g., `const fn from_static`).
/// It uses a scalar implementation that produces identical hashes to the
/// runtime SIMD versions.
///
/// Only supports inputs up to 128 bytes, which covers all practical use cases
/// for compile-time string literals.
///
/// # Panics
///
/// Panics at compile time if `CAP > 128`.
#[inline]
pub const fn hash_const<const CAP: usize>(data: &[u8]) -> u64 {
    xxh3::hash_const::<CAP>(data, 0)
}

/// Const-compatible hash with seed.
#[inline]
pub const fn hash_const_with_seed<const CAP: usize>(data: &[u8], seed: u64) -> u64 {
    xxh3::hash_const::<CAP>(data, seed)
}

// =============================================================================
// Truncation helpers
// =============================================================================

/// Truncate a 64-bit hash to 48 bits (lower bits).
#[inline(always)]
pub const fn truncate_lower_48(h: u64) -> [u8; 6] {
    [
        h as u8,
        (h >> 8) as u8,
        (h >> 16) as u8,
        (h >> 24) as u8,
        (h >> 32) as u8,
        (h >> 40) as u8,
    ]
}

/// Truncate a 64-bit hash to 48 bits (upper bits).
#[inline(always)]
pub const fn truncate_upper_48(h: u64) -> [u8; 6] {
    [
        (h >> 16) as u8,
        (h >> 24) as u8,
        (h >> 32) as u8,
        (h >> 40) as u8,
        (h >> 48) as u8,
        (h >> 56) as u8,
    ]
}

/// Reconstruct u64 from 48-bit hash (for Hash trait impl).
#[inline(always)]
pub const fn expand_48_to_64(h: [u8; 6]) -> u64 {
    (h[0] as u64)
        | ((h[1] as u64) << 8)
        | ((h[2] as u64) << 16)
        | ((h[3] as u64) << 24)
        | ((h[4] as u64) << 32)
        | ((h[5] as u64) << 40)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_deterministic() {
        let data = b"hello world";
        let h1 = hash::<32>(data);
        let h2 = hash::<32>(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_different_inputs() {
        let h1 = hash::<32>(b"hello");
        let h2 = hash::<32>(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_matches_scalar() {
        // Verify our dispatch produces same results as scalar reference
        for len in 0..=128 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let h_dispatch = hash::<128>(&data);
            let h_scalar = xxh3::hash_bounded::<128>(&data);
            assert_eq!(h_dispatch, h_scalar, "mismatch at len={}", len);
        }
    }

    #[test]
    fn hash_with_seed_works() {
        let data = b"test";
        let h1 = hash_with_seed::<32>(data, 0);
        let h2 = hash_with_seed::<32>(data, 12345);
        assert_ne!(h1, h2);
    }

    #[test]
    fn truncate_roundtrip() {
        let original: u64 = 0x123456789ABC;
        let truncated = truncate_lower_48(original);
        let expanded = expand_48_to_64(truncated);
        assert_eq!(expanded, original);
    }

    #[test]
    fn truncate_upper_captures_high_bits() {
        let h: u64 = 0xAABBCCDDEEFF1122;
        let upper = truncate_upper_48(h);
        assert_eq!(upper[5], 0xAA);
        assert_eq!(upper[4], 0xBB);
        assert_eq!(upper[3], 0xCC);
    }

    #[test]
    fn hash_const_matches_runtime_dispatch() {
        // Verify const hash matches the dispatched runtime hash
        // (which may use SIMD on x86_64)
        for len in 0..=128 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let h_const = hash_const::<128>(&data);
            let h_runtime = hash::<128>(&data);
            assert_eq!(h_const, h_runtime, "mismatch at len={}", len);
        }
    }

    #[test]
    fn hash_const_usable_in_const_context() {
        const H1: u64 = hash_const::<32>(b"hello");
        const H2: u64 = hash_const::<32>(b"world");

        assert_ne!(H1, H2);
        assert_eq!(H1, hash::<32>(b"hello"));
        assert_eq!(H2, hash::<32>(b"world"));
    }
}
