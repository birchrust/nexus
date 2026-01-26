//! AVX2 SIMD validation (32 bytes at a time).
//!
//! Available when compiled with `-C target-feature=+avx2`.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use super::sse2;

// =============================================================================
// ASCII Validation
// =============================================================================

/// Validate that all bytes are ASCII (< 128) using AVX2.
///
/// Processes 32 bytes at a time using `_mm256_movemask_epi8` to check high bits.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn validate_ascii(bytes: &[u8]) -> Result<(), (u8, usize)> {
    let len = bytes.len();
    let mut i = 0;

    // SAFETY: AVX2 availability is guaranteed by target_feature cfg
    unsafe {
        // Process 32 bytes at a time
        while i + 32 <= len {
            let chunk = _mm256_loadu_si256(bytes.as_ptr().add(i).cast());
            // movemask extracts the high bit of each byte into a 32-bit mask
            let mask = _mm256_movemask_epi8(chunk);
            if mask != 0 {
                // Found non-ASCII byte(s) - find the first one
                let offset = mask.trailing_zeros() as usize;
                let pos = i + offset;
                return Err((bytes[pos], pos));
            }
            i += 32;
        }
    }

    // Handle remainder with SSE2 (which cascades to scalar for its own tail)
    if i < len {
        sse2::validate_ascii(&bytes[i..]).map_err(|(b, p)| (b, i + p))?;
    }

    Ok(())
}

// =============================================================================
// Printable Validation
// =============================================================================

/// Validate that all bytes are printable ASCII (0x20-0x7E) using AVX2.
///
/// Uses the bias trick to convert unsigned comparisons to signed:
/// XOR with 0x80 maps [0,255] to signed [-128,127], then use signed compares.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn validate_printable(bytes: &[u8]) -> Result<(), (u8, usize)> {
    let len = bytes.len();
    let mut i = 0;

    // SAFETY: AVX2 availability is guaranteed by target_feature cfg
    unsafe {
        // Bias value: XOR with 0x80 to convert unsigned to signed
        let bias = _mm256_set1_epi8(-128i8);

        // After biasing:
        // - 0x20 (space) becomes 0xA0 = -96 (signed)
        // - 0x7E (tilde) becomes 0xFE = -2 (signed)
        // We want to reject bytes with biased value < -96 or > -2

        // Low bound: 0x20 - 1 = 0x1F, biased = 0x9F = -97
        let lo_bound = _mm256_set1_epi8(-97i8);
        // High bound: 0x7E, biased = 0xFE = -2
        let hi_bound = _mm256_set1_epi8(-2i8);

        // Process 32 bytes at a time
        while i + 32 <= len {
            let chunk = _mm256_loadu_si256(bytes.as_ptr().add(i).cast());

            // XOR with bias to convert to signed range
            let biased = _mm256_xor_si256(chunk, bias);

            // Check for bytes outside [0x20, 0x7E]:
            // - cmpgt(lo_bound, biased) catches bytes < 0x20
            // - cmpgt(biased, hi_bound) catches bytes > 0x7E
            let below = _mm256_cmpgt_epi8(lo_bound, biased);
            let above = _mm256_cmpgt_epi8(biased, hi_bound);
            let invalid = _mm256_or_si256(below, above);

            let mask = _mm256_movemask_epi8(invalid);
            if mask != 0 {
                let offset = mask.trailing_zeros() as usize;
                let pos = i + offset;
                return Err((bytes[pos], pos));
            }

            i += 32;
        }
    }

    // Handle remainder with SSE2 (which cascades to scalar for its own tail)
    if i < len {
        sse2::validate_printable(&bytes[i..]).map_err(|(b, p)| (b, i + p))?;
    }

    Ok(())
}

// =============================================================================
// Case Conversion
// =============================================================================

