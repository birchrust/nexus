//! Newtype wrappers for ID values.
//!
//! These types provide type safety and encapsulation for generated IDs.
//! Each type wraps an internal representation and provides methods for
//! conversion, parsing, and access to the underlying data.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::Deref;

use nexus_ascii::AsciiString;

// ============================================================================
// UUID Types
// ============================================================================

/// UUID in standard dashed format.
///
/// Format: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` (36 characters)
///
/// This type wraps the string representation of a UUID. It implements
/// `Copy`, `Hash`, `Eq`, and `Deref<Target = str>` for ergonomic usage.
///
/// # Example
///
/// ```rust
/// use nexus_id::uuid::UuidV4;
///
/// let mut generator = UuidV4::new(12345);
/// let id = generator.next();
///
/// // Use as &str via Deref
/// println!("{}", &*id);
///
/// // Or explicitly
/// println!("{}", id.as_str());
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Uuid(pub(crate) AsciiString<36>);

impl Uuid {
    /// Create a new Uuid from raw (hi, lo) bytes.
    ///
    /// # Safety
    ///
    /// This is used internally by generators. The bytes must form a valid UUID.
    #[inline]
    pub(crate) fn from_raw(hi: u64, lo: u64) -> Self {
        Self(crate::encode::uuid_dashed(hi, lo))
    }

    /// Returns the UUID as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns the UUID as a byte slice.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Decode the UUID back to raw (hi, lo) bytes.
    ///
    /// This parses the hex digits and reconstructs the 128-bit value.
    pub fn decode(&self) -> (u64, u64) {
        let bytes = self.0.as_bytes();
        // Parse hex chars, skipping dashes at positions 8, 13, 18, 23
        let mut hi: u64 = 0;
        let mut lo: u64 = 0;

        // Bytes 0-7 (chars 0-7) -> hi bits 32-63
        for &b in &bytes[0..8] {
            hi = (hi << 4) | hex_digit(b) as u64;
        }
        // Bytes 9-12 (chars 9-12, skip dash at 8) -> hi bits 16-31
        for &b in &bytes[9..13] {
            hi = (hi << 4) | hex_digit(b) as u64;
        }
        // Bytes 14-17 (chars 14-17, skip dash at 13) -> hi bits 0-15
        for &b in &bytes[14..18] {
            hi = (hi << 4) | hex_digit(b) as u64;
        }
        // Bytes 19-22 (chars 19-22, skip dash at 18) -> lo bits 48-63
        for &b in &bytes[19..23] {
            lo = (lo << 4) | hex_digit(b) as u64;
        }
        // Bytes 24-35 (chars 24-35, skip dash at 23) -> lo bits 0-47
        for &b in &bytes[24..36] {
            lo = (lo << 4) | hex_digit(b) as u64;
        }

        (hi, lo)
    }

    /// Extract the UUID version (4 bits).
    #[inline]
    pub fn version(&self) -> u8 {
        // Version is char at position 14
        hex_digit(self.0.as_bytes()[14])
    }
}

impl Deref for Uuid {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for Uuid {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uuid({})", self.0.as_str())
    }
}

impl Hash for Uuid {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Use the precomputed hash from AsciiString
        self.0.hash(state);
    }
}

// ============================================================================
// UUID Compact (no dashes)
// ============================================================================

/// UUID in compact format (no dashes).
///
/// Format: `xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx` (32 characters)
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct UuidCompact(pub(crate) AsciiString<32>);

