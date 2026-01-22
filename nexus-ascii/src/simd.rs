//! SIMD-accelerated validation functions.
//!
//! Provides ASCII and printable validation with automatic dispatch to the best
//! available implementation based on compile-time target features:
//!
//! - AVX-512 if `target_feature = "avx512bw"` is enabled (64 bytes at a time)
//! - AVX2 if `target_feature = "avx2"` is enabled (32 bytes at a time)
//! - SSE2 on x86_64 (16 bytes at a time, always available)
//! - Scalar SWAR on other architectures (8 bytes at a time)
//!
//! ## Usage
//!
//! ```bash
//! # Default (SSE2 on x86_64, scalar elsewhere)
//! cargo build --release
//!
//! # AVX2 (recommended for modern CPUs)
//! RUSTFLAGS="-C target-feature=+avx2" cargo build --release
//!
//! # AVX-512 (server CPUs, requires avx512bw for byte operations)
//! RUSTFLAGS="-C target-feature=+avx512bw" cargo build --release
//!
//! # Native (auto-detect CPU features)
//! RUSTFLAGS="-C target-cpu=native" cargo build --release
//! ```

// =============================================================================
// Implementations (internal)
// =============================================================================

mod scalar;

// SSE2: baseline x86_64, used when no higher SIMD is available
#[cfg(all(
    target_arch = "x86_64",
    not(target_feature = "avx2"),
    not(target_feature = "avx512bw")
))]
mod sse2;

// AVX2: 32 bytes/iteration, used when AVX2 available but not AVX-512
#[cfg(all(
    target_arch = "x86_64",
    target_feature = "avx2",
    not(target_feature = "avx512bw")
))]
mod avx2;

// AVX-512: 64 bytes/iteration, highest priority when available
#[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
mod avx512;

// =============================================================================
// ASCII Validation
// =============================================================================

/// Validate that all bytes are ASCII (< 128).
///
/// Returns `Ok(())` if all bytes are valid ASCII, or `Err((byte, pos))` with
/// the first invalid byte and its position.
///
/// Uses the best available SIMD implementation:
/// - AVX-512: 64 bytes at a time
/// - AVX2: 32 bytes at a time
/// - SSE2: 16 bytes at a time (x86_64 baseline)
/// - Scalar: 8 bytes at a time (SWAR)
///
/// # Example
///
/// ```
/// use nexus_ascii::simd;
///
/// assert!(simd::validate_ascii(b"Hello, World!").is_ok());
/// assert!(simd::validate_ascii(b"\x80").is_err());
/// ```
#[inline]
pub fn validate_ascii(bytes: &[u8]) -> Result<(), (u8, usize)> {
    // AVX-512: 64 bytes at a time (highest priority)
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
    {
        avx512::validate_ascii(bytes)
    }

    // AVX2: 32 bytes at a time
    #[cfg(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        not(target_feature = "avx512bw")
    ))]
    {
        avx2::validate_ascii(bytes)
    }

    // SSE2: 16 bytes at a time (x86_64 baseline)
    #[cfg(all(
        target_arch = "x86_64",
        not(target_feature = "avx2"),
        not(target_feature = "avx512bw")
    ))]
    {
        sse2::validate_ascii(bytes)
    }

    // Scalar SWAR: 8 bytes at a time
    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::validate_ascii(bytes)
    }
}

/// Validate ASCII with compile-time capacity bound.
///
/// Uses `CAP` to eliminate unreachable SIMD code paths at compile time:
/// - `CAP < 16`: Scalar only (SSE2 loop would never run)
/// - `CAP < 32`: SSE2 max (AVX2/AVX-512 loops would never run)
/// - `CAP < 64`: AVX2 max (AVX-512 loop would never run)
/// - `CAP >= 64`: Full SIMD dispatch
///
/// This is more efficient for fixed-capacity types like `AsciiString<7>` where
/// the compiler can inline just the scalar path.
#[inline]
pub fn validate_ascii_bounded<const CAP: usize>(bytes: &[u8]) -> Result<(), (u8, usize)> {
    debug_assert!(bytes.len() <= CAP);

    // CAP < 16: SSE2/AVX2/AVX-512 loops will never run, use scalar directly
    // This eliminates SIMD setup overhead for small strings
    if CAP < 16 {
        return scalar::validate_ascii(bytes);
    }

    // CAP < 32: AVX2/AVX-512 loops will never run
    #[cfg(all(
        target_arch = "x86_64",
        any(target_feature = "avx2", target_feature = "avx512bw")
    ))]
    if CAP < 32 {
        // Fall back to scalar since we don't have SSE2 module compiled
        // when AVX2/AVX-512 is enabled. Scalar handles up to 31 bytes efficiently.
        return scalar::validate_ascii(bytes);
    }

    // CAP < 64: AVX-512 loop will never run
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
    if CAP < 64 {
        // Fall back to scalar for medium sizes when only AVX-512 is compiled
        return scalar::validate_ascii(bytes);
    }

    // Full dispatch for larger capacities
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
    {
        avx512::validate_ascii(bytes)
    }

    #[cfg(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        not(target_feature = "avx512bw")
    ))]
    {
        avx2::validate_ascii(bytes)
    }

    #[cfg(all(
        target_arch = "x86_64",
        not(target_feature = "avx2"),
        not(target_feature = "avx512bw")
    ))]
    {
        sse2::validate_ascii(bytes)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::validate_ascii(bytes)
    }
}