/// Convert byte slice to lowercase in-place using AVX2 (32 bytes at a time).
///
/// Only ASCII uppercase letters (A-Z) are affected.
/// Cascades to SSE2 for remainder.
#[inline]
pub fn make_lowercase(bytes: &mut [u8]) {
    let len = bytes.len();
    if len < 32 {
        sse2::make_lowercase(bytes);
        return;
    }

    let mut i = 0;

    // SAFETY: AVX2 availability is guaranteed by target_feature cfg
    unsafe {
        let case_bit = _mm256_set1_epi8(0x20);
        let lo_bound = _mm256_set1_epi8(0x40); // 'A' - 1
        let hi_bound = _mm256_set1_epi8(0x5B); // 'Z' + 1

        while i + 32 <= len {
            let ptr = bytes.as_ptr().add(i).cast();
            let chunk = _mm256_loadu_si256(ptr);

            let ge_a = _mm256_cmpgt_epi8(chunk, lo_bound);
            let lt_z1 = _mm256_cmpgt_epi8(hi_bound, chunk);
            let is_upper = _mm256_and_si256(ge_a, lt_z1);
            let mask = _mm256_and_si256(is_upper, case_bit);
            let result = _mm256_or_si256(chunk, mask);
            _mm256_storeu_si256(bytes.as_mut_ptr().add(i).cast(), result);

            i += 32;
        }
    }

    // Handle remainder with SSE2 (which cascades to scalar)
    if i < len {
        sse2::make_lowercase(&mut bytes[i..]);
    }
}

/// Convert byte slice to uppercase in-place using AVX2 (32 bytes at a time).
///
/// Only ASCII lowercase letters (a-z) are affected.
/// Cascades to SSE2 for remainder.
#[inline]
pub fn make_uppercase(bytes: &mut [u8]) {
    let len = bytes.len();
    if len < 32 {
        sse2::make_uppercase(bytes);
        return;
    }

    let mut i = 0;

    // SAFETY: AVX2 availability is guaranteed by target_feature cfg
    unsafe {
        let case_bit = _mm256_set1_epi8(0x20);
        let lo_bound = _mm256_set1_epi8(0x60); // 'a' - 1
        let hi_bound = _mm256_set1_epi8(0x7B); // 'z' + 1

        while i + 32 <= len {
            let ptr = bytes.as_ptr().add(i).cast();
            let chunk = _mm256_loadu_si256(ptr);

            let ge_a = _mm256_cmpgt_epi8(chunk, lo_bound);
            let lt_z1 = _mm256_cmpgt_epi8(hi_bound, chunk);
            let is_lower = _mm256_and_si256(ge_a, lt_z1);
            let mask = _mm256_and_si256(is_lower, case_bit);
            let result = _mm256_andnot_si256(mask, chunk);
            _mm256_storeu_si256(bytes.as_mut_ptr().add(i).cast(), result);

            i += 32;
        }
    }

    // Handle remainder with SSE2 (which cascades to scalar)
    if i < len {
        sse2::make_uppercase(&mut bytes[i..]);
    }
}

// =============================================================================
// Case-Insensitive Comparison
// =============================================================================

