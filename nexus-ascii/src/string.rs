//! Fixed-capacity ASCII string type.

use core::hash::{Hash, Hasher};

use crate::AsciiError;
use crate::char::AsciiChar;
use crate::hash;

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

/// Compute header for empty string (runtime).
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

    /// Creates an ASCII string from a static string literal at compile time.
    ///
    /// This is a `const fn` that validates the input and computes the hash
    /// at compile time. Invalid input (non-ASCII or too long) causes a
    /// compile-time panic.
    ///
    /// # Panics
    ///
    /// Panics at compile time if:
    /// - The string contains non-ASCII bytes (> 127)
    /// - The string is longer than `CAP`
    /// - `CAP > 128` (const hash limitation)
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiString;
    ///
    /// // Compile-time construction
    /// const BTC: AsciiString<16> = AsciiString::from_static("BTC-USD");
    /// const ETH: AsciiString<16> = AsciiString::from_static("ETH-USD");
    ///
    /// assert_eq!(BTC.as_str(), "BTC-USD");
    /// assert_eq!(ETH.len(), 7);
    /// ```
    #[inline]
    pub const fn from_static(s: &'static str) -> Self {
        assert!(CAP <= 128, "from_static only supports CAP <= 128");

        let bytes = s.as_bytes();
        let len = bytes.len();

        assert!(len <= CAP, "string exceeds capacity");

        // Validate ASCII at compile time
        let mut i = 0;
        while i < len {
            assert!(bytes[i] <= 127, "string contains non-ASCII byte");
            i += 1;
        }

        // Compute hash at compile time
        let h = hash::hash_const::<CAP>(bytes);
        let header = pack_header(len as u16, h);

        // Copy bytes into data array
        let mut data = [0u8; CAP];
        let mut j = 0;
        while j < len {
            data[j] = bytes[j];
            j += 1;
        }

        Self { header, data }
    }

    /// Creates an ASCII string from a static byte slice at compile time.
    ///
    /// This is a `const fn` that validates the input and computes the hash
    /// at compile time. Invalid input (non-ASCII or too long) causes a
    /// compile-time panic.
    ///
    /// # Panics
    ///
    /// Panics at compile time if:
    /// - Any byte is > 127 (non-ASCII)
    /// - The slice is longer than `CAP`
    /// - `CAP > 128` (const hash limitation)
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiString;
    ///
    /// // Compile-time construction from bytes
    /// const SYMBOL: AsciiString<16> = AsciiString::from_static_bytes(b"BTC-USD");
    /// const WITH_CTRL: AsciiString<16> = AsciiString::from_static_bytes(&[0x01, b'A', b'B']);
    ///
    /// assert_eq!(SYMBOL.as_str(), "BTC-USD");
    /// assert_eq!(WITH_CTRL.len(), 3);
    /// ```
    #[inline]
    pub const fn from_static_bytes(bytes: &'static [u8]) -> Self {
        assert!(CAP <= 128, "from_static_bytes only supports CAP <= 128");

        let len = bytes.len();

        assert!(len <= CAP, "bytes exceed capacity");

        // Validate ASCII at compile time
        let mut i = 0;
        while i < len {
            assert!(bytes[i] <= 127, "bytes contain non-ASCII byte");
            i += 1;
        }

        // Compute hash at compile time
        let h = hash::hash_const::<CAP>(bytes);
        let header = pack_header(len as u16, h);

        // Copy bytes into data array
        let mut data = [0u8; CAP];
        let mut j = 0;
        while j < len {
            data[j] = bytes[j];
            j += 1;
        }

        Self { header, data }
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
        debug_assert!(bytes.iter().all(|&b| b <= 127), "bytes contain non-ASCII");

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

    /// Returns the character at the given index, or `None` if out of bounds.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiString, AsciiChar};
    ///
    /// let s: AsciiString<32> = AsciiString::try_from("hello")?;
    /// assert_eq!(s.get(0), Some(AsciiChar::h));
    /// assert_eq!(s.get(4), Some(AsciiChar::o));
    /// assert_eq!(s.get(5), None);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn get(&self, index: usize) -> Option<AsciiChar> {
        if index < self.len() {
            // SAFETY: index is within bounds and data contains valid ASCII
            Some(unsafe { AsciiChar::new_unchecked(self.data[index]) })
        } else {
            None
        }
    }

    /// Returns the character at the given index without bounds checking.
    ///
    /// # Safety
    ///
    /// The index must be less than `self.len()`.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiString, AsciiChar};
    ///
    /// let s: AsciiString<32> = AsciiString::try_from("hello")?;
    /// // SAFETY: 0 < 5
    /// let ch = unsafe { s.get_unchecked(0) };
    /// assert_eq!(ch, AsciiChar::h);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub unsafe fn get_unchecked(&self, index: usize) -> AsciiChar {
        debug_assert!(index < self.len());
        // SAFETY: caller guarantees index < len, data contains valid ASCII
        unsafe { AsciiChar::new_unchecked(*self.data.get_unchecked(index)) }
    }

    /// Returns an iterator over the characters in the string.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiString, AsciiChar};
    ///
    /// let s: AsciiString<32> = AsciiString::try_from("ABC")?;
    /// let chars: Vec<_> = s.chars().collect();
    /// assert_eq!(chars, vec![AsciiChar::A, AsciiChar::B, AsciiChar::C]);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn chars(&self) -> impl Iterator<Item = AsciiChar> + '_ {
        self.as_bytes().iter().map(|&b| {
            // SAFETY: all bytes in the string are valid ASCII
            unsafe { AsciiChar::new_unchecked(b) }
        })
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

    // =========================================================================
    // from_static tests
    // =========================================================================

    #[test]
    fn from_static_basic() {
        const S: AsciiString<32> = AsciiString::from_static("hello");
        assert_eq!(S.len(), 5);
        assert_eq!(S.as_str(), "hello");
    }

    #[test]
    fn from_static_empty() {
        const S: AsciiString<32> = AsciiString::from_static("");
        assert!(S.is_empty());
        assert_eq!(S.len(), 0);
    }

    #[test]
    fn from_static_full_capacity() {
        const S: AsciiString<8> = AsciiString::from_static("12345678");
        assert_eq!(S.len(), 8);
        assert_eq!(S.as_str(), "12345678");
    }

    #[test]
    fn from_static_matches_runtime() {
        // Verify const construction produces same result as runtime
        const CONST_S: AsciiString<32> = AsciiString::from_static("BTC-USD");
        let runtime_s: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();

        assert_eq!(CONST_S, runtime_s);
        assert_eq!(CONST_S.header(), runtime_s.header());
        assert_eq!(CONST_S.as_str(), runtime_s.as_str());
    }

    #[test]
    fn from_static_hash_matches_runtime() {
        // Critical: const hash must match runtime hash
        const CONST_S: AsciiString<32> = AsciiString::from_static("ETH-USDT");
        let runtime_s: AsciiString<32> = AsciiString::try_from("ETH-USDT").unwrap();

        // Headers must be identical (same length + same hash)
        assert_eq!(CONST_S.header(), runtime_s.header());
    }

    #[test]
    fn from_static_various_lengths() {
        // Test various lengths to cover different hash paths
        const L1: AsciiString<128> = AsciiString::from_static("a");
        const L3: AsciiString<128> = AsciiString::from_static("abc");
        const L4: AsciiString<128> = AsciiString::from_static("abcd");
        const L8: AsciiString<128> = AsciiString::from_static("abcdefgh");
        const L9: AsciiString<128> = AsciiString::from_static("abcdefghi");
        const L16: AsciiString<128> = AsciiString::from_static("abcdefghijklmnop");
        const L17: AsciiString<128> = AsciiString::from_static("abcdefghijklmnopq");
        const L32: AsciiString<128> = AsciiString::from_static("abcdefghijklmnopqrstuvwxyz012345");

        // Verify they match runtime
        assert_eq!(L1, AsciiString::try_from("a").unwrap());
        assert_eq!(L3, AsciiString::try_from("abc").unwrap());
        assert_eq!(L4, AsciiString::try_from("abcd").unwrap());
        assert_eq!(L8, AsciiString::try_from("abcdefgh").unwrap());
        assert_eq!(L9, AsciiString::try_from("abcdefghi").unwrap());
        assert_eq!(L16, AsciiString::try_from("abcdefghijklmnop").unwrap());
        assert_eq!(L17, AsciiString::try_from("abcdefghijklmnopq").unwrap());
        assert_eq!(
            L32,
            AsciiString::try_from("abcdefghijklmnopqrstuvwxyz012345").unwrap()
        );
    }

    #[test]
    fn from_static_in_hashmap() {
        use std::collections::HashMap;

        const KEY: AsciiString<16> = AsciiString::from_static("BTC-USD");

        let mut map: HashMap<AsciiString<16>, i32> = HashMap::new();
        map.insert(KEY, 100);

        // Lookup with runtime-constructed key
        let lookup: AsciiString<16> = AsciiString::try_from("BTC-USD").unwrap();
        assert_eq!(map.get(&lookup), Some(&100));

        // Lookup with the const key itself
        assert_eq!(map.get(&KEY), Some(&100));
    }

    #[test]
    fn from_static_equality_with_runtime() {
        const BTC: AsciiString<16> = AsciiString::from_static("BTC-USD");
        const ETH: AsciiString<16> = AsciiString::from_static("ETH-USD");

        let btc_runtime: AsciiString<16> = AsciiString::try_from("BTC-USD").unwrap();
        let eth_runtime: AsciiString<16> = AsciiString::try_from("ETH-USD").unwrap();

        // Const == Runtime
        assert_eq!(BTC, btc_runtime);
        assert_eq!(ETH, eth_runtime);

        // Const != Different Runtime
        assert_ne!(BTC, eth_runtime);
        assert_ne!(ETH, btc_runtime);

        // Const != Const
        assert_ne!(BTC, ETH);
    }

    #[test]
    fn from_static_with_symbols() {
        const S: AsciiString<64> = AsciiString::from_static("!@#$%^&*()_+-=[]{}|;':\",./<>?");
        assert_eq!(S.as_str(), "!@#$%^&*()_+-=[]{}|;':\",./<>?");
    }

    #[test]
    fn from_static_with_digits() {
        const S: AsciiString<32> = AsciiString::from_static("0123456789");
        assert_eq!(S.as_str(), "0123456789");
    }

    #[test]
    fn from_static_realistic_identifiers() {
        const ORDER_ID: AsciiString<64> = AsciiString::from_static("ORD-2024-01-20-001-ABC123");
        const SYMBOL: AsciiString<16> = AsciiString::from_static("BTCUSDT");
        const EXCHANGE: AsciiString<16> = AsciiString::from_static("BINANCE");

        assert_eq!(ORDER_ID.as_str(), "ORD-2024-01-20-001-ABC123");
        assert_eq!(SYMBOL.as_str(), "BTCUSDT");
        assert_eq!(EXCHANGE.as_str(), "BINANCE");

        // Verify they work in lookups
        let runtime_symbol: AsciiString<16> = AsciiString::try_from("BTCUSDT").unwrap();
        assert_eq!(SYMBOL, runtime_symbol);
    }

    // =========================================================================
    // from_static_bytes tests
    // =========================================================================

    #[test]
    fn from_static_bytes_basic() {
        const S: AsciiString<32> = AsciiString::from_static_bytes(b"hello");
        assert_eq!(S.len(), 5);
        assert_eq!(S.as_str(), "hello");
    }

    #[test]
    fn from_static_bytes_empty() {
        const S: AsciiString<32> = AsciiString::from_static_bytes(b"");
        assert!(S.is_empty());
        assert_eq!(S.len(), 0);
    }

    #[test]
    fn from_static_bytes_with_control_chars() {
        // This is a key use case - control characters that can't be in str literals easily
        const S: AsciiString<16> = AsciiString::from_static_bytes(&[0x01, 0x02, b'A', b'B']);
        assert_eq!(S.len(), 4);
        assert_eq!(S.as_bytes(), &[0x01, 0x02, b'A', b'B']);
    }

    #[test]
    fn from_static_bytes_fix_delimiter() {
        // FIX protocol uses SOH (0x01) as delimiter
        const FIX_FIELD: AsciiString<32> =
            AsciiString::from_static_bytes(b"8=FIX.4.4\x019=123\x01");
        assert_eq!(FIX_FIELD.len(), 16);
        assert_eq!(FIX_FIELD.as_bytes()[9], 0x01); // SOH delimiter
    }

    #[test]
    fn from_static_bytes_matches_from_static_str() {
        // When content is the same, both should produce identical results
        const FROM_STR: AsciiString<32> = AsciiString::from_static("BTC-USD");
        const FROM_BYTES: AsciiString<32> = AsciiString::from_static_bytes(b"BTC-USD");

        assert_eq!(FROM_STR, FROM_BYTES);
        assert_eq!(FROM_STR.header(), FROM_BYTES.header());
    }

    #[test]
    fn from_static_bytes_matches_runtime() {
        const CONST_S: AsciiString<32> = AsciiString::from_static_bytes(b"ETH-USDT");
        let runtime_s: AsciiString<32> = AsciiString::try_from_bytes(b"ETH-USDT").unwrap();

        assert_eq!(CONST_S, runtime_s);
        assert_eq!(CONST_S.header(), runtime_s.header());
    }

    #[test]
    fn from_static_bytes_various_lengths() {
        const L1: AsciiString<128> = AsciiString::from_static_bytes(b"a");
        const L8: AsciiString<128> = AsciiString::from_static_bytes(b"abcdefgh");
        const L16: AsciiString<128> = AsciiString::from_static_bytes(b"abcdefghijklmnop");
        const L32: AsciiString<128> =
            AsciiString::from_static_bytes(b"abcdefghijklmnopqrstuvwxyz012345");

        assert_eq!(L1, AsciiString::try_from_bytes(b"a").unwrap());
        assert_eq!(L8, AsciiString::try_from_bytes(b"abcdefgh").unwrap());
        assert_eq!(
            L16,
            AsciiString::try_from_bytes(b"abcdefghijklmnop").unwrap()
        );
        assert_eq!(
            L32,
            AsciiString::try_from_bytes(b"abcdefghijklmnopqrstuvwxyz012345").unwrap()
        );
    }

    #[test]
    fn from_static_bytes_in_hashmap() {
        use std::collections::HashMap;

        const KEY: AsciiString<16> = AsciiString::from_static_bytes(b"BTC-USD");

        let mut map: HashMap<AsciiString<16>, i32> = HashMap::new();
        map.insert(KEY, 100);

        // Lookup with runtime-constructed key
        let lookup: AsciiString<16> = AsciiString::try_from_bytes(b"BTC-USD").unwrap();
        assert_eq!(map.get(&lookup), Some(&100));

        // Lookup with str-constructed key (should also match)
        let lookup_str: AsciiString<16> = AsciiString::try_from("BTC-USD").unwrap();
        assert_eq!(map.get(&lookup_str), Some(&100));
    }

    #[test]
    fn from_static_bytes_all_ascii_values() {
        // Test with bytes spanning the full ASCII range
        const LOW: AsciiString<32> = AsciiString::from_static_bytes(&[0x00, 0x01, 0x02, 0x03]);
        const HIGH: AsciiString<32> = AsciiString::from_static_bytes(&[0x7C, 0x7D, 0x7E, 0x7F]);

        assert_eq!(LOW.len(), 4);
        assert_eq!(HIGH.len(), 4);
        assert_eq!(HIGH.as_bytes()[3], 0x7F); // DEL character
    }

    // =========================================================================
    // Character access tests
    // =========================================================================

    #[test]
    fn get_valid_index() {
        let s: AsciiString<32> = AsciiString::try_from("hello").unwrap();
        assert_eq!(s.get(0), Some(AsciiChar::h));
        assert_eq!(s.get(1), Some(AsciiChar::e));
        assert_eq!(s.get(2), Some(AsciiChar::l));
        assert_eq!(s.get(3), Some(AsciiChar::l));
        assert_eq!(s.get(4), Some(AsciiChar::o));
    }

    #[test]
    fn get_out_of_bounds() {
        let s: AsciiString<32> = AsciiString::try_from("hello").unwrap();
        assert_eq!(s.get(5), None);
        assert_eq!(s.get(100), None);
    }

    #[test]
    fn get_empty_string() {
        let s: AsciiString<32> = AsciiString::empty();
        assert_eq!(s.get(0), None);
    }

    #[test]
    fn get_unchecked_valid() {
        let s: AsciiString<32> = AsciiString::try_from("ABC").unwrap();
        unsafe {
            assert_eq!(s.get_unchecked(0), AsciiChar::A);
            assert_eq!(s.get_unchecked(1), AsciiChar::B);
            assert_eq!(s.get_unchecked(2), AsciiChar::C);
        }
    }

    #[test]
    fn chars_iterator() {
        let s: AsciiString<32> = AsciiString::try_from("ABC").unwrap();
        let chars: Vec<_> = s.chars().collect();
        assert_eq!(chars, vec![AsciiChar::A, AsciiChar::B, AsciiChar::C]);
    }

    #[test]
    fn chars_empty() {
        let s: AsciiString<32> = AsciiString::empty();
        assert_eq!(s.chars().count(), 0);
    }

    #[test]
    fn chars_with_digits() {
        let s: AsciiString<32> = AsciiString::try_from("a1b2").unwrap();
        let chars: Vec<_> = s.chars().collect();
        assert_eq!(
            chars,
            vec![
                AsciiChar::a,
                AsciiChar::DIGIT_1,
                AsciiChar::b,
                AsciiChar::DIGIT_2
            ]
        );
    }

    #[test]
    fn chars_iterate_and_transform() {
        let s: AsciiString<32> = AsciiString::try_from("abc").unwrap();
        let upper: Vec<_> = s.chars().map(|c| c.to_uppercase()).collect();
        assert_eq!(upper, vec![AsciiChar::A, AsciiChar::B, AsciiChar::C]);
    }

    #[test]
    fn chars_count_alphabetic() {
        let s: AsciiString<32> = AsciiString::try_from("ab12cd").unwrap();
        let alpha_count = s.chars().filter(|c| c.is_alphabetic()).count();
        assert_eq!(alpha_count, 4);
    }
}
