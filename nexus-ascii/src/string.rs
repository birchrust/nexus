//! Fixed-capacity ASCII string type.

use core::hash::{Hash, Hasher};

use crate::hash;
use crate::AsciiError;

// =============================================================================
// Header Packing
// =============================================================================

/// Pack length and hash into a single u64 header.
///
/// Layout: bits 0-15 = length, bits 16-63 = upper 48 bits of hash.
#[inline(always)]
const fn pack_header(len: u16, hash: u64) -> u64 {
    // Clear lower 16 bits of hash, insert length
    (hash & 0xFFFF_FFFF_FFFF_0000) | (len as u64)
}

/// Extract length from header.
#[inline(always)]
const fn unpack_len(header: u64) -> usize {
    (header & 0xFFFF) as usize
}

/// Compute header for empty string.
/// Note: Not const because hash::hash is not const (yet).
#[inline(always)]
fn empty_header() -> u64 {
    pack_header(0, hash::hash::<0>(&[]))
}

// =============================================================================
// AsciiString
// =============================================================================

/// A fixed-capacity, immutable ASCII string.
///
/// `AsciiString<CAP>` stores up to `CAP` ASCII bytes inline with a precomputed
/// hash. The hash and length are packed into a single `u64` header, enabling
/// fast equality checks (single 64-bit comparison rejects most non-equal strings).
///
/// # Design
///
/// - **Immutable**: Once created, the string cannot be modified. This guarantees
///   the hash is always valid.
/// - **Copy**: Always implements `Copy`. For move semantics, wrap in a newtype.
/// - **Full ASCII**: Accepts bytes 0x00-0x7F. For printable-only, use `AsciiText`.
///
/// # Example
///
/// ```
/// use nexus_ascii::AsciiString;
///
/// let s: AsciiString<32> = AsciiString::try_from("hello")?;
/// assert_eq!(s.len(), 5);
/// assert_eq!(s.as_str(), "hello");
/// # Ok::<(), nexus_ascii::AsciiError>(())
/// ```
#[derive(Clone, Copy)]
#[repr(C)]
pub struct AsciiString<const CAP: usize> {
    /// Packed header: bits 0-15 = length, bits 16-63 = hash (upper 48 bits).
    header: u64,
    /// Raw ASCII bytes. Only `len()` bytes are valid.
    data: [u8; CAP],
}

// =============================================================================
// Constructors
// =============================================================================

impl<const CAP: usize> AsciiString<CAP> {
    /// Creates an empty ASCII string.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiString;
    ///
    /// let s: AsciiString<32> = AsciiString::empty();
    /// assert!(s.is_empty());
    /// assert_eq!(s.len(), 0);
    /// ```
    #[inline]
    pub fn empty() -> Self {
        Self {
            header: empty_header(),
            data: [0u8; CAP],
        }
    }

    /// Creates an ASCII string from a byte slice without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - All bytes are valid ASCII (0x00-0x7F)
    /// - `bytes.len() <= CAP`
    ///
    /// Violating these invariants causes undefined behavior in downstream code
    /// that assumes ASCII validity.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiString;
    ///
    /// let bytes = b"HELLO";
    /// // SAFETY: bytes are known ASCII and len <= 32
    /// let s: AsciiString<32> = unsafe { AsciiString::from_bytes_unchecked(bytes) };
    /// assert_eq!(s.as_str(), "HELLO");
    /// ```
    #[inline]
    pub unsafe fn from_bytes_unchecked(bytes: &[u8]) -> Self {
        debug_assert!(bytes.len() <= CAP, "bytes exceed capacity");
        debug_assert!(
            bytes.iter().all(|&b| b <= 127),
            "bytes contain non-ASCII"
        );

        let len = bytes.len();
        let hash = hash::hash::<CAP>(bytes);
        let header = pack_header(len as u16, hash);

        let mut data = [0u8; CAP];
        // SAFETY: len <= CAP guaranteed by caller
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), data.as_mut_ptr(), len);
        }

        Self { header, data }
    }

    /// Attempts to create an ASCII string from a byte slice.
    ///
    /// Returns an error if:
    /// - The slice is longer than `CAP` ([`AsciiError::TooLong`])
    /// - Any byte is > 127 ([`AsciiError::InvalidByte`])
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiString, AsciiError};
    ///
    /// let s: AsciiString<8> = AsciiString::try_from_bytes(b"hello")?;
    /// assert_eq!(s.as_str(), "hello");
    ///
    /// // Too long
    /// let err = AsciiString::<4>::try_from_bytes(b"hello").unwrap_err();
    /// assert!(matches!(err, AsciiError::TooLong { .. }));
    ///
    /// // Invalid ASCII
    /// let err = AsciiString::<8>::try_from_bytes(&[0xFF]).unwrap_err();
    /// assert!(matches!(err, AsciiError::InvalidByte { .. }));
    /// # Ok::<(), AsciiError>(())
    /// ```
    #[inline]
    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, AsciiError> {
        if bytes.len() > CAP {
            return Err(AsciiError::TooLong {
                len: bytes.len(),
                cap: CAP,
            });
        }

        // Validate ASCII
        for (pos, &byte) in bytes.iter().enumerate() {
            if byte > 127 {
                return Err(AsciiError::InvalidByte { byte, pos });
            }
        }

        // SAFETY: We just validated all bytes are ASCII and len <= CAP
        Ok(unsafe { Self::from_bytes_unchecked(bytes) })
    }

    /// Attempts to create an ASCII string from a string slice.
    ///
    /// This is equivalent to `try_from_bytes(s.as_bytes())`.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiString;
    ///
    /// let s: AsciiString<32> = AsciiString::try_from_str("BTC-USD")?;
    /// assert_eq!(s.as_str(), "BTC-USD");
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn try_from_str(s: &str) -> Result<Self, AsciiError> {
        Self::try_from_bytes(s.as_bytes())
    }
}

