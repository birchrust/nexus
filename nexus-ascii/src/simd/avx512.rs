//! AVX-512 SIMD validation (64 bytes at a time).
//!
//! Available when compiled with `-C target-feature=+avx512bw`.
//! Note: Requires `avx512bw` (byte/word operations), not just `avx512f`.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use super::avx2;

// =============================================================================
// ASCII Validation
// =============================================================================

/// Validate that all bytes are ASCII (< 128) using AVX-512.
///
/// Processes 64 bytes at a time using `_mm512_movepi8_mask` to check high bits.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn validate_ascii(bytes: &[u8]) -> Result<(), (u8, usize)> {
    let len = bytes.len();
    let mut i = 0;

    // SAFETY: AVX-512BW availability is guaranteed by target_feature cfg
    unsafe {
        // Process 64 bytes at a time
        while i + 64 <= len {
            let chunk = _mm512_loadu_si512(bytes.as_ptr().add(i).cast());
            // movepi8_mask extracts the high bit of each byte into a 64-bit mask
            // This is cleaner than AVX2's movemask which returns i32
            let mask = _mm512_movepi8_mask(chunk);
            if mask != 0 {
                // Found non-ASCII byte(s) - find the first one
                let offset = mask.trailing_zeros() as usize;
                let pos = i + offset;
                return Err((bytes[pos], pos));
            }
            i += 64;
        }
    }

    // Handle remainder with AVX2 (which cascades to SSE2, then scalar)
    if i < len {
        avx2::validate_ascii(&bytes[i..]).map_err(|(b, p)| (b, i + p))?;
    }

    Ok(())
}

// =============================================================================
// Printable Validation
// =============================================================================

/// Validate that all bytes are printable ASCII (0x20-0x7E) using AVX-512.
///
/// Uses mask registers for efficient range checking.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn validate_printable(bytes: &[u8]) -> Result<(), (u8, usize)> {
    let len = bytes.len();
    let mut i = 0;

    // SAFETY: AVX-512BW availability is guaranteed by target_feature cfg
    unsafe {
        // Broadcast bounds for comparison
        let lo = _mm512_set1_epi8(0x1F_i8); // 0x20 - 1
        let hi = _mm512_set1_epi8(0x7E_i8);

        // Process 64 bytes at a time
        while i + 64 <= len {
            let chunk = _mm512_loadu_si512(bytes.as_ptr().add(i).cast());

            // AVX-512 has direct unsigned comparison with mask output
            // cmpgt returns mask where chunk[i] > lo (i.e., chunk[i] >= 0x20)
            // cmple returns mask where chunk[i] <= hi (i.e., chunk[i] <= 0x7E)
            //
            // We want to find bytes OUTSIDE the range, so:
            // - below_range: chunk <= 0x1F (not > 0x1F)
            // - above_range: chunk > 0x7E
            //
            // Using unsigned comparison (_mm512_cmpgt_epu8_mask) for correct results
            let ge_low = _mm512_cmpgt_epi8_mask(chunk, lo); // chunk > 0x1F (signed, works for 0x00-0x7F)
            let le_high = _mm512_cmple_epi8_mask(chunk, hi); // chunk <= 0x7E (signed)

            // Valid if in range [0x20, 0x7E]: both conditions must be true
            // Invalid mask = NOT (ge_low AND le_high)
            let valid = ge_low & le_high;
            let invalid = !valid;

            if invalid != 0 {
                let offset = invalid.trailing_zeros() as usize;
                let pos = i + offset;
                return Err((bytes[pos], pos));
            }

            i += 64;
        }
    }

    // Handle remainder with AVX2 (which cascades to SSE2, then scalar)
    if i < len {
        avx2::validate_printable(&bytes[i..]).map_err(|(b, p)| (b, i + p))?;
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::scalar;

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
    fn test_validate_ascii_exact_64() {
        let bytes = b"0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        assert_eq!(bytes.len(), 64);
        assert!(validate_ascii(bytes).is_ok());
    }

    #[test]
    fn test_validate_ascii_over_64() {
        let bytes = b"0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEFGHIJ";
        assert_eq!(bytes.len(), 68);
        assert!(validate_ascii(bytes).is_ok());
    }

    #[test]
    fn test_validate_ascii_invalid_in_first_64() {
        let mut bytes = [b'a'; 64];
        bytes[31] = 0x80;
        assert_eq!(validate_ascii(&bytes), Err((0x80, 31)));
    }

    #[test]
    fn test_validate_ascii_invalid_in_remainder() {
        let mut bytes = [b'a'; 70];
        bytes[65] = 0x80;
        assert_eq!(validate_ascii(&bytes), Err((0x80, 65)));
    }

    #[test]
    fn test_validate_ascii_all_positions() {
        for pos in 0..128 {
            let mut bytes = vec![b'a'; 128];
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
    fn test_validate_printable_exact_64() {
        // 64 printable characters
        let bytes: Vec<u8> = (0..64).map(|i| b' ' + (i % 95) as u8).collect();
        assert!(validate_printable(&bytes).is_ok());
    }

    #[test]
    fn test_validate_printable_all_printable_chars() {
        let printable: Vec<u8> = (0x20..=0x7E).collect();
        assert!(validate_printable(&printable).is_ok());
    }

    #[test]
    fn test_validate_printable_control_rejected() {
        assert_eq!(validate_printable(&[0x00]), Err((0x00, 0)));
        assert_eq!(validate_printable(&[0x09]), Err((0x09, 0)));
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
        let mut bytes = [b'a'; 64];
        bytes[31] = 0x00;
        assert_eq!(validate_printable(&bytes), Err((0x00, 31)));
    }

    #[test]
    fn test_validate_printable_invalid_in_remainder() {
        let mut bytes = [b'a'; 70];
        bytes[66] = 0x7F;
        assert_eq!(validate_printable(&bytes), Err((0x7F, 66)));
    }

    #[test]
    fn test_validate_printable_boundary_values() {
        assert!(validate_printable(&[0x20]).is_ok());
        assert!(validate_printable(&[0x7E]).is_ok());
        assert_eq!(validate_printable(&[0x1F]), Err((0x1F, 0)));
        assert_eq!(validate_printable(&[0x7F]), Err((0x7F, 0)));
    }

    // -------------------------------------------------------------------------
    // Consistency with scalar
    // -------------------------------------------------------------------------

    #[test]
    fn test_ascii_matches_scalar() {
        for len in 0..=160 {
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
        for len in 0..=160 {
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
        for len in 1..=128 {
            for pos in (0..len).step_by(7) {
                // Step by 7 to reduce test time
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