// =============================================================================
// Printable Validation
// =============================================================================

/// Validate that all bytes are printable ASCII (0x20-0x7E).
///
/// Returns `Ok(())` if all bytes are printable, or `Err((byte, pos))` with
/// the first non-printable byte and its position.
///
/// Uses the best available SIMD implementation:
/// - AVX-512: 64 bytes at a time
/// - AVX2: 32 bytes at a time
/// - SSE2: 16 bytes at a time (x86_64 baseline)
/// - Scalar: 8 bytes at a time (SWAR)
///
/// # Example
///
/// ```
/// use nexus_ascii::simd;
///
/// assert!(simd::validate_printable(b"Hello, World!").is_ok());
/// assert!(simd::validate_printable(b"\x00").is_err()); // control char
/// assert!(simd::validate_printable(b"\x7F").is_err()); // DEL
/// ```
#[inline]
pub fn validate_printable(bytes: &[u8]) -> Result<(), (u8, usize)> {
    // AVX-512: 64 bytes at a time (highest priority)
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
    {
        avx512::validate_printable(bytes)
    }

    // AVX2: 32 bytes at a time
    #[cfg(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        not(target_feature = "avx512bw")
    ))]
    {
        avx2::validate_printable(bytes)
    }

    // SSE2: 16 bytes at a time (x86_64 baseline)
    #[cfg(all(
        target_arch = "x86_64",
        not(target_feature = "avx2"),
        not(target_feature = "avx512bw")
    ))]
    {
        sse2::validate_printable(bytes)
    }

    // Scalar SWAR: 8 bytes at a time
    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::validate_printable(bytes)
    }
}

/// Validate printable ASCII with compile-time capacity bound.
///
/// Uses `CAP` to eliminate unreachable SIMD code paths at compile time:
/// - `CAP < 16`: Scalar only (SSE2 loop would never run)
/// - `CAP < 32`: SSE2 max (AVX2/AVX-512 loops would never run)
/// - `CAP < 64`: AVX2 max (AVX-512 loop would never run)
/// - `CAP >= 64`: Full SIMD dispatch
///
/// This is more efficient for fixed-capacity types like `AsciiText<7>` where
/// the compiler can inline just the scalar path.
#[inline]
pub fn validate_printable_bounded<const CAP: usize>(bytes: &[u8]) -> Result<(), (u8, usize)> {
    debug_assert!(bytes.len() <= CAP);

    // CAP < 16: SSE2/AVX2/AVX-512 loops will never run, use scalar directly
    if CAP < 16 {
        return scalar::validate_printable(bytes);
    }

    // CAP < 32: AVX2/AVX-512 loops will never run
    #[cfg(all(
        target_arch = "x86_64",
        any(target_feature = "avx2", target_feature = "avx512bw")
    ))]
    if CAP < 32 {
        return scalar::validate_printable(bytes);
    }

    // CAP < 64: AVX-512 loop will never run
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
    if CAP < 64 {
        return scalar::validate_printable(bytes);
    }

    // Full dispatch for larger capacities
    #[cfg(all(target_arch = "x86_64", target_feature = "avx512bw"))]
    {
        avx512::validate_printable(bytes)
    }

    #[cfg(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        not(target_feature = "avx512bw")
    ))]
    {
        avx2::validate_printable(bytes)
    }

    #[cfg(all(
        target_arch = "x86_64",
        not(target_feature = "avx2"),
        not(target_feature = "avx512bw")
    ))]
    {
        sse2::validate_printable(bytes)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        scalar::validate_printable(bytes)
    }
}

// =============================================================================
// Case-Insensitive Comparison
// =============================================================================

