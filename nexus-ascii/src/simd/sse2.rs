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
// Case Conversion
// =============================================================================

/// Convert byte slice to lowercase in-place using SSE2 (16 bytes at a time).
///
/// Only ASCII uppercase letters (A-Z) are affected.
/// Cascades to scalar for inputs shorter than 16 bytes.
#[inline]
pub fn make_lowercase(bytes: &mut [u8]) {
    let len = bytes.len();
    if len < 16 {
        scalar::make_lowercase(bytes);
        return;
    }

    let mut i = 0;

    // SAFETY: SSE2 is baseline for x86_64
    unsafe {
        let case_bit = _mm_set1_epi8(0x20);
        let lo_bound = _mm_set1_epi8(0x40); // 'A' - 1
        let hi_bound = _mm_set1_epi8(0x5B); // 'Z' + 1

        // Process 16 bytes at a time
        while i + 16 <= len {
            let ptr = bytes.as_ptr().add(i).cast();
            let chunk = _mm_loadu_si128(ptr);

            // Identify uppercase letters: byte > 0x40 AND byte < 0x5B
            let ge_a = _mm_cmpgt_epi8(chunk, lo_bound);
            let lt_z1 = _mm_cmpgt_epi8(hi_bound, chunk);
            let is_upper = _mm_and_si128(ge_a, lt_z1);

            // Create mask: 0x20 where uppercase, 0x00 elsewhere
            let mask = _mm_and_si128(is_upper, case_bit);

            // Apply: OR sets bit 5 for uppercase letters
            let result = _mm_or_si128(chunk, mask);
            _mm_storeu_si128(bytes.as_mut_ptr().add(i).cast(), result);

            i += 16;
        }

        // Handle remainder with overlapping last-16 (idempotent)
        if i < len {
            let offset = len - 16;
            let ptr = bytes.as_ptr().add(offset).cast();
            let chunk = _mm_loadu_si128(ptr);

            let ge_a = _mm_cmpgt_epi8(chunk, lo_bound);
            let lt_z1 = _mm_cmpgt_epi8(hi_bound, chunk);
            let is_upper = _mm_and_si128(ge_a, lt_z1);
            let mask = _mm_and_si128(is_upper, case_bit);
            let result = _mm_or_si128(chunk, mask);
            _mm_storeu_si128(bytes.as_mut_ptr().add(offset).cast(), result);
        }
    }
}

/// Convert byte slice to uppercase in-place using SSE2 (16 bytes at a time).
///
/// Only ASCII lowercase letters (a-z) are affected.
/// Cascades to scalar for inputs shorter than 16 bytes.
#[inline]
pub fn make_uppercase(bytes: &mut [u8]) {
    let len = bytes.len();
    if len < 16 {
        scalar::make_uppercase(bytes);
        return;
    }

    let mut i = 0;

    // SAFETY: SSE2 is baseline for x86_64
    unsafe {
        let case_bit = _mm_set1_epi8(0x20);
        let lo_bound = _mm_set1_epi8(0x60); // 'a' - 1
        let hi_bound = _mm_set1_epi8(0x7B); // 'z' + 1

        // Process 16 bytes at a time
        while i + 16 <= len {
            let ptr = bytes.as_ptr().add(i).cast();
            let chunk = _mm_loadu_si128(ptr);

            // Identify lowercase letters: byte > 0x60 AND byte < 0x7B
            let ge_a = _mm_cmpgt_epi8(chunk, lo_bound);
            let lt_z1 = _mm_cmpgt_epi8(hi_bound, chunk);
            let is_lower = _mm_and_si128(ge_a, lt_z1);

            // Create mask: 0x20 where lowercase, 0x00 elsewhere
            let mask = _mm_and_si128(is_lower, case_bit);

            // Apply: ANDNOT clears bit 5 for lowercase letters
            let result = _mm_andnot_si128(mask, chunk);
            _mm_storeu_si128(bytes.as_mut_ptr().add(i).cast(), result);

            i += 16;
        }

        // Handle remainder with overlapping last-16 (idempotent)
        if i < len {
            let offset = len - 16;
            let ptr = bytes.as_ptr().add(offset).cast();
            let chunk = _mm_loadu_si128(ptr);

            let ge_a = _mm_cmpgt_epi8(chunk, lo_bound);
            let lt_z1 = _mm_cmpgt_epi8(hi_bound, chunk);
            let is_lower = _mm_and_si128(ge_a, lt_z1);
            let mask = _mm_and_si128(is_lower, case_bit);
            let result = _mm_andnot_si128(mask, chunk);
            _mm_storeu_si128(bytes.as_mut_ptr().add(offset).cast(), result);
        }
    }
}

// =============================================================================
// Case-Insensitive Comparison
// =============================================================================