// =============================================================================
// Accessors
// =============================================================================

impl<const CAP: usize> AsciiString<CAP> {
    /// Returns the length of the string in bytes.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiString;
    ///
    /// let s: AsciiString<32> = AsciiString::try_from("hello")?;
    /// assert_eq!(s.len(), 5);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline(always)]
    pub const fn len(&self) -> usize {
        unpack_len(self.header)
    }

    /// Returns `true` if the string is empty.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiString;
    ///
    /// let empty: AsciiString<32> = AsciiString::empty();
    /// assert!(empty.is_empty());
    ///
    /// let s: AsciiString<32> = AsciiString::try_from("x")?;
    /// assert!(!s.is_empty());
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the maximum capacity of the string.
    ///
    /// This is always equal to the const generic `CAP`.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiString;
    ///
    /// let s: AsciiString<32> = AsciiString::empty();
    /// assert_eq!(s.capacity(), 32);
    /// ```
    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        CAP
    }

    /// Returns the string as a byte slice.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiString;
    ///
    /// let s: AsciiString<32> = AsciiString::try_from("hello")?;
    /// assert_eq!(s.as_bytes(), b"hello");
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: len is always <= CAP and data[..len] contains valid ASCII
        unsafe { self.data.get_unchecked(..self.len()) }
    }

    /// Returns the string as a `&str`.
    ///
    /// This is a zero-cost conversion since ASCII is valid UTF-8.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiString;
    ///
    /// let s: AsciiString<32> = AsciiString::try_from("hello")?;
    /// assert_eq!(s.as_str(), "hello");
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline(always)]
    pub fn as_str(&self) -> &str {
        // SAFETY: ASCII is always valid UTF-8
        unsafe { core::str::from_utf8_unchecked(self.as_bytes()) }
    }

    /// Returns the packed header (for advanced use).
    ///
    /// The header contains both length (bits 0-15) and hash (bits 16-63).
    /// This is primarily useful for debugging or low-level operations.
    #[inline(always)]
    pub const fn header(&self) -> u64 {
        self.header
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl<const CAP: usize> Default for AsciiString<CAP> {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl<const CAP: usize> PartialEq for AsciiString<CAP> {
    /// Compares two ASCII strings for equality.
    ///
    /// This uses a fast path: first compare the 64-bit headers (which include
    /// both length and hash). If headers differ, the strings are definitely
    /// not equal. If headers match, fall back to byte comparison.
    ///
    /// The fast path rejects most non-equal strings with a single comparison.
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        // Fast path: header includes length + hash
        // Different headers = definitely different strings
        if self.header != other.header {
            return false;
        }

        // Headers match (same length + same hash)
        // Must verify actual content (rare to reach here for non-equal strings)
        self.as_bytes() == other.as_bytes()
    }
}

impl<const CAP: usize> Eq for AsciiString<CAP> {}

impl<const CAP: usize> Hash for AsciiString<CAP> {
    /// Hashes the ASCII string.
    ///
    /// Uses the precomputed hash from the header, extracting the upper 48 bits
    /// and passing them to the hasher.
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        // The header itself is a good hash - it contains length + 48-bit hash
        // Using the whole header means equal strings hash equally
        self.header.hash(state);
    }
}

impl<const CAP: usize> core::fmt::Debug for AsciiString<CAP> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AsciiString")
            .field("value", &self.as_str())
            .field("len", &self.len())
            .field("cap", &CAP)
            .finish()
    }
}

impl<const CAP: usize> core::fmt::Display for AsciiString<CAP> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

// =============================================================================
// TryFrom Implementations
// =============================================================================

impl<const CAP: usize> TryFrom<&str> for AsciiString<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::try_from_str(s)
    }
}

impl<const CAP: usize> TryFrom<&[u8]> for AsciiString<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::try_from_bytes(bytes)
    }
}

impl<const CAP: usize> TryFrom<String> for AsciiString<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::try_from_str(&s)
    }
}

impl<const CAP: usize> TryFrom<&String> for AsciiString<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: &String) -> Result<Self, Self::Error> {
        Self::try_from_str(s)
    }
}