impl UuidCompact {
    #[inline]
    pub(crate) fn from_raw(hi: u64, lo: u64) -> Self {
        Self(crate::encode::hex_u128(hi, lo))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Decode back to raw (hi, lo) bytes.
    pub fn decode(&self) -> (u64, u64) {
        let bytes = self.0.as_bytes();
        let mut hi: u64 = 0;
        let mut lo: u64 = 0;

        for &b in &bytes[0..16] {
            hi = (hi << 4) | hex_digit(b) as u64;
        }
        for &b in &bytes[16..32] {
            lo = (lo << 4) | hex_digit(b) as u64;
        }

        (hi, lo)
    }
}

impl Deref for UuidCompact {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for UuidCompact {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for UuidCompact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl fmt::Debug for UuidCompact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UuidCompact({})", self.0.as_str())
    }
}

impl Hash for UuidCompact {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

// ============================================================================
// HexId64 - Hex-encoded u64
// ============================================================================

/// Hex-encoded 64-bit ID.
///
/// Format: 16 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct HexId64(pub(crate) AsciiString<16>);

impl HexId64 {
    /// Encode a u64 as hex.
    #[inline]
    pub fn encode(value: u64) -> Self {
        Self(crate::encode::hex_u64(value))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Decode back to u64.
    pub fn decode(&self) -> u64 {
        let bytes = self.0.as_bytes();
        let mut value: u64 = 0;
        for &b in bytes {
            value = (value << 4) | hex_digit(b) as u64;
        }
        value
    }
}

impl Deref for HexId64 {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for HexId64 {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for HexId64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl fmt::Debug for HexId64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HexId64({})", self.0.as_str())
    }
}

impl Hash for HexId64 {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

// ============================================================================
// Base62Id - Base62-encoded u64
// ============================================================================

/// Base62-encoded 64-bit ID.
///
/// Format: 11 alphanumeric characters (0-9, A-Z, a-z).
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Base62Id(pub(crate) AsciiString<11>);

impl Base62Id {
    /// Encode a u64 as base62.
    #[inline]
    pub fn encode(value: u64) -> Self {
        Self(crate::encode::base62_u64(value))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Decode back to u64.
    pub fn decode(&self) -> u64 {
        let bytes = self.0.as_bytes();
        let mut value: u64 = 0;
        for &b in bytes {
            value = value * 62 + base62_digit(b) as u64;
        }
        value
    }
}

impl Deref for Base62Id {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for Base62Id {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for Base62Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl fmt::Debug for Base62Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Base62Id({})", self.0.as_str())
    }
}

impl Hash for Base62Id {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

// ============================================================================
// Base36Id - Base36-encoded u64
// ============================================================================

/// Base36-encoded 64-bit ID.
///
/// Format: 13 alphanumeric characters (0-9, a-z), case-insensitive.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Base36Id(pub(crate) AsciiString<13>);

impl Base36Id {
    /// Encode a u64 as base36.
    #[inline]
    pub fn encode(value: u64) -> Self {
        Self(crate::encode::base36_u64(value))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Decode back to u64.
    pub fn decode(&self) -> u64 {
        let bytes = self.0.as_bytes();
        let mut value: u64 = 0;
        for &b in bytes {
            value = value * 36 + base36_digit(b) as u64;
        }
        value
    }
}

impl Deref for Base36Id {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for Base36Id {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for Base36Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl fmt::Debug for Base36Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Base36Id({})", self.0.as_str())
    }
}

impl Hash for Base36Id {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

// ============================================================================
// ULID
// ============================================================================

/// ULID (Universally Unique Lexicographically Sortable Identifier).
///
/// Format: 26 Crockford Base32 characters (128 bits total)
/// - First 10 chars: 48-bit timestamp (milliseconds since Unix epoch)
/// - Last 16 chars: 80 bits of randomness
///
/// ULIDs are lexicographically sortable and monotonically increasing.
///
/// # Example
///
/// ```rust
/// use std::time::{Instant, SystemTime, UNIX_EPOCH};
/// use nexus_id::ulid::UlidGenerator;
///
/// let epoch = Instant::now();
/// let unix_base = SystemTime::now()
///     .duration_since(UNIX_EPOCH)
///     .unwrap()
///     .as_millis() as u64;
///
/// let mut generator = UlidGenerator::new(epoch, unix_base, 12345);
/// let id = generator.next(Instant::now());
/// assert_eq!(id.len(), 26);
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Ulid(pub(crate) AsciiString<26>);

impl Ulid {
    #[inline]
    pub(crate) fn from_raw(timestamp_ms: u64, rand_hi: u16, rand_lo: u64) -> Self {
        Self(crate::encode::ulid_encode(timestamp_ms, rand_hi, rand_lo))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Extract the timestamp (milliseconds since Unix epoch).
    pub fn timestamp_ms(&self) -> u64 {
        let bytes = self.0.as_bytes();
        let mut ts: u64 = 0;

        // Decode first 10 characters (48 bits of timestamp)
        // Char 0: 3 bits, Chars 1-9: 5 bits each = 3 + 45 = 48 bits
        ts = (ts << 3) | crockford32_digit(bytes[0]) as u64;
        for &b in &bytes[1..10] {
            ts = (ts << 5) | crockford32_digit(b) as u64;
        }

        ts
    }

    /// Decode the random portion as (hi: u16, lo: u64).
    pub fn random(&self) -> (u16, u64) {
        let bytes = self.0.as_bytes();

        // Chars 10-13 contain rand_hi (16 bits) spread across boundaries
        // Char 10: bits 11-15 of rand_hi (5 bits)
        // Char 11: bits 6-10 of rand_hi (5 bits)
        // Char 12: bits 1-5 of rand_hi (5 bits)
        // Char 13: bit 0 of rand_hi (1 bit) + bits 60-63 of rand_lo (4 bits)

        let c10 = crockford32_digit(bytes[10]) as u16;
        let c11 = crockford32_digit(bytes[11]) as u16;
        let c12 = crockford32_digit(bytes[12]) as u16;
        let c13 = crockford32_digit(bytes[13]) as u64;

        let rand_hi = (c10 << 11) | (c11 << 6) | (c12 << 1) | ((c13 >> 4) as u16);

        // Chars 13-25 contain rand_lo (64 bits)
        // Char 13 contributes 4 bits (already extracted above for rand_hi)
        let mut rand_lo: u64 = c13 & 0x0F;
        for &b in &bytes[14..26] {
            rand_lo = (rand_lo << 5) | crockford32_digit(b) as u64;
        }

        (rand_hi, rand_lo)
    }
}

impl Deref for Ulid {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for Ulid {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for Ulid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl fmt::Debug for Ulid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ulid({})", self.0.as_str())
    }
}

impl Hash for Ulid {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

// ============================================================================
// Helper functions
// ============================================================================

/// Convert Crockford Base32 character to value (0-31).
#[inline]
#[allow(clippy::match_same_arms)] // Intentional: 0/O/o map to 0, _ is invalid fallback
const fn crockford32_digit(b: u8) -> u8 {
    // Crockford Base32: 0123456789ABCDEFGHJKMNPQRSTVWXYZ
    // Also accepts lowercase
    match b {
        b'0' | b'O' | b'o' => 0,
        b'1' | b'I' | b'i' | b'L' | b'l' => 1,
        b'2' => 2,
        b'3' => 3,
        b'4' => 4,
        b'5' => 5,
        b'6' => 6,
        b'7' => 7,
        b'8' => 8,
        b'9' => 9,
        b'A' | b'a' => 10,
        b'B' | b'b' => 11,
        b'C' | b'c' => 12,
        b'D' | b'd' => 13,
        b'E' | b'e' => 14,
        b'F' | b'f' => 15,
        b'G' | b'g' => 16,
        b'H' | b'h' => 17,
        b'J' | b'j' => 18,
        b'K' | b'k' => 19,
        b'M' | b'm' => 20,
        b'N' | b'n' => 21,
        b'P' | b'p' => 22,
        b'Q' | b'q' => 23,
        b'R' | b'r' => 24,
        b'S' | b's' => 25,
        b'T' | b't' => 26,
        b'V' | b'v' => 27,
        b'W' | b'w' => 28,
        b'X' | b'x' => 29,
        b'Y' | b'y' => 30,
        b'Z' | b'z' => 31,
        _ => 0,
    }
}

/// Convert hex character to value (0-15).
#[inline]
const fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0, // Should never happen for valid IDs
    }
}

/// Convert base62 character to value (0-61).
#[inline]
const fn base62_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'A'..=b'Z' => b - b'A' + 10,
        b'a'..=b'z' => b - b'a' + 36,
        _ => 0,
    }
}

/// Convert base36 character to value (0-35).
#[inline]
const fn base36_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'z' => b - b'a' + 10,
        b'A'..=b'Z' => b - b'A' + 10, // Case insensitive
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_decode_roundtrip() {
        let hi = 0x0123_4567_89AB_CDEF_u64;
        let lo = 0xFEDC_BA98_7654_3210_u64;

        let uuid = Uuid::from_raw(hi, lo);
        let (decoded_hi, decoded_lo) = uuid.decode();

        assert_eq!(hi, decoded_hi);
        assert_eq!(lo, decoded_lo);
    }

