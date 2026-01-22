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
// Case-Insensitive Comparison
// =============================================================================

/// Compare two 8-byte words for case-insensitive ASCII equality.
///
/// Returns true if the words are equal when ignoring ASCII case.
/// Two bytes are case-insensitively equal if:
/// - They are identical, OR
/// - They differ by exactly 0x20 (bit 5) AND at least one is an ASCII letter
#[inline(always)]
const fn eq_ignore_ascii_case_word(a: u64, b: u64) -> bool {
    let xor = a ^ b;

    // Fast path: already equal
    if xor == 0 {
        return true;
    }

    // All differences must be exactly in bit 5 (0x20)
    // If any other bit differs, bytes can't be case-equivalent
    if xor & !0x2020_2020_2020_2020 != 0 {
        return false;
    }

    // For differing bytes, verify they're ASCII letters.
    // Force to lowercase, then check if in 'a'-'z' range.
    let lower_a = a | 0x2020_2020_2020_2020;

    // SWAR range check [0x61, 0x7A] (letters 'a'-'z'):
    // A byte is NOT in range if: byte < 0x61 OR byte > 0x7A
    // We detect this by checking if subtraction wraps (sets high bit)
    let below = lower_a.wrapping_sub(0x6161_6161_6161_6161);
    let above = 0x7a7a_7a7a_7a7a_7a7a_u64.wrapping_sub(lower_a);

    // High bit set in either means byte is not a letter
    let not_letter = (below | above) & 0x8080_8080_8080_8080;

    // Convert diff mask: 0x20 -> 0x80, 0x00 -> 0x00
    // This marks bytes that actually differ
    let diff_mask = (xor << 2) & 0x8080_8080_8080_8080;

    // If any differing byte is not a letter, fail
    (not_letter & diff_mask) == 0
}

/// Compare two byte slices for case-insensitive ASCII equality using SWAR.
///
/// Processes 8 bytes at a time. For ASCII letters (A-Z, a-z), case is ignored.
/// Non-letter ASCII characters must match exactly.
///
/// # Example
///
/// ```ignore
/// assert!(eq_ignore_ascii_case(b"Hello", b"HELLO"));
/// assert!(eq_ignore_ascii_case(b"BTC-USD", b"btc-usd"));
/// assert!(!eq_ignore_ascii_case(b"Hello", b"World"));
/// ```
#[inline]
pub fn eq_ignore_ascii_case(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let len = a.len();
    let mut i = 0;

    // Process 8 bytes at a time
    while i + 8 <= len {
        // SAFETY: We just checked that i + 8 <= len
        let word_a = unsafe {
            a.as_ptr()
                .add(i)
                .cast::<[u8; 8]>()
                .read_unaligned()
        };
        let word_b = unsafe {
            b.as_ptr()
                .add(i)
                .cast::<[u8; 8]>()
                .read_unaligned()
        };

        if !eq_ignore_ascii_case_word(u64::from_ne_bytes(word_a), u64::from_ne_bytes(word_b)) {
            return false;
        }
        i += 8;
    }

    // Handle remainder byte by byte
    while i < len {
        let x = a[i];
        let y = b[i];
        if x != y {
            // Check if they differ only by case
            if (x ^ y) != 0x20 {
                return false;
            }
            // Verify one is a letter
            let lower = x | 0x20;
            if !(b'a'..=b'z').contains(&lower) {
                return false;
            }
        }
        i += 1;
    }

    true
}

// =============================================================================
// Branchless Case Conversion
// =============================================================================

/// Convert a byte to lowercase using branchless arithmetic.
///
/// Only ASCII letters (A-Z) are affected; all other bytes pass through unchanged.
#[inline(always)]
pub const fn to_lowercase_branchless(byte: u8) -> u8 {
    // is_upper = 1 if byte in A-Z, 0 otherwise
    // A-Z range: 0x41-0x5A
    // (byte - 0x41) < 26 means byte is in A-Z
    let in_range = (byte.wrapping_sub(b'A') < 26) as u8;
    byte | (in_range << 5) // Add 0x20 if uppercase
}

/// Convert a byte to uppercase using branchless arithmetic.
///
/// Only ASCII letters (a-z) are affected; all other bytes pass through unchanged.
#[inline(always)]
pub const fn to_uppercase_branchless(byte: u8) -> u8 {
    // is_lower = 1 if byte in a-z, 0 otherwise
    // a-z range: 0x61-0x7A
    let in_range = (byte.wrapping_sub(b'a') < 26) as u8;
    byte & !(in_range << 5) // Clear bit 5 if lowercase
}

