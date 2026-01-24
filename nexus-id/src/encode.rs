//! Fast encoding utilities for ID string generation.
//!
//! Provides hex, base36, and base62 encoding optimized for fixed-size
//! integer values. All functions write directly to `AsciiString` buffers
//! with no allocation.
//!
//! Hex encoding dispatches to SIMD (SSSE3 pshufb) when available,
//! falling back to scalar lookup table on other architectures.

use nexus_ascii::AsciiString;

/// Base62 alphabet: 0-9, A-Z, a-z
const BASE62_ALPHABET: &[u8; 62] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Base36 alphabet: 0-9, a-z (lowercase for consistency)
const BASE36_ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Crockford Base32 alphabet: 0-9, A-Z excluding I, L, O, U
/// See: https://www.crockford.com/base32.html
const CROCKFORD32_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 62² — used for digit-pair decomposition in base62 encoding.
const BASE62_SQ: u64 = 62 * 62;

/// 36² — used for digit-pair decomposition in base36 encoding.
const BASE36_SQ: u64 = 36 * 36;

/// Encode a u64 as 16-character lowercase hex.
#[inline]
pub(crate) fn hex_u64(value: u64) -> AsciiString<16> {
    let buf = crate::simd::hex_encode_u64(value);
    // SAFETY: All bytes are valid ASCII hex digits (produced by hex encoder)
    unsafe { AsciiString::from_bytes_unchecked(&buf) }
}

/// Encode two u64s as 32-character lowercase hex.
#[inline]
pub(crate) fn hex_u128(hi: u64, lo: u64) -> AsciiString<32> {
    let buf = crate::simd::hex_encode_u128(hi, lo);
    // SAFETY: All bytes are valid ASCII hex digits (produced by hex encoder)
    unsafe { AsciiString::from_bytes_unchecked(&buf) }
}

/// Encode a u64 as 11-character base62.
///
/// Base62 uses: 0-9, A-Z, a-z (62 characters).
/// Produces fixed-length output with leading zeros.
///
/// Uses digit-pair decomposition: divmod by 62² (3844) per iteration,
/// halving the serial dependency chain from 11 to 6 divisions.
#[inline]
pub(crate) fn base62_u64(mut value: u64) -> AsciiString<16> {
    let mut buf = [b'0'; 11];

    // 11 digits = 5 pairs + 1 single.
    // Each pair: value % 3844 gives a 2-digit remainder,
    // decomposed into (r / 62, r % 62). The remainder decomposition
    // is independent of the next value /= 3844.

    let r = (value % BASE62_SQ) as usize;
    value /= BASE62_SQ;
    buf[9] = BASE62_ALPHABET[r / 62];
    buf[10] = BASE62_ALPHABET[r % 62];

    let r = (value % BASE62_SQ) as usize;
    value /= BASE62_SQ;
    buf[7] = BASE62_ALPHABET[r / 62];
    buf[8] = BASE62_ALPHABET[r % 62];

    let r = (value % BASE62_SQ) as usize;
    value /= BASE62_SQ;
    buf[5] = BASE62_ALPHABET[r / 62];
    buf[6] = BASE62_ALPHABET[r % 62];

    let r = (value % BASE62_SQ) as usize;
    value /= BASE62_SQ;
    buf[3] = BASE62_ALPHABET[r / 62];
    buf[4] = BASE62_ALPHABET[r % 62];

    let r = (value % BASE62_SQ) as usize;
    value /= BASE62_SQ;
    buf[1] = BASE62_ALPHABET[r / 62];
    buf[2] = BASE62_ALPHABET[r % 62];

    buf[0] = BASE62_ALPHABET[value as usize];

    // SAFETY: All bytes are valid ASCII alphanumeric
    unsafe { AsciiString::from_bytes_unchecked(&buf) }
}