    #[test]
    fn uuid_compact_decode_roundtrip() {
        let hi = 0x0123_4567_89AB_CDEF_u64;
        let lo = 0xFEDC_BA98_7654_3210_u64;

        let uuid = UuidCompact::from_raw(hi, lo);
        let (decoded_hi, decoded_lo) = uuid.decode();

        assert_eq!(hi, decoded_hi);
        assert_eq!(lo, decoded_lo);
    }

    #[test]
    fn hex_id64_decode_roundtrip() {
        for value in [0, 1, 12345, u64::MAX, 0xDEAD_BEEF_CAFE_BABE] {
            let id = HexId64::encode(value);
            assert_eq!(id.decode(), value);
        }
    }

    #[test]
    fn base62_id_decode_roundtrip() {
        for value in [0, 1, 12345, u64::MAX] {
            let id = Base62Id::encode(value);
            assert_eq!(id.decode(), value);
        }
    }

    #[test]
    fn base36_id_decode_roundtrip() {
        for value in [0, 1, 12345, u64::MAX] {
            let id = Base36Id::encode(value);
            assert_eq!(id.decode(), value);
        }
    }

    #[test]
    fn uuid_version() {
        // V4 UUID
        let hi = 0x0123_4567_89AB_4DEF_u64; // version 4 at position
        let lo = 0x8EDC_BA98_7654_3210_u64;
        let uuid = Uuid::from_raw(hi, lo);
        assert_eq!(uuid.version(), 4);

        // V7 UUID
        let hi = 0x0123_4567_89AB_7DEF_u64; // version 7 at position
        let uuid = Uuid::from_raw(hi, lo);
        assert_eq!(uuid.version(), 7);
    }

    #[test]
    fn display_works() {
        let uuid = Uuid::from_raw(0x0123_4567_89AB_CDEF, 0xFEDC_BA98_7654_3210);
        let s = format!("{}", uuid);
        assert_eq!(s, "01234567-89ab-cdef-fedc-ba9876543210");
    }

    #[test]
    fn deref_works() {
        let uuid = Uuid::from_raw(0x0123_4567_89AB_CDEF, 0xFEDC_BA98_7654_3210);
        let s: &str = &uuid;
        assert_eq!(s, "01234567-89ab-cdef-fedc-ba9876543210");
    }
}