/// Convert a u64 word to lowercase using SWAR.
///
/// All ASCII uppercase letters (A-Z) become lowercase (a-z).
/// Non-letter bytes are unchanged.
#[inline(always)]
pub const fn to_lowercase_word(word: u64) -> u64 {
    // For each byte, set bit 5 (OR with 0x20) if it's in A-Z range [0x41, 0x5A]
    //
    // SWAR range check technique:
    // - Add (0x80 - 'A') = 0x3F: if byte >= 'A', result has high bit set
    // - Add (0x80 - 'Z' - 1) = 0x25: if byte > 'Z', result has high bit set
    // - is_upper = (ge_a) AND NOT(gt_z)

    let ge_a = word.wrapping_add(0x3F3F_3F3F_3F3F_3F3F); // High bit set if >= 'A'
    let gt_z = word.wrapping_add(0x2525_2525_2525_2525); // High bit set if > 'Z'

    // Byte is uppercase if: high bit set in ge_a AND high bit clear in gt_z
    let is_upper = ge_a & !gt_z & 0x8080_8080_8080_8080;

    // Convert 0x80 -> 0x20 for the mask
    let mask = is_upper >> 2;

    word | mask
}

/// Convert a u64 word to uppercase using SWAR.
///
/// All ASCII lowercase letters (a-z) become uppercase (A-Z).
/// Non-letter bytes are unchanged.
#[inline(always)]
pub const fn to_uppercase_word(word: u64) -> u64 {
    // For each byte, clear bit 5 (AND with ~0x20) if it's in a-z range [0x61, 0x7A]
    //
    // SWAR range check technique:
    // - Add (0x80 - 'a') = 0x1F: if byte >= 'a', result has high bit set
    // - Add (0x80 - 'z' - 1) = 0x05: if byte > 'z', result has high bit set
    // - is_lower = (ge_a) AND NOT(gt_z)

    let ge_a = word.wrapping_add(0x1F1F_1F1F_1F1F_1F1F); // High bit set if >= 'a'
    let gt_z = word.wrapping_add(0x0505_0505_0505_0505); // High bit set if > 'z'

    // Byte is lowercase if: high bit set in ge_a AND high bit clear in gt_z
    let is_lower = ge_a & !gt_z & 0x8080_8080_8080_8080;

    // Convert 0x80 -> 0x20 for the mask, then clear bit 5 for lowercase letters
    let mask = is_lower >> 2;

    word & !mask
}

/// Convert byte slice to lowercase in-place using SWAR.
#[inline]
pub fn make_lowercase(bytes: &mut [u8]) {
    let len = bytes.len();
    let mut i = 0;

    // Process 8 bytes at a time
    while i + 8 <= len {
        // SAFETY: We just checked that i + 8 <= len
        let chunk: [u8; 8] = unsafe {
            bytes
                .as_ptr()
                .add(i)
                .cast::<[u8; 8]>()
                .read_unaligned()
        };
        let word = u64::from_ne_bytes(chunk);
        let lower = to_lowercase_word(word);

        // Write back
        // SAFETY: Same bounds as read
        unsafe {
            bytes
                .as_mut_ptr()
                .add(i)
                .cast::<[u8; 8]>()
                .write_unaligned(lower.to_ne_bytes());
        }
        i += 8;
    }

    // Handle remainder byte by byte
    while i < len {
        bytes[i] = to_lowercase_branchless(bytes[i]);
        i += 1;
    }
}