/// Encode a u64 as 13-character base36.
///
/// Base36 uses: 0-9, a-z (36 characters, case-insensitive).
/// Produces fixed-length output with leading zeros.
///
/// Uses digit-pair decomposition: divmod by 36² (1296) per iteration,
/// halving the serial dependency chain from 13 to 7 divisions.
#[inline]
pub(crate) fn base36_u64(mut value: u64) -> AsciiString<16> {
    let mut buf = [b'0'; 13];

    // 13 digits = 6 pairs + 1 single.

    let r = (value % BASE36_SQ) as usize;
    value /= BASE36_SQ;
    buf[11] = BASE36_ALPHABET[r / 36];
    buf[12] = BASE36_ALPHABET[r % 36];

    let r = (value % BASE36_SQ) as usize;
    value /= BASE36_SQ;
    buf[9] = BASE36_ALPHABET[r / 36];
    buf[10] = BASE36_ALPHABET[r % 36];

    let r = (value % BASE36_SQ) as usize;
    value /= BASE36_SQ;
    buf[7] = BASE36_ALPHABET[r / 36];
    buf[8] = BASE36_ALPHABET[r % 36];

    let r = (value % BASE36_SQ) as usize;
    value /= BASE36_SQ;
    buf[5] = BASE36_ALPHABET[r / 36];
    buf[6] = BASE36_ALPHABET[r % 36];

    let r = (value % BASE36_SQ) as usize;
    value /= BASE36_SQ;
    buf[3] = BASE36_ALPHABET[r / 36];
    buf[4] = BASE36_ALPHABET[r % 36];

    let r = (value % BASE36_SQ) as usize;
    value /= BASE36_SQ;
    buf[1] = BASE36_ALPHABET[r / 36];
    buf[2] = BASE36_ALPHABET[r % 36];

    buf[0] = BASE36_ALPHABET[value as usize];

    // SAFETY: All bytes are valid ASCII alphanumeric
    unsafe { AsciiString::from_bytes_unchecked(&buf) }
}

