//! Scalar SWAR (SIMD Within A Register) validation.
//!
//! Processes 8 bytes at a time using u64 arithmetic. This is the fallback
//! implementation for non-x86_64 architectures and serves as the reference
//! implementation for testing SIMD versions.

// =============================================================================
// ASCII Validation
// =============================================================================

/// Validate that all bytes are ASCII (< 128) using SWAR.
///
/// Processes 8 bytes at a time by checking if any byte has its high bit set.
#[inline]
pub fn validate_ascii(bytes: &[u8]) -> Result<(), (u8, usize)> {
    const HI: u64 = 0x8080_8080_8080_8080;
    let mut i = 0;

    // Process 8 bytes at a time - just check if any high bit is set
    while i + 8 <= bytes.len() {
        // SAFETY: We just checked that i + 8 <= bytes.len()
        let chunk: [u8; 8] = unsafe {
            bytes
                .as_ptr()
                .add(i)
                .cast::<[u8; 8]>()
                .read_unaligned()
        };
        let word = u64::from_ne_bytes(chunk);
        let mask = word & HI;
        if mask != 0 {
            // Found non-ASCII - use trailing_zeros to find first invalid byte
            let offset = (mask.trailing_zeros() / 8) as usize;
            let pos = i + offset;
            return Err((bytes[pos], pos));
        }
        i += 8;
    }

    // Handle remainder byte by byte
    while i < bytes.len() {
        if bytes[i] > 127 {
            return Err((bytes[i], i));
        }
        i += 1;
    }

    Ok(())
}

// =============================================================================
// Printable Validation
// =============================================================================

/// Check if any byte in a u64 is outside printable ASCII range (0x20-0x7E).
/// Returns a non-zero mask if any byte is non-printable.
///
/// Uses SWAR technique:
/// - Catches bytes < 0x20 (control characters)
/// - Catches bytes >= 0x7F (DEL and non-ASCII)
#[inline(always)]
const fn has_non_printable(word: u64) -> u64 {
    const LO: u64 = 0x2020_2020_2020_2020; // 0x20 in each byte
    const HI: u64 = 0x7F7F_7F7F_7F7F_7F7F; // 0x7F in each byte
    const MASK: u64 = 0x8080_8080_8080_8080; // high bit of each byte

    // Check for bytes < 0x20:
    // Set high bit to prevent cross-byte borrows, then subtract 0x20.
    // If result's high bit is clear, byte was < 0x20.
    let tmp_lo = (word | MASK).wrapping_sub(LO);
    let low_violation = !tmp_lo & MASK;

    // Check for bytes >= 0x7F:
    // Set high bit to prevent cross-byte borrows, then subtract 0x7F.
    // If result's high bit is set, byte was >= 0x7F.
    let tmp_hi = (word | MASK).wrapping_sub(HI);
    let high_violation = tmp_hi & MASK;

    low_violation | high_violation
}

/// Validate that all bytes are printable ASCII (0x20-0x7E) using SWAR.
///
/// Processes 8 bytes at a time using range checking.
#[inline]
pub fn validate_printable(bytes: &[u8]) -> Result<(), (u8, usize)> {
    let mut i = 0;

    // Process 8 bytes at a time
    while i + 8 <= bytes.len() {
        // SAFETY: We just checked that i + 8 <= bytes.len()
        let chunk: [u8; 8] = unsafe {
            bytes
                .as_ptr()
                .add(i)
                .cast::<[u8; 8]>()
                .read_unaligned()
        };
        let word = u64::from_ne_bytes(chunk);
        let mask = has_non_printable(word);
        if mask != 0 {
            // Found non-printable - use trailing_zeros to find first invalid byte
            let offset = (mask.trailing_zeros() / 8) as usize;
            let pos = i + offset;
            return Err((bytes[pos], pos));
        }
        i += 8;
    }

    // Handle remainder byte by byte
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x20 || b > 0x7E {
            return Err((b, i));
        }
        i += 1;
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_ascii_valid() {
        assert!(validate_ascii(b"Hello, World!").is_ok());
        assert!(validate_ascii(b"").is_ok());
        assert!(validate_ascii(&[0u8]).is_ok());
        assert!(validate_ascii(&[127u8]).is_ok());
    }

    #[test]
    fn test_validate_ascii_invalid() {
        assert_eq!(validate_ascii(&[128u8]), Err((128, 0)));
        assert_eq!(validate_ascii(&[255u8]), Err((255, 0)));
        assert_eq!(validate_ascii(b"Hello\x80"), Err((0x80, 5)));
    }

    #[test]
    fn test_validate_printable_valid() {
        assert!(validate_printable(b"Hello, World!").is_ok());
        assert!(validate_printable(b"").is_ok());
        assert!(validate_printable(b" ").is_ok());
        assert!(validate_printable(b"~").is_ok());
    }

    #[test]
    fn test_validate_printable_invalid() {
        assert_eq!(validate_printable(&[0x00]), Err((0x00, 0)));
        assert_eq!(validate_printable(&[0x1F]), Err((0x1F, 0)));
        assert_eq!(validate_printable(&[0x7F]), Err((0x7F, 0)));
        assert_eq!(validate_printable(&[0x80]), Err((0x80, 0)));
    }

    #[test]
    fn test_has_non_printable_boundaries() {
        // Test the SWAR function directly
        let all_space = 0x2020_2020_2020_2020u64;
        assert_eq!(has_non_printable(all_space), 0);

        let all_tilde = 0x7E7E_7E7E_7E7E_7E7Eu64;
        assert_eq!(has_non_printable(all_tilde), 0);

        // 0x1F in first byte (control char)
        let with_control = 0x2020_2020_2020_201Fu64;
        assert_ne!(has_non_printable(with_control), 0);

        // 0x7F in first byte (DEL)
        let with_del = 0x2020_2020_2020_207Fu64;
        assert_ne!(has_non_printable(with_del), 0);
    }
}