// =============================================================================
// AsRef Implementations
// =============================================================================

impl<const CAP: usize> AsRef<str> for AsciiString<CAP> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const CAP: usize> AsRef<[u8]> for AsciiString<CAP> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string() {
        let s: AsciiString<32> = AsciiString::empty();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s.as_str(), "");
        assert_eq!(s.as_bytes(), b"");
    }

    #[test]
    fn from_str() {
        let s: AsciiString<32> = AsciiString::try_from("hello").unwrap();
        assert_eq!(s.len(), 5);
        assert_eq!(s.as_str(), "hello");
    }

    #[test]
    fn from_bytes() {
        let s: AsciiString<32> = AsciiString::try_from_bytes(b"world").unwrap();
        assert_eq!(s.len(), 5);
        assert_eq!(s.as_str(), "world");
    }

    #[test]
    fn too_long() {
        let result = AsciiString::<4>::try_from("hello");
        assert!(matches!(
            result,
            Err(AsciiError::TooLong { len: 5, cap: 4 })
        ));
    }

    #[test]
    fn invalid_ascii() {
        let result = AsciiString::<32>::try_from_bytes(&[0x80]);
        assert!(matches!(
            result,
            Err(AsciiError::InvalidByte { byte: 0x80, pos: 0 })
        ));

        let result = AsciiString::<32>::try_from_bytes(&[b'a', b'b', 0xFF]);
        assert!(matches!(
            result,
            Err(AsciiError::InvalidByte { byte: 0xFF, pos: 2 })
        ));
    }

    #[test]
    fn equality_same() {
        let s1: AsciiString<32> = AsciiString::try_from("test").unwrap();
        let s2: AsciiString<32> = AsciiString::try_from("test").unwrap();
        assert_eq!(s1, s2);
    }

    #[test]
    fn equality_different() {
        let s1: AsciiString<32> = AsciiString::try_from("test").unwrap();
        let s2: AsciiString<32> = AsciiString::try_from("other").unwrap();
        assert_ne!(s1, s2);
    }

    #[test]
    fn equality_different_length() {
        let s1: AsciiString<32> = AsciiString::try_from("test").unwrap();
        let s2: AsciiString<32> = AsciiString::try_from("testing").unwrap();
        assert_ne!(s1, s2);
    }

    #[test]
    fn hash_consistency() {
        use std::collections::hash_map::DefaultHasher;

        let s1: AsciiString<32> = AsciiString::try_from("test").unwrap();
        let s2: AsciiString<32> = AsciiString::try_from("test").unwrap();

        let mut h1 = DefaultHasher::new();
        let mut h2 = DefaultHasher::new();
        s1.hash(&mut h1);
        s2.hash(&mut h2);

        assert_eq!(h1.finish(), h2.finish());
    }

    #[test]
    fn hash_in_hashmap() {
        use std::collections::HashMap;

        let mut map: HashMap<AsciiString<32>, i32> = HashMap::new();

        let key: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
        map.insert(key, 42);

        let lookup: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
        assert_eq!(map.get(&lookup), Some(&42));
    }

    #[test]
    fn default_is_empty() {
        let s: AsciiString<32> = Default::default();
        assert!(s.is_empty());
    }

    #[test]
    fn display() {
        let s: AsciiString<32> = AsciiString::try_from("hello").unwrap();
        assert_eq!(format!("{}", s), "hello");
    }

    #[test]
    fn debug() {
        let s: AsciiString<32> = AsciiString::try_from("hi").unwrap();
        let debug = format!("{:?}", s);
        assert!(debug.contains("AsciiString"));
        assert!(debug.contains("hi"));
    }

    #[test]
    fn copy_semantics() {
        let s1: AsciiString<32> = AsciiString::try_from("copy").unwrap();
        let s2 = s1; // Copy
        assert_eq!(s1, s2); // s1 still valid
    }

    #[test]
    fn capacity() {
        let s: AsciiString<64> = AsciiString::empty();
        assert_eq!(s.capacity(), 64);
    }

    #[test]
    fn as_ref_str() {
        let s: AsciiString<32> = AsciiString::try_from("test").unwrap();
        let r: &str = s.as_ref();
        assert_eq!(r, "test");
    }

    #[test]
    fn as_ref_bytes() {
        let s: AsciiString<32> = AsciiString::try_from("test").unwrap();
        let r: &[u8] = s.as_ref();
        assert_eq!(r, b"test");
    }

    #[test]
    fn full_capacity() {
        let input = "12345678";
        let s: AsciiString<8> = AsciiString::try_from(input).unwrap();
        assert_eq!(s.len(), 8);
        assert_eq!(s.as_str(), input);
    }

    #[test]
    fn control_characters_allowed() {
        // Full ASCII includes control characters
        let s: AsciiString<8> = AsciiString::try_from_bytes(&[0x01, 0x02, 0x03]).unwrap();
        assert_eq!(s.len(), 3);
    }
}