/// Compare two byte slices for case-insensitive ASCII equality using SSE2.
///
/// Processes 16 bytes at a time with a single movemask per chunk.
/// For ASCII letters, case is ignored. Non-letter bytes must match exactly.
///
/// Uses single-movemask approach: for each byte pair, compute whether it's
/// "ok" (identical OR valid case flip), then check all bytes in one operation.
/// This avoids the 3× movemask domain-crossing overhead of the naive approach.
#[inline]
pub fn eq_ignore_ascii_case(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let len = a.len();
    if len < 16 {
        return scalar::eq_ignore_ascii_case(a, b);
    }

    let mut i = 0;

    // SAFETY: SSE2 is baseline for x86_64
    unsafe {
        let case_bit = _mm_set1_epi8(0x20);
        let lo_bound = _mm_set1_epi8(0x60); // 'a' - 1
        let hi_bound = _mm_set1_epi8(0x7B); // 'z' + 1
        let zero = _mm_setzero_si128();

        while i + 16 <= len {
            let chunk_a = _mm_loadu_si128(a.as_ptr().add(i).cast());
            let chunk_b = _mm_loadu_si128(b.as_ptr().add(i).cast());

            let xor = _mm_xor_si128(chunk_a, chunk_b);

            // Fast path: if chunks are identical, skip letter check entirely.
            // cmpeq+movemask is needed here (SSE2 has no PTEST), but this
            // avoids the more expensive letter-checking path below.
            if _mm_movemask_epi8(_mm_cmpeq_epi8(xor, zero)) == 0xFFFF {
                i += 16;
                continue;
            }

            // A byte pair is "ok" if:
            //   1. identical (xor == 0), OR
            //   2. differs by exactly 0x20 AND the byte is an ASCII letter

            let identical = _mm_cmpeq_epi8(xor, zero);
            let is_case_flip = _mm_cmpeq_epi8(xor, case_bit);

            // Letter check: (chunk_a | 0x20) in ['a', 'z']
            let lower_a = _mm_or_si128(chunk_a, case_bit);
            let ge_a = _mm_cmpgt_epi8(lower_a, lo_bound);
            let lt_z1 = _mm_cmpgt_epi8(hi_bound, lower_a);
            let is_letter = _mm_and_si128(ge_a, lt_z1);

            // Valid case flip = differs by 0x20 AND is a letter
            let valid_flip = _mm_and_si128(is_case_flip, is_letter);

            // Byte is ok = identical OR valid case flip
            let ok = _mm_or_si128(identical, valid_flip);

            // Single movemask: if any byte is not ok (0x00 lane), fail
            if _mm_movemask_epi8(ok) != 0xFFFF {
                return false;
            }

            i += 16;
        }
    }

    // Handle remainder with scalar
    if i < len {
        return scalar::eq_ignore_ascii_case(&a[i..], &b[i..]);
    }

    true
}

// =============================================================================
// Numeric Validation
// =============================================================================

/// Check if all bytes are ASCII digits ('0'-'9') using SSE2 (16 bytes at a time).
///
/// Cascades to scalar for inputs shorter than 16 bytes.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn is_all_numeric(bytes: &[u8]) -> bool {
    let len = bytes.len();
    if len < 16 {
        return scalar::is_all_numeric(bytes);
    }

    let mut i = 0;

    // SAFETY: SSE2 is baseline for x86_64
    unsafe {
        // Range check: byte in ['0', '9'] means byte > '/' AND byte < ':'
        let lo_bound = _mm_set1_epi8(0x2F); // '0' - 1 = 0x2F
        let hi_bound = _mm_set1_epi8(0x3A); // '9' + 1 = 0x3A

        // Process 16 bytes at a time
        while i + 16 <= len {
            let chunk = _mm_loadu_si128(bytes.as_ptr().add(i).cast());

            // Check if each byte is in range ['0', '9']
            // is_digit = (byte > 0x2F) AND (byte < 0x3A)
            let gt_lo = _mm_cmpgt_epi8(chunk, lo_bound);
            let lt_hi = _mm_cmpgt_epi8(hi_bound, chunk);
            let is_digit = _mm_and_si128(gt_lo, lt_hi);

            // All bytes must be digits (mask = 0xFFFF)
            if _mm_movemask_epi8(is_digit) != 0xFFFF {
                return false;
            }

            i += 16;
        }
    }

    // Handle remainder with scalar
    if i < len {
        return scalar::is_all_numeric(&bytes[i..]);
    }

    true
}

// =============================================================================
// Alphanumeric Validation
// =============================================================================

