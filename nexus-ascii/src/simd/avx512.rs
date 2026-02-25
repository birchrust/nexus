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
/// Uses signed comparisons directly (no bias trick needed) since the printable
/// range [0x20, 0x7E] is entirely within the signed-positive byte space [0x00, 0x7F].
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

            // Signed comparison works correctly here because the printable range
            // [0x20, 0x7E] lies entirely within [0x00, 0x7F] — the signed-positive
            // half of the byte space. Bytes in [0x80, 0xFF] are interpreted as
            // negative (-128 to -1), which always fail the `chunk > 0x1F` check
            // (since -128..-1 < 31), so they're correctly flagged as invalid.
            //
            // This is simpler than the SSE2/AVX2 bias trick (XOR 0x80) but relies
            // on the target range being within the signed-positive half.
            let ge_low = _mm512_cmpgt_epi8_mask(chunk, lo); // chunk > 0x1F (signed)
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
// Numeric Validation
// =============================================================================

/// Check if all bytes are ASCII digits ('0'-'9') using AVX-512 (64 bytes at a time).
///
/// Cascades to AVX2 for inputs shorter than 64 bytes.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn is_all_numeric(bytes: &[u8]) -> bool {
    let len = bytes.len();
    if len < 64 {
        return avx2::is_all_numeric(bytes);
    }

    let mut i = 0;

    // SAFETY: AVX-512BW availability is guaranteed by target_feature cfg
    unsafe {
        // Range check: byte in ['0', '9']
        // Using signed comparison: byte > 0x2F AND byte <= 0x39
        let lo = _mm512_set1_epi8(0x2F_i8); // '0' - 1
        let hi = _mm512_set1_epi8(0x39_i8); // '9'

        // Process 64 bytes at a time
        while i + 64 <= len {
            let chunk = _mm512_loadu_si512(bytes.as_ptr().add(i).cast());

            // Check if each byte is in range ['0', '9']
            // is_digit = (byte > 0x2F) AND (byte <= 0x39)
            let gt_lo = _mm512_cmpgt_epi8_mask(chunk, lo);
            let le_hi = _mm512_cmple_epi8_mask(chunk, hi);
            let is_digit = gt_lo & le_hi;

            // All bytes must be digits (mask = all 1s = u64::MAX)
            if is_digit != u64::MAX {
                return false;
            }

            i += 64;
        }
    }

    // Handle remainder with AVX2 (which cascades to SSE2, then scalar)
    if i < len {
        return avx2::is_all_numeric(&bytes[i..]);
    }

    true
}

// =============================================================================
// Alphanumeric Validation
// =============================================================================

