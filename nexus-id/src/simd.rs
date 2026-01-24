//! SIMD-accelerated hex encode/decode.
//!
//! Provides hex encoding and decoding with automatic dispatch to the best
//! available implementation based on compile-time target features:
//!
//! ## Encode (byte → hex chars)
//!
//! - SSSE3: `pshufb` as a 16-entry LUT (replaces 256-byte HEX_TABLE)
//! - Scalar: lookup table, 8 bytes at a time
//!
//! ## Decode (hex chars → bytes)
//!
//! - SSE2: parallel range classification + nibble packing (x86_64 baseline)
//! - Scalar: per-byte match + accumulate
//!
//! ## Usage
//!
//! ```bash
//! # Default (SSE2 decode on x86_64, scalar encode)
//! cargo build --release
//!
//! # SSSE3 encode (most modern x86_64 CPUs, Core 2+)
//! RUSTFLAGS="-C target-feature=+ssse3" cargo build --release
//!
//! # Native (auto-detect CPU features)
//! RUSTFLAGS="-C target-cpu=native" cargo build --release
//! ```

// =============================================================================
// Implementations
// =============================================================================

mod scalar;

// SSE2: baseline x86_64 (always available)
#[cfg(target_arch = "x86_64")]
mod sse2;

// SSSE3: pshufb for hex encode (Core 2+, ~2006)
#[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))]
mod ssse3;

// =============================================================================
// Hex Encode
// =============================================================================

/// Encode u64 as 16 lowercase hex bytes.
#[inline]
pub(crate) fn hex_encode_u64(value: u64) -> [u8; 16] {
    #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))]
    {
        ssse3::hex_encode_u64(value)
    }

    #[cfg(all(target_arch = "x86_64", not(target_feature = "ssse3")))]
    {
        scalar::hex_encode_u64(value)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::hex_encode_u64(value)
    }
}

/// Encode two u64s as 32 lowercase hex bytes.
#[inline]
pub(crate) fn hex_encode_u128(hi: u64, lo: u64) -> [u8; 32] {
    #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))]
    {
        ssse3::hex_encode_u128(hi, lo)
    }

    #[cfg(all(target_arch = "x86_64", not(target_feature = "ssse3")))]
    {
        scalar::hex_encode_u128(hi, lo)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::hex_encode_u128(hi, lo)
    }
}

// =============================================================================
// Hex Decode
// =============================================================================

/// Decode 16 hex chars to u64.
///
/// Returns `Err(position)` on first invalid character.
#[inline]
pub(crate) fn hex_decode_16(bytes: &[u8; 16]) -> Result<u64, usize> {
    #[cfg(target_arch = "x86_64")]
    {
        sse2::hex_decode_16(bytes)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::hex_decode_16(bytes)
    }
}

/// Decode 32 hex chars to (hi, lo) u64 pair.
///
/// Returns `Err(position)` on first invalid character.
#[inline]
pub(crate) fn hex_decode_32(bytes: &[u8; 32]) -> Result<(u64, u64), usize> {
    #[cfg(target_arch = "x86_64")]
    {
        sse2::hex_decode_32(bytes)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::hex_decode_32(bytes)
    }
}

// =============================================================================
// UUID Dashed Decode
// =============================================================================

/// Decode a 36-byte dashed UUID string to (hi, lo) u64 pair.
///
/// Caller must have validated: `bytes.len() == 36` and dashes at positions 8,13,18,23.
/// Returns `Err(position)` in the compacted 32-char hex space on first invalid character.
#[inline]
pub(crate) fn uuid_parse_dashed(bytes: &[u8; 36]) -> Result<(u64, u64), usize> {
    #[cfg(all(target_arch = "x86_64", target_feature = "ssse3"))]
    {
        ssse3::uuid_decode_dashed(bytes)
    }

    #[cfg(all(target_arch = "x86_64", not(target_feature = "ssse3")))]
    {
        // Buffer compaction: strip dashes into contiguous 32-byte buffer, SSE2 decode.
        let mut buf = [0u8; 32];
        buf[0..8].copy_from_slice(&bytes[0..8]);
        buf[8..12].copy_from_slice(&bytes[9..13]);
        buf[12..16].copy_from_slice(&bytes[14..18]);
        buf[16..20].copy_from_slice(&bytes[19..23]);
        buf[20..32].copy_from_slice(&bytes[24..36]);
        hex_decode_32(&buf)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        // Buffer compaction + scalar decode.
        let mut buf = [0u8; 32];
        buf[0..8].copy_from_slice(&bytes[0..8]);
        buf[8..12].copy_from_slice(&bytes[9..13]);
        buf[12..16].copy_from_slice(&bytes[14..18]);
        buf[16..20].copy_from_slice(&bytes[19..23]);
        buf[20..32].copy_from_slice(&bytes[24..36]);
        hex_decode_32(&buf)
    }
}