/// Compare two byte slices for case-insensitive ASCII equality using SWAR.
///
/// Processes 8 bytes at a time. For ASCII letters (A-Z, a-z), case is ignored.
/// Non-letter ASCII characters must match exactly.
///
/// # Example
///
/// ```
/// use nexus_ascii::simd;
///
/// assert!(simd::eq_ignore_ascii_case(b"Hello", b"HELLO"));
/// assert!(simd::eq_ignore_ascii_case(b"BTC-USD", b"btc-usd"));
/// assert!(!simd::eq_ignore_ascii_case(b"Hello", b"World"));
/// ```
#[inline]
pub fn eq_ignore_ascii_case(a: &[u8], b: &[u8]) -> bool {
    scalar::eq_ignore_ascii_case(a, b)
}

// =============================================================================
// Case Conversion
// =============================================================================

/// Convert byte slice to lowercase in-place using SWAR.
///
/// Processes 8 bytes at a time. Only ASCII letters (A-Z) are affected.
///
/// # Example
///
/// ```
/// use nexus_ascii::simd;
///
/// let mut bytes = *b"Hello, World!";
/// simd::make_lowercase(&mut bytes);
/// assert_eq!(&bytes, b"hello, world!");
/// ```
#[inline]
pub fn make_lowercase(bytes: &mut [u8]) {
    scalar::make_lowercase(bytes)
}