/// Check if all bytes are ASCII alphanumeric (0-9, A-Z, a-z) using SSE2.
///
/// Cascades to scalar for inputs shorter than 16 bytes.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn is_all_alphanumeric(bytes: &[u8]) -> bool {
    let len = bytes.len();
    if len < 16 {
        return scalar::is_all_alphanumeric(bytes);
    }

    let mut i = 0;

    // SAFETY: SSE2 is baseline for x86_64
    unsafe {
        // Digit range: ['0', '9'] = [0x30, 0x39]
        let digit_lo = _mm_set1_epi8(0x2F); // '0' - 1
        let digit_hi = _mm_set1_epi8(0x3A); // '9' + 1

        // Uppercase range: ['A', 'Z'] = [0x41, 0x5A]
        let upper_lo = _mm_set1_epi8(0x40); // 'A' - 1
        let upper_hi = _mm_set1_epi8(0x5B); // 'Z' + 1

        // Lowercase range: ['a', 'z'] = [0x61, 0x7A]
        let lower_lo = _mm_set1_epi8(0x60); // 'a' - 1
        let lower_hi = _mm_set1_epi8(0x7B); // 'z' + 1

        // Process 16 bytes at a time
        while i + 16 <= len {
            let chunk = _mm_loadu_si128(bytes.as_ptr().add(i).cast());

            // Check digit: byte > 0x2F AND byte < 0x3A
            let is_digit = _mm_and_si128(
                _mm_cmpgt_epi8(chunk, digit_lo),
                _mm_cmpgt_epi8(digit_hi, chunk),
            );

            // Check uppercase: byte > 0x40 AND byte < 0x5B
            let is_upper = _mm_and_si128(
                _mm_cmpgt_epi8(chunk, upper_lo),
                _mm_cmpgt_epi8(upper_hi, chunk),
            );

            // Check lowercase: byte > 0x60 AND byte < 0x7B
            let is_lower = _mm_and_si128(
                _mm_cmpgt_epi8(chunk, lower_lo),
                _mm_cmpgt_epi8(lower_hi, chunk),
            );

            // Alphanumeric = digit OR upper OR lower
            let is_alnum = _mm_or_si128(_mm_or_si128(is_digit, is_upper), is_lower);

            // All bytes must be alphanumeric (mask = 0xFFFF)
            if _mm_movemask_epi8(is_alnum) != 0xFFFF {
                return false;
            }

            i += 16;
        }
    }

    // Handle remainder with scalar
    if i < len {
        return scalar::is_all_alphanumeric(&bytes[i..]);
    }

    true
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

    // -------------------------------------------------------------------------
    // Case conversion
    // -------------------------------------------------------------------------

    #[test]
    fn test_make_lowercase_matches_scalar() {
        for len in 0..=64 {
            let bytes: Vec<u8> = (0..len).map(|i| (0x20 + (i % 95)) as u8).collect();
            let mut simd_result = bytes.clone();
            let mut scalar_result = bytes.clone();
            make_lowercase(&mut simd_result);
            scalar::make_lowercase(&mut scalar_result);
            assert_eq!(simd_result, scalar_result, "len={}", len);
        }
    }

    #[test]
    fn test_make_uppercase_matches_scalar() {
        for len in 0..=64 {
            let bytes: Vec<u8> = (0..len).map(|i| (0x20 + (i % 95)) as u8).collect();
            let mut simd_result = bytes.clone();
            let mut scalar_result = bytes.clone();
            make_uppercase(&mut simd_result);
            scalar::make_uppercase(&mut scalar_result);
            assert_eq!(simd_result, scalar_result, "len={}", len);
        }
    }

    #[test]
    fn test_make_lowercase_all_bytes() {
        // Test every ASCII byte value
        let mut bytes: Vec<u8> = (0..128).collect();
        let mut expected = bytes.clone();
        scalar::make_lowercase(&mut expected);
        make_lowercase(&mut bytes);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn test_make_uppercase_all_bytes() {
        let mut bytes: Vec<u8> = (0..128).collect();
        let mut expected = bytes.clone();
        scalar::make_uppercase(&mut expected);
        make_uppercase(&mut bytes);
        assert_eq!(bytes, expected);
    }

    // -------------------------------------------------------------------------
    // Case-insensitive comparison
    // -------------------------------------------------------------------------

    #[test]
    fn test_eq_ignore_case_matches_scalar() {
        for len in 0..=64 {
            let a: Vec<u8> = (0..len).map(|i| (0x41 + (i % 26)) as u8).collect(); // A-Z repeating
            let b: Vec<u8> = a.iter().map(|&c| c | 0x20).collect(); // a-z repeating
            assert_eq!(
                eq_ignore_ascii_case(&a, &b),
                scalar::eq_ignore_ascii_case(&a, &b),
                "len={}",
                len
            );
        }
    }

    #[test]
    fn test_eq_ignore_case_not_equal() {
        for len in 1..=48 {
            for pos in 0..len {
                let a: Vec<u8> = vec![b'A'; len];
                let mut b: Vec<u8> = vec![b'a'; len];
                b[pos] = b'0'; // Non-letter difference
                assert_eq!(
                    eq_ignore_ascii_case(&a, &b),
                    scalar::eq_ignore_ascii_case(&a, &b),
                    "len={}, pos={}",
                    len,
                    pos
                );
            }
        }
    }

    #[test]
    fn test_eq_ignore_case_exhaustive_single_byte() {
        for a in 0..128u8 {
            for b in 0..128u8 {
                let expected = scalar::eq_ignore_ascii_case(&[a], &[b]);
                let actual = eq_ignore_ascii_case(&[a], &[b]);
                assert_eq!(actual, expected, "a=0x{:02X}, b=0x{:02X}", a, b);
            }
        }
    }
}