/// Convert byte slice to uppercase in-place using SWAR.
#[inline]
pub fn make_uppercase(bytes: &mut [u8]) {
    let len = bytes.len();
    let mut i = 0;

    // Process 8 bytes at a time
    while i + 8 <= len {
        // SAFETY: We just checked that i + 8 <= len
        let chunk: [u8; 8] = unsafe {
            bytes
                .as_ptr()
                .add(i)
                .cast::<[u8; 8]>()
                .read_unaligned()
        };
        let word = u64::from_ne_bytes(chunk);
        let upper = to_uppercase_word(word);

        // Write back
        unsafe {
            bytes
                .as_mut_ptr()
                .add(i)
                .cast::<[u8; 8]>()
                .write_unaligned(upper.to_ne_bytes());
        }
        i += 8;
    }

    // Handle remainder byte by byte
    while i < len {
        bytes[i] = to_uppercase_branchless(bytes[i]);
        i += 1;
    }
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

    // -------------------------------------------------------------------------
    // Case-insensitive comparison tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_eq_ignore_ascii_case_equal() {
        assert!(eq_ignore_ascii_case(b"hello", b"hello"));
        assert!(eq_ignore_ascii_case(b"HELLO", b"HELLO"));
        assert!(eq_ignore_ascii_case(b"", b""));
    }

    #[test]
    fn test_eq_ignore_ascii_case_different_case() {
        assert!(eq_ignore_ascii_case(b"hello", b"HELLO"));
        assert!(eq_ignore_ascii_case(b"HELLO", b"hello"));
        assert!(eq_ignore_ascii_case(b"HeLLo", b"hEllO"));
    }

    #[test]
    fn test_eq_ignore_ascii_case_with_symbols() {
        assert!(eq_ignore_ascii_case(b"BTC-USD", b"btc-usd"));
        assert!(eq_ignore_ascii_case(b"Hello, World!", b"HELLO, WORLD!"));
        assert!(eq_ignore_ascii_case(b"abc123xyz", b"ABC123XYZ"));
    }

    #[test]
    fn test_eq_ignore_ascii_case_different() {
        assert!(!eq_ignore_ascii_case(b"hello", b"world"));
        assert!(!eq_ignore_ascii_case(b"abc", b"abd"));
        assert!(!eq_ignore_ascii_case(b"hello", b"hell"));
    }

    #[test]
    fn test_eq_ignore_ascii_case_non_letter_differs_by_0x20() {
        // '@' (0x40) and '`' (0x60) differ by 0x20 but neither is a letter
        assert!(!eq_ignore_ascii_case(b"@", b"`"));
        // '!' (0x21) and '!' (0x21) are equal
        assert!(eq_ignore_ascii_case(b"!", b"!"));
        // '1' and '1' are equal
        assert!(eq_ignore_ascii_case(b"1", b"1"));
        // But '1' (0x31) and 'Q' (0x51) differ by 0x20, '1' is not a letter
        // Actually wait: 0x51 - 0x31 = 0x20, but 'Q' is a letter, '1' is not
        // eq_ignore_ascii_case should return false because '1' is not a letter
        assert!(!eq_ignore_ascii_case(b"1", b"Q"));
    }

    #[test]
    fn test_eq_ignore_ascii_case_long_strings() {
        // Test SWAR path (> 8 bytes)
        assert!(eq_ignore_ascii_case(
            b"The Quick Brown Fox",
            b"THE QUICK BROWN FOX"
        ));
        assert!(eq_ignore_ascii_case(
            b"abcdefghijklmnopqrstuvwxyz",
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZ"
        ));
    }

    #[test]
    fn test_eq_ignore_ascii_case_boundary_letters() {
        // Test boundary letters A/a and Z/z
        assert!(eq_ignore_ascii_case(b"A", b"a"));
        assert!(eq_ignore_ascii_case(b"Z", b"z"));
        assert!(eq_ignore_ascii_case(b"AZ", b"az"));
        assert!(eq_ignore_ascii_case(b"za", b"ZA"));
    }

    // -------------------------------------------------------------------------
    // Branchless case conversion tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_to_lowercase_branchless() {
        assert_eq!(to_lowercase_branchless(b'A'), b'a');
        assert_eq!(to_lowercase_branchless(b'Z'), b'z');
        assert_eq!(to_lowercase_branchless(b'M'), b'm');
        // Already lowercase
        assert_eq!(to_lowercase_branchless(b'a'), b'a');
        // Non-letters unchanged
        assert_eq!(to_lowercase_branchless(b'1'), b'1');
        assert_eq!(to_lowercase_branchless(b'@'), b'@');
        assert_eq!(to_lowercase_branchless(b' '), b' ');
    }

    #[test]
    fn test_to_uppercase_branchless() {
        assert_eq!(to_uppercase_branchless(b'a'), b'A');
        assert_eq!(to_uppercase_branchless(b'z'), b'Z');
        assert_eq!(to_uppercase_branchless(b'm'), b'M');
        // Already uppercase
        assert_eq!(to_uppercase_branchless(b'A'), b'A');
        // Non-letters unchanged
        assert_eq!(to_uppercase_branchless(b'1'), b'1');
        assert_eq!(to_uppercase_branchless(b'@'), b'@');
        assert_eq!(to_uppercase_branchless(b' '), b' ');
    }

    #[test]
    fn test_make_lowercase() {
        let mut bytes = *b"Hello, World!";
        make_lowercase(&mut bytes);
        assert_eq!(&bytes, b"hello, world!");

        let mut bytes = *b"BTC-USD";
        make_lowercase(&mut bytes);
        assert_eq!(&bytes, b"btc-usd");

        // Long string to test SWAR path
        let mut bytes = *b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        make_lowercase(&mut bytes);
        assert_eq!(&bytes, b"abcdefghijklmnopqrstuvwxyz");
    }

    #[test]
    fn test_make_uppercase() {
        let mut bytes = *b"Hello, World!";
        make_uppercase(&mut bytes);
        assert_eq!(&bytes, b"HELLO, WORLD!");

        let mut bytes = *b"btc-usd";
        make_uppercase(&mut bytes);
        assert_eq!(&bytes, b"BTC-USD");

        // Long string to test SWAR path
        let mut bytes = *b"abcdefghijklmnopqrstuvwxyz";
        make_uppercase(&mut bytes);
        assert_eq!(&bytes, b"ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    }

    #[test]
    fn test_case_conversion_all_ascii() {
        // Verify all ASCII bytes are handled correctly
        for b in 0u8..=127 {
            let lower = to_lowercase_branchless(b);
            let upper = to_uppercase_branchless(b);

            if b.is_ascii_uppercase() {
                assert_eq!(lower, b + 32);
                assert_eq!(upper, b);
            } else if b.is_ascii_lowercase() {
                assert_eq!(lower, b);
                assert_eq!(upper, b - 32);
            } else {
                assert_eq!(lower, b);
                assert_eq!(upper, b);
            }
        }
    }
}