/// Convert byte slice to uppercase in-place using SWAR.
///
/// Processes 8 bytes at a time. Only ASCII letters (a-z) are affected.
///
/// # Example
///
/// ```
/// use nexus_ascii::simd;
///
/// let mut bytes = *b"Hello, World!";
/// simd::make_uppercase(&mut bytes);
/// assert_eq!(&bytes, b"HELLO, WORLD!");
/// ```
#[inline]
pub fn make_uppercase(bytes: &mut [u8]) {
    scalar::make_uppercase(bytes)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // ASCII validation tests
    // -------------------------------------------------------------------------

    #[test]
    fn validate_ascii_empty() {
        assert!(validate_ascii(b"").is_ok());
    }

    #[test]
    fn validate_ascii_single_valid() {
        for b in 0u8..=127 {
            assert!(validate_ascii(&[b]).is_ok(), "byte {} should be valid", b);
        }
    }

    #[test]
    fn validate_ascii_single_invalid() {
        for b in 128u8..=255 {
            let result = validate_ascii(&[b]);
            assert_eq!(result, Err((b, 0)), "byte {} should be invalid", b);
        }
    }

    #[test]
    fn validate_ascii_all_valid() {
        let bytes: Vec<u8> = (0..=127).collect();
        assert!(validate_ascii(&bytes).is_ok());
    }

    #[test]
    fn validate_ascii_invalid_at_various_positions() {
        for len in 1..=64 {
            for pos in 0..len {
                let mut bytes = vec![b'a'; len];
                bytes[pos] = 0x80;
                let result = validate_ascii(&bytes);
                assert_eq!(result, Err((0x80, pos)), "len={}, pos={}", len, pos);
            }
        }
    }

    #[test]
    fn validate_ascii_short_strings() {
        assert!(validate_ascii(b"a").is_ok());
        assert!(validate_ascii(b"ab").is_ok());
        assert!(validate_ascii(b"abc").is_ok());
        assert!(validate_ascii(b"abcdefg").is_ok());
        assert!(validate_ascii(b"abcdefgh").is_ok());
    }

    #[test]
    fn validate_ascii_boundary_lengths() {
        // Test around SIMD boundaries (8, 16, 32)
        for len in [7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65] {
            let bytes = vec![b'x'; len];
            assert!(validate_ascii(&bytes).is_ok(), "len={}", len);
        }
    }

    // -------------------------------------------------------------------------
    // Printable validation tests
    // -------------------------------------------------------------------------

    #[test]
    fn validate_printable_empty() {
        assert!(validate_printable(b"").is_ok());
    }

    #[test]
    fn validate_printable_all_printable() {
        let bytes: Vec<u8> = (0x20..=0x7E).collect();
        assert!(validate_printable(&bytes).is_ok());
    }

    #[test]
    fn validate_printable_control_chars_rejected() {
        for b in 0u8..0x20 {
            let result = validate_printable(&[b]);
            assert_eq!(result, Err((b, 0)), "control char {} should be rejected", b);
        }
    }

    #[test]
    fn validate_printable_del_rejected() {
        let result = validate_printable(&[0x7F]);
        assert_eq!(result, Err((0x7F, 0)));
    }

    #[test]
    fn validate_printable_high_ascii_rejected() {
        for b in 0x80u8..=0xFF {
            let result = validate_printable(&[b]);
            assert_eq!(result, Err((b, 0)), "high byte {} should be rejected", b);
        }
    }

    #[test]
    fn validate_printable_invalid_at_various_positions() {
        for len in 1..=64 {
            for pos in 0..len {
                let mut bytes = vec![b'a'; len];
                bytes[pos] = 0x00; // control char
                let result = validate_printable(&bytes);
                assert_eq!(result, Err((0x00, pos)), "len={}, pos={}", len, pos);
            }
        }
    }

    #[test]
    fn validate_printable_boundary_chars() {
        // Space (0x20) is valid
        assert!(validate_printable(b" ").is_ok());
        // Tilde (0x7E) is valid
        assert!(validate_printable(b"~").is_ok());
        // 0x1F (unit separator) is invalid
        assert_eq!(validate_printable(&[0x1F]), Err((0x1F, 0)));
        // 0x7F (DEL) is invalid
        assert_eq!(validate_printable(&[0x7F]), Err((0x7F, 0)));
    }

    #[test]
    fn validate_printable_boundary_lengths() {
        for len in [7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65] {
            let bytes = vec![b'x'; len];
            assert!(validate_printable(&bytes).is_ok(), "len={}", len);
        }
    }

    // -------------------------------------------------------------------------
    // Cross-implementation consistency (scalar as reference)
    // -------------------------------------------------------------------------

    #[test]
    fn validate_ascii_matches_scalar() {
        for len in 0..=128 {
            let bytes: Vec<u8> = (0..len).map(|i| (i % 128) as u8).collect();
            let dispatch_result = validate_ascii(&bytes);
            let scalar_result = scalar::validate_ascii(&bytes);
            assert_eq!(dispatch_result, scalar_result, "mismatch at len={}", len);
        }
    }

    #[test]
    fn validate_printable_matches_scalar() {
        for len in 0..=128 {
            // Create printable data
            let bytes: Vec<u8> = (0..len).map(|i| (0x20 + (i % 95)) as u8).collect();
            let dispatch_result = validate_printable(&bytes);
            let scalar_result = scalar::validate_printable(&bytes);
            assert_eq!(dispatch_result, scalar_result, "mismatch at len={}", len);
        }
    }

    // -------------------------------------------------------------------------
    // Bounded validation tests
    // -------------------------------------------------------------------------

    #[test]
    fn validate_ascii_bounded_small_cap() {
        // Test with CAP < 16 (scalar path)
        let data = b"BTC-USD";
        assert!(validate_ascii_bounded::<7>(data).is_ok());
        assert!(validate_ascii_bounded::<8>(data).is_ok());
        assert!(validate_ascii_bounded::<15>(data).is_ok());
    }

    #[test]
    fn validate_ascii_bounded_medium_cap() {
        // Test with 16 <= CAP < 32 (SSE2 path on x86_64)
        let data = b"Hello, World!!!!";
        assert!(validate_ascii_bounded::<16>(data).is_ok());
        assert!(validate_ascii_bounded::<20>(data).is_ok());
        assert!(validate_ascii_bounded::<31>(data).is_ok());
    }

    #[test]
    fn validate_ascii_bounded_large_cap() {
        // Test with CAP >= 32 (AVX2 path when available)
        let data = b"This is a longer string for testing";
        assert!(validate_ascii_bounded::<64>(data).is_ok());
        assert!(validate_ascii_bounded::<128>(data).is_ok());
    }

    #[test]
    fn validate_ascii_bounded_matches_unbounded() {
        for len in 0..=64 {
            let bytes: Vec<u8> = (0..len).map(|i| b'A' + (i % 26) as u8).collect();
            let bounded = validate_ascii_bounded::<64>(&bytes);
            let unbounded = validate_ascii(&bytes);
            assert_eq!(bounded, unbounded, "len={}", len);
        }
    }

    #[test]
    fn validate_printable_bounded_small_cap() {
        let data = b"BTC-USD";
        assert!(validate_printable_bounded::<7>(data).is_ok());
        assert!(validate_printable_bounded::<8>(data).is_ok());
        assert!(validate_printable_bounded::<15>(data).is_ok());
    }

    #[test]
    fn validate_printable_bounded_matches_unbounded() {
        for len in 0..=64 {
            let bytes: Vec<u8> = (0..len).map(|i| b'A' + (i % 26) as u8).collect();
            let bounded = validate_printable_bounded::<64>(&bytes);
            let unbounded = validate_printable(&bytes);
            assert_eq!(bounded, unbounded, "len={}", len);
        }
    }
}
