//! Scalar hex encode/decode implementations.
//!
//! Reference implementations using lookup tables. Used as fallback
//! on non-x86_64 architectures or when SIMD features are unavailable.

/// Lookup table for byte-to-hex conversion.
/// Each entry is the 2-char lowercase hex representation of that byte.
#[allow(dead_code)] // Used when SSSE3 is not available
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

/// Encode a u64 as 16 lowercase hex bytes.
#[allow(dead_code)] // Used when SSSE3 is not available
#[inline]
pub fn hex_encode_u64(value: u64) -> [u8; 16] {
    let bytes = value.to_be_bytes();
    let mut buf = [0u8; 16];

    let mut i = 0;
    while i < 8 {
        let h = HEX_TABLE[bytes[i] as usize];
        buf[i * 2] = h[0];
        buf[i * 2 + 1] = h[1];
        i += 1;
    }

    buf
}

/// Encode two u64s as 32 lowercase hex bytes.
#[allow(dead_code)] // Used when SSSE3 is not available
#[inline]
pub fn hex_encode_u128(hi: u64, lo: u64) -> [u8; 32] {
    let hi_bytes = hi.to_be_bytes();
    let lo_bytes = lo.to_be_bytes();
    let mut buf = [0u8; 32];

    let mut i = 0;
    while i < 8 {
        let h = HEX_TABLE[hi_bytes[i] as usize];
        buf[i * 2] = h[0];
        buf[i * 2 + 1] = h[1];
        i += 1;
    }

    i = 0;
    while i < 8 {
        let h = HEX_TABLE[lo_bytes[i] as usize];
        buf[16 + i * 2] = h[0];
        buf[16 + i * 2 + 1] = h[1];
        i += 1;
    }

    buf
}

/// Decode a hex nibble value from an ASCII byte.
/// Returns 0-15 for valid hex, 0xFF for invalid.
#[allow(dead_code)] // Used on non-x86_64 targets
#[inline(always)]
const fn hex_nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0xFF,
    }
}

/// Decode 16 hex chars to u64.
/// Returns Err(position) on first invalid character.
#[allow(dead_code)] // Used on non-x86_64 targets
#[inline]
pub fn hex_decode_16(bytes: &[u8; 16]) -> Result<u64, usize> {
    let mut value: u64 = 0;
    let mut i = 0;
    while i < 16 {
        let nibble = hex_nibble(bytes[i]);
        if nibble == 0xFF {
            return Err(i);
        }
        value = (value << 4) | nibble as u64;
        i += 1;
    }
    Ok(value)
}

/// Decode 32 hex chars to (hi, lo) u64 pair.
/// Returns Err(position) on first invalid character.
#[allow(dead_code)] // Used on non-x86_64 targets
#[inline]
pub fn hex_decode_32(bytes: &[u8; 32]) -> Result<(u64, u64), usize> {
    let mut hi: u64 = 0;
    let mut i = 0;
    while i < 16 {
        let nibble = hex_nibble(bytes[i]);
        if nibble == 0xFF {
            return Err(i);
        }
        hi = (hi << 4) | nibble as u64;
        i += 1;
    }
    let mut lo: u64 = 0;
    while i < 32 {
        let nibble = hex_nibble(bytes[i]);
        if nibble == 0xFF {
            return Err(i);
        }
        lo = (lo << 4) | nibble as u64;
        i += 1;
    }
    Ok((hi, lo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_u64_zero() {
        assert_eq!(&hex_encode_u64(0), b"0000000000000000");
    }

    #[test]
    fn encode_u64_max() {
        assert_eq!(&hex_encode_u64(u64::MAX), b"ffffffffffffffff");
    }

    #[test]
    fn encode_u64_known() {
        assert_eq!(&hex_encode_u64(0xDEAD_BEEF_CAFE_BABE), b"deadbeefcafebabe");
    }

    #[test]
    fn encode_u128_known() {
        let buf = hex_encode_u128(0x0123_4567_89AB_CDEF, 0xFEDC_BA98_7654_3210);
        assert_eq!(&buf, b"0123456789abcdeffedcba9876543210");
    }

    #[test]
    fn decode_16_valid() {
        assert_eq!(
            hex_decode_16(b"deadbeefcafebabe"),
            Ok(0xDEAD_BEEF_CAFE_BABE)
        );
        assert_eq!(
            hex_decode_16(b"DEADBEEFCAFEBABE"),
            Ok(0xDEAD_BEEF_CAFE_BABE)
        );
        assert_eq!(
            hex_decode_16(b"DeAdBeEfCaFeBaBe"),
            Ok(0xDEAD_BEEF_CAFE_BABE)
        );
        assert_eq!(hex_decode_16(b"0000000000000000"), Ok(0));
        assert_eq!(hex_decode_16(b"ffffffffffffffff"), Ok(u64::MAX));
    }

    #[test]
    fn decode_16_invalid() {
        assert_eq!(hex_decode_16(b"deadbeefcafebage"), Err(14)); // 'g' at pos 14
        assert_eq!(hex_decode_16(b"zzzzzzzzzzzzzzzz"), Err(0));
        assert_eq!(hex_decode_16(b"0000000000000g00"), Err(13));
    }

    #[test]
    fn decode_32_valid() {
        let result = hex_decode_32(b"0123456789abcdeffedcba9876543210");
        assert_eq!(result, Ok((0x0123_4567_89AB_CDEF, 0xFEDC_BA98_7654_3210)));
    }

    #[test]
    fn decode_32_invalid_hi() {
        assert_eq!(hex_decode_32(b"0123456789abcdexfedcba9876543210"), Err(15));
    }

    #[test]
    fn decode_32_invalid_lo() {
        assert_eq!(hex_decode_32(b"0123456789abcdef0000000000g00000"), Err(26));
    }

    #[test]
    fn encode_decode_roundtrip() {
        for value in [0u64, 1, 42, 0xDEAD_BEEF, u64::MAX, 0x0123_4567_89AB_CDEF] {
            let encoded = hex_encode_u64(value);
            let decoded = hex_decode_16(&encoded).unwrap();
            assert_eq!(decoded, value);
        }
    }
}
