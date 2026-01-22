//! SSE2 SIMD validation (16 bytes at a time).
//!
//! Available on all x86_64 targets (SSE2 is baseline).

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use super::scalar;

// =============================================================================
// ASCII Validation
// =============================================================================

/// Validate that all bytes are ASCII (< 128) using SSE2.
///
/// Processes 16 bytes at a time using `_mm_movemask_epi8` to check high bits.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn validate_ascii(bytes: &[u8]) -> Result<(), (u8, usize)> {
    let len = bytes.len();
    let mut i = 0;

    // Process 16 bytes at a time
    while i + 16 <= len {
        // SAFETY: SSE2 is baseline for x86_64, bounds checked above
        unsafe {
            let chunk = _mm_loadu_si128(bytes.as_ptr().add(i).cast());
            // movemask extracts the high bit of each byte into a 16-bit mask
            let mask = _mm_movemask_epi8(chunk);
            if mask != 0 {
                // Found non-ASCII byte(s) - find the first one
                let offset = mask.trailing_zeros() as usize;
                let pos = i + offset;
                return Err((bytes[pos], pos));
            }
        }
        i += 16;
    }

    // Handle remainder with scalar
    if i < len {
        scalar::validate_ascii(&bytes[i..]).map_err(|(b, p)| (b, i + p))?;
    }

    Ok(())
}

// =============================================================================
// Printable Validation
// =============================================================================