/// Format 128-bit value as UUID with dashes.
///
/// Encodes hi and lo as hex, then scatters into dashed format:
/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`
#[inline]
pub(crate) fn uuid_dashed(hi: u64, lo: u64) -> AsciiString<40> {
    let hi_hex = crate::simd::hex_encode_u64(hi);
    let lo_hex = crate::simd::hex_encode_u64(lo);
    let mut buf = [0u8; 36];

    // xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
    buf[0..8].copy_from_slice(&hi_hex[0..8]);
    buf[8] = b'-';
    buf[9..13].copy_from_slice(&hi_hex[8..12]);
    buf[13] = b'-';
    buf[14..18].copy_from_slice(&hi_hex[12..16]);
    buf[18] = b'-';
    buf[19..23].copy_from_slice(&lo_hex[0..4]);
    buf[23] = b'-';
    buf[24..36].copy_from_slice(&lo_hex[4..16]);

    // SAFETY: All bytes are valid ASCII (hex digits and dashes)
    unsafe { AsciiString::from_bytes_unchecked(&buf) }
}

/// Encode ULID as 26-character Crockford Base32.
///
/// ULID layout:
/// - Timestamp (48 bits): 10 characters
/// - Random (80 bits): 16 characters
///
/// Input: timestamp_ms (48 bits used), rand_hi (16 bits), rand_lo (64 bits)
#[inline]
pub(crate) fn ulid_encode(timestamp_ms: u64, rand_hi: u16, rand_lo: u64) -> AsciiString<32> {
    let mut buf = [0u8; 26];

    // Encode timestamp (48 bits → 10 chars)
    // Char 0 uses only 3 bits (48 - 9×5 = 3), chars 1-9 use 5 bits each
    buf[0] = CROCKFORD32_ALPHABET[((timestamp_ms >> 45) & 0x07) as usize]; // top 3 bits
    buf[1] = CROCKFORD32_ALPHABET[((timestamp_ms >> 40) & 0x1F) as usize];
    buf[2] = CROCKFORD32_ALPHABET[((timestamp_ms >> 35) & 0x1F) as usize];
    buf[3] = CROCKFORD32_ALPHABET[((timestamp_ms >> 30) & 0x1F) as usize];
    buf[4] = CROCKFORD32_ALPHABET[((timestamp_ms >> 25) & 0x1F) as usize];
    buf[5] = CROCKFORD32_ALPHABET[((timestamp_ms >> 20) & 0x1F) as usize];
    buf[6] = CROCKFORD32_ALPHABET[((timestamp_ms >> 15) & 0x1F) as usize];
    buf[7] = CROCKFORD32_ALPHABET[((timestamp_ms >> 10) & 0x1F) as usize];
    buf[8] = CROCKFORD32_ALPHABET[((timestamp_ms >> 5) & 0x1F) as usize];
    buf[9] = CROCKFORD32_ALPHABET[(timestamp_ms & 0x1F) as usize];

    // Encode random (80 bits → 16 chars)
    // rand_hi is 16 bits, rand_lo is 64 bits
    // Combine: 80-bit value = (rand_hi << 64) | rand_lo
    // Chars 10-11 come from rand_hi (16 bits → 4 chars but we only need 16 bits which is 3.2 chars)
    // Actually: char 10 uses top 4 bits of rand_hi, chars 11-12 use remaining 12 bits + 3 from lo
    // Let's just compute bit by bit

    // Random layout (80 bits total):
    // - Char 10: bits 75-79 (5 bits from top of rand_hi)
    // - Char 11: bits 70-74
    // - Char 12: bits 65-69
    // - Char 13: bits 60-64 (straddles rand_hi and rand_lo)
    // - Chars 14-25: remaining 60 bits from rand_lo

    let rand_hi = rand_hi as u64;

    // Build a combined 80-bit representation
    // Chars 10-12: top 15 bits are from rand_hi (only 16 bits, so top 15 bits covers chars 10-12 plus 1 bit)
    buf[10] = CROCKFORD32_ALPHABET[((rand_hi >> 11) & 0x1F) as usize]; // bits 11-15 of rand_hi
    buf[11] = CROCKFORD32_ALPHABET[((rand_hi >> 6) & 0x1F) as usize]; // bits 6-10 of rand_hi
    buf[12] = CROCKFORD32_ALPHABET[((rand_hi >> 1) & 0x1F) as usize]; // bits 1-5 of rand_hi

    // Char 13: bit 0 of rand_hi + bits 60-63 of rand_lo
    let combined = ((rand_hi & 0x01) << 4) | ((rand_lo >> 60) & 0x0F);
    buf[13] = CROCKFORD32_ALPHABET[combined as usize];

    // Chars 14-25: remaining 60 bits of rand_lo
    buf[14] = CROCKFORD32_ALPHABET[((rand_lo >> 55) & 0x1F) as usize];
    buf[15] = CROCKFORD32_ALPHABET[((rand_lo >> 50) & 0x1F) as usize];
    buf[16] = CROCKFORD32_ALPHABET[((rand_lo >> 45) & 0x1F) as usize];
    buf[17] = CROCKFORD32_ALPHABET[((rand_lo >> 40) & 0x1F) as usize];
    buf[18] = CROCKFORD32_ALPHABET[((rand_lo >> 35) & 0x1F) as usize];
    buf[19] = CROCKFORD32_ALPHABET[((rand_lo >> 30) & 0x1F) as usize];
    buf[20] = CROCKFORD32_ALPHABET[((rand_lo >> 25) & 0x1F) as usize];
    buf[21] = CROCKFORD32_ALPHABET[((rand_lo >> 20) & 0x1F) as usize];
    buf[22] = CROCKFORD32_ALPHABET[((rand_lo >> 15) & 0x1F) as usize];
    buf[23] = CROCKFORD32_ALPHABET[((rand_lo >> 10) & 0x1F) as usize];
    buf[24] = CROCKFORD32_ALPHABET[((rand_lo >> 5) & 0x1F) as usize];
    buf[25] = CROCKFORD32_ALPHABET[(rand_lo & 0x1F) as usize];

    // SAFETY: All bytes are valid ASCII (Crockford base32 characters)
    unsafe { AsciiString::from_bytes_unchecked(&buf) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_u64_zero() {
        assert_eq!(hex_u64(0).as_str(), "0000000000000000");
    }

    #[test]
    fn hex_u64_max() {
        assert_eq!(hex_u64(u64::MAX).as_str(), "ffffffffffffffff");
    }

    #[test]
    fn hex_u64_known_value() {
        assert_eq!(hex_u64(0xDEAD_BEEF_CAFE_BABE).as_str(), "deadbeefcafebabe");
    }

    #[test]
    fn hex_u128_known_value() {
        let hi = 0x0123_4567_89AB_CDEF;
        let lo = 0xFEDC_BA98_7654_3210;
        assert_eq!(
            hex_u128(hi, lo).as_str(),
            "0123456789abcdeffedcba9876543210"
        );
    }

    #[test]
    fn base62_zero() {
        assert_eq!(base62_u64(0).as_str(), "00000000000");
    }

    #[test]
    fn base62_max() {
        // u64::MAX in base62 should be 11 chars
        let encoded = base62_u64(u64::MAX);
        assert_eq!(encoded.len(), 11);
        // Verify it's all valid base62 chars
        for c in encoded.as_str().chars() {
            assert!(c.is_ascii_alphanumeric());
        }
    }

    #[test]
    fn base62_known_values() {
        // 62^0 = 1, so base62(61) = "0000000000z" (z is index 61)
        // Actually let's verify with small numbers
        assert_eq!(base62_u64(0).as_str(), "00000000000");
        assert_eq!(base62_u64(1).as_str(), "00000000001");
        assert_eq!(base62_u64(9).as_str(), "00000000009");
        assert_eq!(base62_u64(10).as_str(), "0000000000A");
        assert_eq!(base62_u64(35).as_str(), "0000000000Z");
        assert_eq!(base62_u64(36).as_str(), "0000000000a");
        assert_eq!(base62_u64(61).as_str(), "0000000000z");
        assert_eq!(base62_u64(62).as_str(), "00000000010");
    }

    #[test]
    fn base36_zero() {
        assert_eq!(base36_u64(0).as_str(), "0000000000000");
    }

    #[test]
    fn base36_max() {
        let encoded = base36_u64(u64::MAX);
        assert_eq!(encoded.len(), 13);
        // Should be all lowercase alphanumeric
        for c in encoded.as_str().chars() {
            assert!(c.is_ascii_digit() || (c.is_ascii_lowercase()));
        }
    }

    #[test]
    fn base36_known_values() {
        assert_eq!(base36_u64(0).as_str(), "0000000000000");
        assert_eq!(base36_u64(1).as_str(), "0000000000001");
        assert_eq!(base36_u64(9).as_str(), "0000000000009");
        assert_eq!(base36_u64(10).as_str(), "000000000000a");
        assert_eq!(base36_u64(35).as_str(), "000000000000z");
        assert_eq!(base36_u64(36).as_str(), "0000000000010");
    }

    #[test]
    fn uuid_dashed_format() {
        let hi = 0x0123_4567_89AB_CDEF;
        let lo = 0xFEDC_BA98_7654_3210;
        let uuid = uuid_dashed(hi, lo);
        assert_eq!(uuid.as_str(), "01234567-89ab-cdef-fedc-ba9876543210");
        assert_eq!(uuid.len(), 36);
    }

    #[test]
    fn uuid_dashed_zeros() {
        let uuid = uuid_dashed(0, 0);
        assert_eq!(uuid.as_str(), "00000000-0000-0000-0000-000000000000");
    }
}
