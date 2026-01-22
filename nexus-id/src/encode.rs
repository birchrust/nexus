//! Fast encoding utilities for ID string generation.
//!
//! Provides hex, base36, and base62 encoding optimized for fixed-size
//! integer values. All functions write directly to `AsciiString` buffers
//! with no allocation.

use nexus_ascii::AsciiString;

/// Lookup table for byte-to-hex conversion.
/// Each entry is the 2-char lowercase hex representation of that byte.
const HEX_TABLE: [[u8; 2]; 256] = {
    let mut table = [[0u8; 2]; 256];
    let hex_chars = b"0123456789abcdef";
    let mut i = 0;
    while i < 256 {
        table[i][0] = hex_chars[i >> 4];
        table[i][1] = hex_chars[i & 0xF];
        i += 1;
    }
    table
};

/// Base62 alphabet: 0-9, A-Z, a-z
const BASE62_ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Base36 alphabet: 0-9, a-z (lowercase for consistency)
const BASE36_ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Crockford Base32 alphabet: 0-9, A-Z excluding I, L, O, U
/// See: https://www.crockford.com/base32.html
const CROCKFORD32_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Encode a u64 as 16-character lowercase hex.
#[inline]
pub(crate) fn hex_u64(value: u64) -> AsciiString<16> {
    let bytes = value.to_be_bytes();
    let mut buf = [0u8; 16];

    // Unrolled for performance
    let h0 = HEX_TABLE[bytes[0] as usize];
    let h1 = HEX_TABLE[bytes[1] as usize];
    let h2 = HEX_TABLE[bytes[2] as usize];
    let h3 = HEX_TABLE[bytes[3] as usize];
    let h4 = HEX_TABLE[bytes[4] as usize];
    let h5 = HEX_TABLE[bytes[5] as usize];
    let h6 = HEX_TABLE[bytes[6] as usize];
    let h7 = HEX_TABLE[bytes[7] as usize];

    buf[0] = h0[0];
    buf[1] = h0[1];
    buf[2] = h1[0];
    buf[3] = h1[1];
    buf[4] = h2[0];
    buf[5] = h2[1];
    buf[6] = h3[0];
    buf[7] = h3[1];
    buf[8] = h4[0];
    buf[9] = h4[1];
    buf[10] = h5[0];
    buf[11] = h5[1];
    buf[12] = h6[0];
    buf[13] = h6[1];
    buf[14] = h7[0];
    buf[15] = h7[1];

    // SAFETY: All bytes are valid ASCII hex digits
    unsafe { AsciiString::from_bytes_unchecked(&buf) }
}

/// Encode two u64s as 32-character lowercase hex.
#[inline]
pub(crate) fn hex_u128(hi: u64, lo: u64) -> AsciiString<32> {
    let hi_bytes = hi.to_be_bytes();
    let lo_bytes = lo.to_be_bytes();
    let mut buf = [0u8; 32];

    // High 64 bits
    for i in 0..8 {
        let h = HEX_TABLE[hi_bytes[i] as usize];
        buf[i * 2] = h[0];
        buf[i * 2 + 1] = h[1];
    }

    // Low 64 bits
    for i in 0..8 {
        let h = HEX_TABLE[lo_bytes[i] as usize];
        buf[16 + i * 2] = h[0];
        buf[16 + i * 2 + 1] = h[1];
    }

    // SAFETY: All bytes are valid ASCII hex digits
    unsafe { AsciiString::from_bytes_unchecked(&buf) }
}

/// Encode a u64 as 11-character base62.
///
/// Base62 uses: 0-9, A-Z, a-z (62 characters).
/// Produces fixed-length output with leading zeros.
#[inline]
pub(crate) fn base62_u64(mut value: u64) -> AsciiString<11> {
    let mut buf = [b'0'; 11]; // Initialize with '0' for leading zeros
    let mut i = 10;

    // Extract digits from least significant
    if value == 0 {
        // Already filled with '0's
    } else {
        while value > 0 {
            buf[i] = BASE62_ALPHABET[(value % 62) as usize];
            value /= 62;
            i = i.saturating_sub(1);
        }
    }

    // SAFETY: All bytes are valid ASCII alphanumeric
    unsafe { AsciiString::from_bytes_unchecked(&buf) }
}

/// Encode a u64 as 13-character base36.
///
/// Base36 uses: 0-9, a-z (36 characters, case-insensitive).
/// Produces fixed-length output with leading zeros.
#[inline]
pub(crate) fn base36_u64(mut value: u64) -> AsciiString<13> {
    let mut buf = [b'0'; 13]; // Initialize with '0' for leading zeros
    let mut i = 12;

    if value == 0 {
        // Already filled with '0's
    } else {
        while value > 0 {
            buf[i] = BASE36_ALPHABET[(value % 36) as usize];
            value /= 36;
            i = i.saturating_sub(1);
        }
    }

    // SAFETY: All bytes are valid ASCII alphanumeric
    unsafe { AsciiString::from_bytes_unchecked(&buf) }
}

/// Format 128-bit value as UUID with dashes.
#[inline]
pub(crate) fn uuid_dashed(hi: u64, lo: u64) -> AsciiString<36> {
    let hi_bytes = hi.to_be_bytes();
    let lo_bytes = lo.to_be_bytes();
    let mut buf = [0u8; 36];

    // xxxxxxxx- (bytes 0-3 of hi)
    for i in 0..4 {
        let h = HEX_TABLE[hi_bytes[i] as usize];
        buf[i * 2] = h[0];
        buf[i * 2 + 1] = h[1];
    }
    buf[8] = b'-';

    // xxxx- (bytes 4-5 of hi)
    for i in 0..2 {
        let h = HEX_TABLE[hi_bytes[4 + i] as usize];
        buf[9 + i * 2] = h[0];
        buf[9 + i * 2 + 1] = h[1];
    }
    buf[13] = b'-';

    // xxxx- (bytes 6-7 of hi)
    for i in 0..2 {
        let h = HEX_TABLE[hi_bytes[6 + i] as usize];
        buf[14 + i * 2] = h[0];
        buf[14 + i * 2 + 1] = h[1];
    }
    buf[18] = b'-';

    // xxxx- (bytes 0-1 of lo)
    for i in 0..2 {
        let h = HEX_TABLE[lo_bytes[i] as usize];
        buf[19 + i * 2] = h[0];
        buf[19 + i * 2 + 1] = h[1];
    }
    buf[23] = b'-';

    // xxxxxxxxxxxx (bytes 2-7 of lo)
    for i in 0..6 {
        let h = HEX_TABLE[lo_bytes[2 + i] as usize];
        buf[24 + i * 2] = h[0];
        buf[24 + i * 2 + 1] = h[1];
    }

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
pub(crate) fn ulid_encode(timestamp_ms: u64, rand_hi: u16, rand_lo: u64) -> AsciiString<26> {
    let mut buf = [0u8; 26];

    // Encode timestamp (48 bits → 10 chars)
    // We encode from the least significant 5 bits first, then reverse
    // Char 0 uses only 2 bits from timestamp (bits 46-47)
    // Chars 1-9 use 5 bits each
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
        assert_eq!(hex_u128(hi, lo).as_str(), "0123456789abcdeffedcba9876543210");
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