/// Validate that all bytes are printable ASCII (0x20-0x7E) using SSE2.
///
/// Uses the bias trick to convert unsigned comparisons to signed:
/// XOR with 0x80 maps [0,255] to signed [-128,127], then use signed compares.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn validate_printable(bytes: &[u8]) -> Result<(), (u8, usize)> {
    let len = bytes.len();
    let mut i = 0;

    // SAFETY: SSE2 is baseline for x86_64
    unsafe {
        // Bias value: XOR with 0x80 to convert unsigned to signed
        let bias = _mm_set1_epi8(-128i8); // 0x80 as signed

        // After biasing:
        // - 0x20 (space) becomes 0xA0 = -96 (signed)
        // - 0x7E (tilde) becomes 0xFE = -2 (signed)
        // We want to reject bytes with biased value < -96 or > -2

        // Low bound: 0x20 - 1 = 0x1F, biased = 0x9F = -97
        let lo_bound = _mm_set1_epi8(-97i8);
        // High bound: 0x7E, biased = 0xFE = -2
        let hi_bound = _mm_set1_epi8(-2i8);

        // Process 16 bytes at a time
        while i + 16 <= len {
            let chunk = _mm_loadu_si128(bytes.as_ptr().add(i).cast());

            // XOR with bias to convert to signed range
            let biased = _mm_xor_si128(chunk, bias);

            // Check for bytes outside [0x20, 0x7E]:
            // - cmpgt(lo_bound, biased) catches bytes < 0x20 (biased < -96, so -97 > biased)
            // - cmpgt(biased, hi_bound) catches bytes > 0x7E (biased > -2)
            let below = _mm_cmpgt_epi8(lo_bound, biased);
            let above = _mm_cmpgt_epi8(biased, hi_bound);
            let invalid = _mm_or_si128(below, above);

            let mask = _mm_movemask_epi8(invalid);
            if mask != 0 {
                let offset = mask.trailing_zeros() as usize;
                let pos = i + offset;
                return Err((bytes[pos], pos));
            }

            i += 16;
        }
    }

    // Handle remainder with scalar
    if i < len {
        scalar::validate_printable(&bytes[i..]).map_err(|(b, p)| (b, i + p))?;
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // ASCII validation
    // -------------------------------------------------------------------------

    #[test]
    fn test_validate_ascii_empty() {
        assert!(validate_ascii(b"").is_ok());
    }

    #[test]
    fn test_validate_ascii_short() {
        assert!(validate_ascii(b"Hello").is_ok());
        assert!(validate_ascii(b"Hello, World!").is_ok());
    }

    #[test]
    fn test_validate_ascii_exact_16() {
        assert!(validate_ascii(b"0123456789ABCDEF").is_ok());
    }

    #[test]
    fn test_validate_ascii_over_16() {
        assert!(validate_ascii(b"0123456789ABCDEFGHIJ").is_ok());
    }

    #[test]
    fn test_validate_ascii_invalid_in_first_16() {
        let mut bytes = *b"0123456789ABCDEF";
        bytes[5] = 0x80;
        assert_eq!(validate_ascii(&bytes), Err((0x80, 5)));
    }

    #[test]
    fn test_validate_ascii_invalid_in_remainder() {
        let mut bytes = *b"0123456789ABCDEFGH";
        bytes[17] = 0x80;
        assert_eq!(validate_ascii(&bytes), Err((0x80, 17)));
    }

    #[test]
    fn test_validate_ascii_all_positions() {
        for pos in 0..32 {
            let mut bytes = vec![b'a'; 32];
            bytes[pos] = 0xFF;
            assert_eq!(validate_ascii(&bytes), Err((0xFF, pos)), "pos={}", pos);
        }
    }

    // -------------------------------------------------------------------------
    // Printable validation
    // -------------------------------------------------------------------------

    #[test]
    fn test_validate_printable_empty() {
        assert!(validate_printable(b"").is_ok());
    }

    #[test]
    fn test_validate_printable_short() {
        assert!(validate_printable(b"Hello").is_ok());
        assert!(validate_printable(b" ~").is_ok());
    }

    #[test]
    fn test_validate_printable_exact_16() {
        assert!(validate_printable(b"Hello, World!   ").is_ok());
    }

    #[test]
    fn test_validate_printable_all_printable_chars() {
        // All 95 printable ASCII chars
        let printable: Vec<u8> = (0x20..=0x7E).collect();
        assert!(validate_printable(&printable).is_ok());
    }

    #[test]
    fn test_validate_printable_control_rejected() {
        // Null
        assert_eq!(validate_printable(&[0x00]), Err((0x00, 0)));
        // Tab
        assert_eq!(validate_printable(&[0x09]), Err((0x09, 0)));
        // Unit separator (just below space)
        assert_eq!(validate_printable(&[0x1F]), Err((0x1F, 0)));
    }

    #[test]
    fn test_validate_printable_del_rejected() {
        assert_eq!(validate_printable(&[0x7F]), Err((0x7F, 0)));
    }

    #[test]
    fn test_validate_printable_high_ascii_rejected() {
        assert_eq!(validate_printable(&[0x80]), Err((0x80, 0)));
        assert_eq!(validate_printable(&[0xFF]), Err((0xFF, 0)));
    }

    #[test]
    fn test_validate_printable_invalid_in_simd_chunk() {
        let mut bytes = *b"Hello, World!   ";
        bytes[5] = 0x00;
        assert_eq!(validate_printable(&bytes), Err((0x00, 5)));
    }

    #[test]
    fn test_validate_printable_invalid_in_remainder() {
        let mut bytes = *b"Hello, World! abc";
        bytes[16] = 0x7F;
        assert_eq!(validate_printable(&bytes), Err((0x7F, 16)));
    }

    #[test]
    fn test_validate_printable_boundary_values() {
        // Test exact boundaries
        assert!(validate_printable(&[0x20]).is_ok()); // space
        assert!(validate_printable(&[0x7E]).is_ok()); // tilde
        assert_eq!(validate_printable(&[0x1F]), Err((0x1F, 0))); // unit sep
        assert_eq!(validate_printable(&[0x7F]), Err((0x7F, 0))); // DEL
    }

    // -------------------------------------------------------------------------
    // Consistency with scalar
    // -------------------------------------------------------------------------

    #[test]
    fn test_ascii_matches_scalar() {
        for len in 0..=64 {
            let bytes: Vec<u8> = (0..len).map(|i| (i % 128) as u8).collect();
            assert_eq!(
                validate_ascii(&bytes),
                scalar::validate_ascii(&bytes),
                "len={}",
                len
            );
        }
    }

    #[test]
    fn test_printable_matches_scalar() {
        for len in 0..=64 {
            let bytes: Vec<u8> = (0..len).map(|i| (0x20 + (i % 95)) as u8).collect();
            assert_eq!(
                validate_printable(&bytes),
                scalar::validate_printable(&bytes),
                "len={}",
                len
            );
        }
    }

    #[test]
    fn test_printable_invalid_matches_scalar() {
        for len in 1..=64 {
            for pos in 0..len {
                let mut bytes = vec![b'a'; len];
                bytes[pos] = 0x00;
                assert_eq!(
                    validate_printable(&bytes),
                    scalar::validate_printable(&bytes),
                    "len={}, pos={}",
                    len,
                    pos
                );
            }
        }
    }
}