/// Compare two byte slices for case-insensitive ASCII equality using AVX2.
///
/// Processes 32 bytes at a time with a single movemask per chunk.
/// For ASCII letters, case is ignored. Non-letter bytes must match exactly.
/// Cascades to SSE2 for remainder.
///
/// Uses single-movemask approach: for each byte pair, compute whether it's
/// "ok" (identical OR valid case flip), then check all bytes in one operation.
#[inline]
pub fn eq_ignore_ascii_case(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let len = a.len();
    if len < 32 {
        return sse2::eq_ignore_ascii_case(a, b);
    }

    let mut i = 0;

    // SAFETY: AVX2 availability is guaranteed by target_feature cfg
    unsafe {
        let case_bit = _mm256_set1_epi8(0x20);
        let lo_bound = _mm256_set1_epi8(0x60); // 'a' - 1
        let hi_bound = _mm256_set1_epi8(0x7B); // 'z' + 1
        let zero = _mm256_setzero_si256();

        while i + 32 <= len {
            let chunk_a = _mm256_loadu_si256(a.as_ptr().add(i).cast());
            let chunk_b = _mm256_loadu_si256(b.as_ptr().add(i).cast());

            let xor = _mm256_xor_si256(chunk_a, chunk_b);

            // Fast path: if chunks are identical, skip letter check entirely.
            // VPTEST writes directly to FLAGS — no movemask domain crossing.
            if _mm256_testz_si256(xor, xor) != 0 {
                i += 32;
                continue;
            }

            // A byte pair is "ok" if:
            //   1. identical (xor == 0), OR
            //   2. differs by exactly 0x20 AND the byte is an ASCII letter

            let identical = _mm256_cmpeq_epi8(xor, zero);
            let is_case_flip = _mm256_cmpeq_epi8(xor, case_bit);

            // Letter check: (chunk_a | 0x20) in ['a', 'z']
            let lower_a = _mm256_or_si256(chunk_a, case_bit);
            let ge_a = _mm256_cmpgt_epi8(lower_a, lo_bound);
            let lt_z1 = _mm256_cmpgt_epi8(hi_bound, lower_a);
            let is_letter = _mm256_and_si256(ge_a, lt_z1);

            // Valid case flip = differs by 0x20 AND is a letter
            let valid_flip = _mm256_and_si256(is_case_flip, is_letter);

            // Byte is ok = identical OR valid case flip
            let ok = _mm256_or_si256(identical, valid_flip);

            // Single movemask: if any byte is not ok (0x00 lane), fail
            if _mm256_movemask_epi8(ok) != -1i32 {
                return false;
            }

            i += 32;
        }
    }

    // Handle remainder with SSE2 (which cascades to scalar)
    if i < len {
        return sse2::eq_ignore_ascii_case(&a[i..], &b[i..]);
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
    fn test_validate_ascii_exact_32() {
        assert!(validate_ascii(b"0123456789ABCDEF0123456789ABCDEF").is_ok());
    }

    #[test]
    fn test_validate_ascii_over_32() {
        let bytes = b"0123456789ABCDEF0123456789ABCDEFGHIJ";
        assert!(validate_ascii(bytes).is_ok());
    }

    #[test]
    fn test_validate_ascii_invalid_in_first_32() {
        let mut bytes = *b"0123456789ABCDEF0123456789ABCDEF";
        bytes[15] = 0x80;
        assert_eq!(validate_ascii(&bytes), Err((0x80, 15)));
    }

    #[test]
    fn test_validate_ascii_invalid_in_remainder() {
        let mut bytes = *b"0123456789ABCDEF0123456789ABCDEFGH";
        bytes[33] = 0x80;
        assert_eq!(validate_ascii(&bytes), Err((0x80, 33)));
    }

    #[test]
    fn test_validate_ascii_all_positions() {
        for pos in 0..64 {
            let mut bytes = vec![b'a'; 64];
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
    fn test_validate_printable_exact_32() {
        assert!(validate_printable(b"Hello, World! How are you today?").is_ok());
    }

    #[test]
    fn test_validate_printable_all_printable_chars() {
        // All 95 printable ASCII chars
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
        let mut bytes = *b"Hello, World! How are you today?";
        bytes[15] = 0x00;
        assert_eq!(validate_printable(&bytes), Err((0x00, 15)));
    }

    #[test]
    fn test_validate_printable_invalid_in_remainder() {
        let mut bytes = *b"Hello, World! How are you today? AB";
        bytes[34] = 0x7F;
        assert_eq!(validate_printable(&bytes), Err((0x7F, 34)));
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
        for len in 0..=96 {
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
        for len in 0..=96 {
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
        for len in 0..=96 {
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
        for len in 0..=96 {
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
        for len in 0..=96 {
            let a: Vec<u8> = (0..len).map(|i| (0x41 + (i % 26)) as u8).collect();
            let b: Vec<u8> = a.iter().map(|&c| c | 0x20).collect();
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
        for len in 1..=64 {
            for pos in 0..len {
                let a: Vec<u8> = vec![b'A'; len];
                let mut b: Vec<u8> = vec![b'a'; len];
                b[pos] = b'0';
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