/// Check if all bytes are ASCII alphanumeric (0-9, A-Z, a-z) using AVX-512.
///
/// Cascades to AVX2 for inputs shorter than 64 bytes.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn is_all_alphanumeric(bytes: &[u8]) -> bool {
    let len = bytes.len();
    if len < 64 {
        return avx2::is_all_alphanumeric(bytes);
    }

    let mut i = 0;

    // SAFETY: AVX-512BW availability is guaranteed by target_feature cfg
    unsafe {
        // Digit range: ['0', '9'] = [0x30, 0x39]
        let digit_lo = _mm512_set1_epi8(0x2F_i8); // '0' - 1
        let digit_hi = _mm512_set1_epi8(0x39_i8); // '9'

        // Uppercase range: ['A', 'Z'] = [0x41, 0x5A]
        let upper_lo = _mm512_set1_epi8(0x40_i8); // 'A' - 1
        let upper_hi = _mm512_set1_epi8(0x5A_i8); // 'Z'

        // Lowercase range: ['a', 'z'] = [0x61, 0x7A]
        let lower_lo = _mm512_set1_epi8(0x60_i8); // 'a' - 1
        let lower_hi = _mm512_set1_epi8(0x7A_i8); // 'z'

        // Process 64 bytes at a time
        while i + 64 <= len {
            let chunk = _mm512_loadu_si512(bytes.as_ptr().add(i).cast());

            // Check digit: byte > 0x2F AND byte <= 0x39
            let is_digit =
                _mm512_cmpgt_epi8_mask(chunk, digit_lo) & _mm512_cmple_epi8_mask(chunk, digit_hi);

            // Check uppercase: byte > 0x40 AND byte <= 0x5A
            let is_upper =
                _mm512_cmpgt_epi8_mask(chunk, upper_lo) & _mm512_cmple_epi8_mask(chunk, upper_hi);

            // Check lowercase: byte > 0x60 AND byte <= 0x7A
            let is_lower =
                _mm512_cmpgt_epi8_mask(chunk, lower_lo) & _mm512_cmple_epi8_mask(chunk, lower_hi);

            // Alphanumeric = digit OR upper OR lower
            let is_alnum = is_digit | is_upper | is_lower;

            // All bytes must be alphanumeric (mask = all 1s = u64::MAX)
            if is_alnum != u64::MAX {
                return false;
            }

            i += 64;
        }
    }

    // Handle remainder with AVX2 (which cascades to SSE2, then scalar)
    if i < len {
        return avx2::is_all_alphanumeric(&bytes[i..]);
    }

    true
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::super::scalar;
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

    // -------------------------------------------------------------------------
    // Numeric validation
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_all_numeric_empty() {
        assert!(is_all_numeric(b""));
    }

    #[test]
    fn test_is_all_numeric_short() {
        assert!(is_all_numeric(b"12345"));
        assert!(!is_all_numeric(b"123a5"));
    }

    #[test]
    fn test_is_all_numeric_exact_64() {
        let bytes = b"0123456789012345678901234567890123456789012345678901234567890123";
        assert_eq!(bytes.len(), 64);
        assert!(is_all_numeric(bytes));

        let mut bytes = *b"0123456789012345678901234567890123456789012345678901234567890123";
        bytes[31] = b'a';
        assert!(!is_all_numeric(&bytes));
    }

    #[test]
    fn test_is_all_numeric_matches_scalar() {
        for len in 0..=160 {
            let bytes: Vec<u8> = (0..len).map(|i| b'0' + (i % 10) as u8).collect();
            assert_eq!(
                is_all_numeric(&bytes),
                scalar::is_all_numeric(&bytes),
                "len={}",
                len
            );
        }
    }

    #[test]
    fn test_is_all_numeric_invalid_matches_scalar() {
        for len in 1..=128 {
            for pos in (0..len).step_by(7) {
                let mut bytes = vec![b'5'; len];
                bytes[pos] = b'x';
                assert_eq!(
                    is_all_numeric(&bytes),
                    scalar::is_all_numeric(&bytes),
                    "len={}, pos={}",
                    len,
                    pos
                );
            }
        }
    }

    // -------------------------------------------------------------------------
    // Alphanumeric validation
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_all_alphanumeric_empty() {
        assert!(is_all_alphanumeric(b""));
    }

    #[test]
    fn test_is_all_alphanumeric_short() {
        assert!(is_all_alphanumeric(b"ABC123xyz"));
        assert!(!is_all_alphanumeric(b"ABC-123"));
    }

    #[test]
    fn test_is_all_alphanumeric_exact_64() {
        let bytes = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789AB";
        assert_eq!(bytes.len(), 64);
        assert!(is_all_alphanumeric(bytes));

        let mut bytes = *b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789AB";
        bytes[31] = b'-';
        assert!(!is_all_alphanumeric(&bytes));
    }

    #[test]
    fn test_is_all_alphanumeric_matches_scalar() {
        for len in 0..=160 {
            // Mix of digits, uppercase, lowercase
            let bytes: Vec<u8> = (0..len)
                .map(|i| match i % 3 {
                    0 => b'0' + (i % 10) as u8,
                    1 => b'A' + (i % 26) as u8,
                    _ => b'a' + (i % 26) as u8,
                })
                .collect();
            assert_eq!(
                is_all_alphanumeric(&bytes),
                scalar::is_all_alphanumeric(&bytes),
                "len={}",
                len
            );
        }
    }

    #[test]
    fn test_is_all_alphanumeric_invalid_matches_scalar() {
        for len in 1..=128 {
            for pos in (0..len).step_by(7) {
                let mut bytes = vec![b'A'; len];
                bytes[pos] = b'-';
                assert_eq!(
                    is_all_alphanumeric(&bytes),
                    scalar::is_all_alphanumeric(&bytes),
                    "len={}, pos={}",
                    len,
                    pos
                );
            }
        }
    }
}
