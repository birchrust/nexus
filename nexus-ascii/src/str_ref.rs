//! Borrowed ASCII string slice type.
//!
//! `AsciiStr` is to `AsciiString` what `str` is to `String` — a borrowed,
//! dynamically-sized view into validated ASCII bytes.

use core::hash::{Hash, Hasher};

use crate::char::AsciiChar;
use crate::AsciiError;

// =============================================================================
// AsciiStr
// =============================================================================

/// A borrowed slice of validated ASCII bytes.
///
/// `AsciiStr` is a dynamically-sized type (DST) that can only exist behind
/// a reference. It provides a zero-copy view into ASCII data without the
/// overhead of the precomputed hash that `AsciiString` carries.
///
/// # When to Use
///
/// - When you need to pass ASCII data without copying
/// - When you don't need the fast equality check (no precomputed hash)
/// - When working with substrings or slices of ASCII data
/// - As a common type for functions that accept both `&AsciiString` and `&AsciiStr`
///
/// # Example
///
/// ```
/// use nexus_ascii::{AsciiStr, AsciiString};
///
/// // From a validated string
/// let s: AsciiString<32> = AsciiString::try_from("hello")?;
/// let ascii_str: &AsciiStr = s.as_ascii_str();
///
/// // From bytes (with validation)
/// let from_bytes: &AsciiStr = AsciiStr::try_from_bytes(b"world")?;
///
/// assert_eq!(ascii_str.len(), 5);
/// assert_eq!(from_bytes.as_str(), "world");
/// # Ok::<(), nexus_ascii::AsciiError>(())
/// ```
#[repr(transparent)]
pub struct AsciiStr([u8]);

// =============================================================================
// Construction
// =============================================================================

impl AsciiStr {
    /// Creates an `&AsciiStr` from a byte slice after validating ASCII.
    ///
    /// Returns an error if any byte is > 127.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiStr, AsciiError};
    ///
    /// let valid: &AsciiStr = AsciiStr::try_from_bytes(b"hello")?;
    /// assert_eq!(valid.as_str(), "hello");
    ///
    /// let invalid = AsciiStr::try_from_bytes(&[0xFF]);
    /// assert!(matches!(invalid, Err(AsciiError::InvalidByte { .. })));
    /// # Ok::<(), AsciiError>(())
    /// ```
    #[inline]
    pub fn try_from_bytes(bytes: &[u8]) -> Result<&Self, AsciiError> {
        // Validate ASCII
        for (pos, &byte) in bytes.iter().enumerate() {
            if byte > 127 {
                return Err(AsciiError::InvalidByte { byte, pos });
            }
        }

        // SAFETY: We just validated all bytes are ASCII
        Ok(unsafe { Self::from_bytes_unchecked(bytes) })
    }

    /// Creates an `&AsciiStr` from a string slice after validating ASCII.
    ///
    /// Returns an error if any byte is > 127 (non-ASCII UTF-8).
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiStr, AsciiError};
    ///
    /// let valid: &AsciiStr = AsciiStr::try_from_str("hello")?;
    /// assert_eq!(valid.as_str(), "hello");
    ///
    /// let invalid = AsciiStr::try_from_str("héllo");
    /// assert!(matches!(invalid, Err(AsciiError::InvalidByte { .. })));
    /// # Ok::<(), AsciiError>(())
    /// ```
    #[inline]
    pub fn try_from_str(s: &str) -> Result<&Self, AsciiError> {
        Self::try_from_bytes(s.as_bytes())
    }

    /// Creates an `&AsciiStr` from a byte slice without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure all bytes are valid ASCII (0x00-0x7F).
    /// Violating this invariant causes undefined behavior in downstream
    /// code that assumes ASCII validity.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let bytes = b"HELLO";
    /// // SAFETY: bytes are known ASCII
    /// let s: &AsciiStr = unsafe { AsciiStr::from_bytes_unchecked(bytes) };
    /// assert_eq!(s.as_str(), "HELLO");
    /// ```
    #[inline]
    pub const unsafe fn from_bytes_unchecked(bytes: &[u8]) -> &Self {
        // SAFETY: AsciiStr is #[repr(transparent)] over [u8]
        unsafe { &*(bytes as *const [u8] as *const Self) }
    }

    /// Creates an `&AsciiStr` from a string slice without validation.
    ///
    /// This is useful when you have a `&str` that you know contains only
    /// ASCII characters (e.g., from a trusted source or after prior validation).
    ///
    /// # Safety
    ///
    /// The caller must ensure all characters are ASCII (code points 0-127).
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let s = "hello";
    /// // SAFETY: "hello" is known ASCII
    /// let ascii: &AsciiStr = unsafe { AsciiStr::from_str_unchecked(s) };
    /// assert_eq!(ascii.len(), 5);
    /// ```
    #[inline]
    pub const unsafe fn from_str_unchecked(s: &str) -> &Self {
        // SAFETY: Caller guarantees ASCII, str is valid UTF-8 which includes ASCII
        unsafe { Self::from_bytes_unchecked(s.as_bytes()) }
    }

    /// Returns an empty `&AsciiStr`.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let empty = AsciiStr::empty();
    /// assert!(empty.is_empty());
    /// assert_eq!(empty.len(), 0);
    /// ```
    #[inline]
    pub const fn empty() -> &'static Self {
        // SAFETY: Empty slice is trivially valid ASCII
        unsafe { Self::from_bytes_unchecked(&[]) }
    }
}

// =============================================================================
// Accessors
// =============================================================================

impl AsciiStr {
    /// Returns the length of the string in bytes.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let s = AsciiStr::try_from_bytes(b"hello")?;
    /// assert_eq!(s.len(), 5);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the string is empty.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let empty = AsciiStr::empty();
    /// assert!(empty.is_empty());
    ///
    /// let s = AsciiStr::try_from_bytes(b"x")?;
    /// assert!(!s.is_empty());
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the string as a byte slice.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let s = AsciiStr::try_from_bytes(b"hello")?;
    /// assert_eq!(s.as_bytes(), b"hello");
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub const fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the string as a `&str`.
    ///
    /// This is a zero-cost conversion since ASCII is valid UTF-8.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let s = AsciiStr::try_from_bytes(b"hello")?;
    /// assert_eq!(s.as_str(), "hello");
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub const fn as_str(&self) -> &str {
        // SAFETY: ASCII is always valid UTF-8
        unsafe { core::str::from_utf8_unchecked(&self.0) }
    }

    /// Returns the character at the given index, or `None` if out of bounds.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiStr, AsciiChar};
    ///
    /// let s = AsciiStr::try_from_bytes(b"hello")?;
    /// assert_eq!(s.get(0), Some(AsciiChar::h));
    /// assert_eq!(s.get(4), Some(AsciiChar::o));
    /// assert_eq!(s.get(5), None);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn get(&self, index: usize) -> Option<AsciiChar> {
        if index < self.len() {
            // SAFETY: index is within bounds and data contains valid ASCII
            Some(unsafe { AsciiChar::new_unchecked(self.0[index]) })
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
    /// use nexus_ascii::{AsciiStr, AsciiChar};
    ///
    /// let s = AsciiStr::try_from_bytes(b"hello")?;
    /// // SAFETY: 0 < 5
    /// let ch = unsafe { s.get_unchecked(0) };
    /// assert_eq!(ch, AsciiChar::h);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub unsafe fn get_unchecked(&self, index: usize) -> AsciiChar {
        debug_assert!(index < self.len());
        // SAFETY: caller guarantees index < len, data contains valid ASCII
        unsafe { AsciiChar::new_unchecked(*self.0.get_unchecked(index)) }
    }

    /// Returns the first character, or `None` if the string is empty.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiStr, AsciiChar};
    ///
    /// let s = AsciiStr::try_from_bytes(b"hello")?;
    /// assert_eq!(s.first(), Some(AsciiChar::h));
    ///
    /// let empty = AsciiStr::empty();
    /// assert_eq!(empty.first(), None);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn first(&self) -> Option<AsciiChar> {
        self.get(0)
    }

    /// Returns the last character, or `None` if the string is empty.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiStr, AsciiChar};
    ///
    /// let s = AsciiStr::try_from_bytes(b"hello")?;
    /// assert_eq!(s.last(), Some(AsciiChar::o));
    ///
    /// let empty = AsciiStr::empty();
    /// assert_eq!(empty.last(), None);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn last(&self) -> Option<AsciiChar> {
        if self.is_empty() {
            None
        } else {
            self.get(self.len() - 1)
        }
    }

    /// Returns an iterator over the characters in the string.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiStr, AsciiChar};
    ///
    /// let s = AsciiStr::try_from_bytes(b"ABC")?;
    /// let chars: Vec<_> = s.chars().collect();
    /// assert_eq!(chars, vec![AsciiChar::A, AsciiChar::B, AsciiChar::C]);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn chars(&self) -> impl Iterator<Item = AsciiChar> + '_ {
        self.0.iter().map(|&b| {
            // SAFETY: all bytes in the string are valid ASCII
            unsafe { AsciiChar::new_unchecked(b) }
        })
    }

    /// Returns an iterator over the bytes in the string.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let s = AsciiStr::try_from_bytes(b"ABC")?;
    /// let bytes: Vec<_> = s.bytes().collect();
    /// assert_eq!(bytes, vec![b'A', b'B', b'C']);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.0.iter().copied()
    }
}

// =============================================================================
// Comparison Methods
// =============================================================================

impl AsciiStr {
    /// Compares two ASCII strings for equality, ignoring ASCII case.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let s1 = AsciiStr::try_from_bytes(b"Hello")?;
    /// let s2 = AsciiStr::try_from_bytes(b"HELLO")?;
    /// assert!(s1.eq_ignore_ascii_case(s2));
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn eq_ignore_ascii_case(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        self.0
            .iter()
            .zip(other.0.iter())
            .all(|(&a, &b)| a.eq_ignore_ascii_case(&b))
    }

    /// Returns `true` if the string starts with the given prefix.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let s = AsciiStr::try_from_bytes(b"BTC-USD")?;
    /// assert!(s.starts_with(b"BTC"));
    /// assert!(s.starts_with("BTC-"));
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn starts_with<P: AsRef<[u8]>>(&self, prefix: P) -> bool {
        self.0.starts_with(prefix.as_ref())
    }

    /// Returns `true` if the string ends with the given suffix.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let s = AsciiStr::try_from_bytes(b"BTC-USD")?;
    /// assert!(s.ends_with(b"USD"));
    /// assert!(s.ends_with("-USD"));
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn ends_with<S: AsRef<[u8]>>(&self, suffix: S) -> bool {
        self.0.ends_with(suffix.as_ref())
    }

    /// Returns `true` if the string contains the given substring.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiStr;
    ///
    /// let s = AsciiStr::try_from_bytes(b"BTC-USD")?;
    /// assert!(s.contains(b"-"));
    /// assert!(s.contains("TC-US"));
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn contains<N: AsRef<[u8]>>(&self, needle: N) -> bool {
        let needle = needle.as_ref();
        if needle.is_empty() {
            return true;
        }
        self.0.windows(needle.len()).any(|window| window == needle)
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl PartialEq for AsciiStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for AsciiStr {}

impl PartialOrd for AsciiStr {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AsciiStr {
    #[inline]
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl Hash for AsciiStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl core::fmt::Debug for AsciiStr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AsciiStr")
            .field("value", &self.as_str())
            .field("len", &self.len())
            .finish()
    }
}

impl core::fmt::Display for AsciiStr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl core::ops::Index<usize> for AsciiStr {
    type Output = AsciiChar;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        assert!(index < self.len(), "index out of bounds");
        // SAFETY: index is within bounds, data contains valid ASCII.
        // AsciiChar is #[repr(transparent)] over u8.
        unsafe { &*(self.0.get_unchecked(index) as *const u8 as *const AsciiChar) }
    }
}

// =============================================================================
// Cross-type equality
// =============================================================================

impl PartialEq<str> for AsciiStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.0 == *other.as_bytes()
    }
}

impl PartialEq<AsciiStr> for str {
    #[inline]
    fn eq(&self, other: &AsciiStr) -> bool {
        *self.as_bytes() == other.0
    }
}

impl PartialEq<[u8]> for AsciiStr {
    #[inline]
    fn eq(&self, other: &[u8]) -> bool {
        self.0 == *other
    }
}

impl PartialEq<AsciiStr> for [u8] {
    #[inline]
    fn eq(&self, other: &AsciiStr) -> bool {
        *self == other.0
    }
}

// Reference versions
impl PartialEq<&str> for AsciiStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other.as_bytes()
    }
}

impl PartialEq<&[u8]> for AsciiStr {
    #[inline]
    fn eq(&self, other: &&[u8]) -> bool {
        self.0 == **other
    }
}

// =============================================================================
// AsRef implementations
// =============================================================================

impl AsRef<[u8]> for AsciiStr {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<str> for AsciiStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_from_bytes_valid() {
        let s = AsciiStr::try_from_bytes(b"hello").unwrap();
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn try_from_bytes_invalid() {
        let result = AsciiStr::try_from_bytes(&[0x80]);
        assert!(matches!(
            result,
            Err(AsciiError::InvalidByte { byte: 0x80, pos: 0 })
        ));
    }

    #[test]
    fn try_from_str_valid() {
        let s = AsciiStr::try_from_str("hello").unwrap();
        assert_eq!(s.as_str(), "hello");
    }

    #[test]
    fn try_from_str_invalid() {
        let result = AsciiStr::try_from_str("héllo");
        assert!(matches!(result, Err(AsciiError::InvalidByte { .. })));
    }

    #[test]
    fn empty() {
        let s = AsciiStr::empty();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s.as_str(), "");
    }

    #[test]
    fn from_bytes_unchecked() {
        let s = unsafe { AsciiStr::from_bytes_unchecked(b"test") };
        assert_eq!(s.as_str(), "test");
    }

    #[test]
    fn from_str_unchecked() {
        let s = unsafe { AsciiStr::from_str_unchecked("test") };
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn get_valid() {
        let s = AsciiStr::try_from_bytes(b"hello").unwrap();
        assert_eq!(s.get(0), Some(AsciiChar::h));
        assert_eq!(s.get(4), Some(AsciiChar::o));
    }

    #[test]
    fn get_out_of_bounds() {
        let s = AsciiStr::try_from_bytes(b"hello").unwrap();
        assert_eq!(s.get(5), None);
        assert_eq!(s.get(100), None);
    }

    #[test]
    fn get_unchecked_valid() {
        let s = AsciiStr::try_from_bytes(b"ABC").unwrap();
        unsafe {
            assert_eq!(s.get_unchecked(0), AsciiChar::A);
            assert_eq!(s.get_unchecked(2), AsciiChar::C);
        }
    }

    #[test]
    fn first_and_last() {
        let s = AsciiStr::try_from_bytes(b"hello").unwrap();
        assert_eq!(s.first(), Some(AsciiChar::h));
        assert_eq!(s.last(), Some(AsciiChar::o));

        let empty = AsciiStr::empty();
        assert_eq!(empty.first(), None);
        assert_eq!(empty.last(), None);
    }

    #[test]
    fn chars_iterator() {
        let s = AsciiStr::try_from_bytes(b"ABC").unwrap();
        let chars: Vec<_> = s.chars().collect();
        assert_eq!(chars, vec![AsciiChar::A, AsciiChar::B, AsciiChar::C]);
    }

    #[test]
    fn bytes_iterator() {
        let s = AsciiStr::try_from_bytes(b"ABC").unwrap();
        let bytes: Vec<_> = s.bytes().collect();
        assert_eq!(bytes, vec![b'A', b'B', b'C']);
    }

    #[test]
    fn equality() {
        let s1 = AsciiStr::try_from_bytes(b"hello").unwrap();
        let s2 = AsciiStr::try_from_bytes(b"hello").unwrap();
        let s3 = AsciiStr::try_from_bytes(b"world").unwrap();

        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
    }

    #[test]
    fn ordering() {
        let a = AsciiStr::try_from_bytes(b"abc").unwrap();
        let b = AsciiStr::try_from_bytes(b"abd").unwrap();
        assert!(a < b);
        assert!(b > a);
    }

    #[test]
    fn eq_ignore_ascii_case() {
        let s1 = AsciiStr::try_from_bytes(b"Hello").unwrap();
        let s2 = AsciiStr::try_from_bytes(b"HELLO").unwrap();
        let s3 = AsciiStr::try_from_bytes(b"world").unwrap();

        assert!(s1.eq_ignore_ascii_case(s2));
        assert!(!s1.eq_ignore_ascii_case(s3));
    }

    #[test]
    fn starts_with() {
        let s = AsciiStr::try_from_bytes(b"BTC-USD").unwrap();
        assert!(s.starts_with(b"BTC"));
        assert!(s.starts_with("BTC-"));
        assert!(!s.starts_with("ETH"));
    }

    #[test]
    fn ends_with() {
        let s = AsciiStr::try_from_bytes(b"BTC-USD").unwrap();
        assert!(s.ends_with(b"USD"));
        assert!(s.ends_with("-USD"));
        assert!(!s.ends_with("EUR"));
    }

    #[test]
    fn contains() {
        let s = AsciiStr::try_from_bytes(b"BTC-USD").unwrap();
        assert!(s.contains(b"-"));
        assert!(s.contains("TC-US"));
        assert!(!s.contains("ETH"));
    }

    #[test]
    fn cross_type_equality_str() {
        let s = AsciiStr::try_from_bytes(b"hello").unwrap();
        assert!(*s == *"hello");
        assert!(s == "hello");
        assert!(s != "world");
    }

    #[test]
    fn cross_type_equality_bytes() {
        let s = AsciiStr::try_from_bytes(b"hello").unwrap();
        assert!(*s == b"hello"[..]);
        assert!(b"hello"[..] == *s);
    }

    #[test]
    fn index() {
        let s = AsciiStr::try_from_bytes(b"hello").unwrap();
        assert_eq!(s[0], AsciiChar::h);
        assert_eq!(s[4], AsciiChar::o);
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn index_out_of_bounds() {
        let s = AsciiStr::try_from_bytes(b"hello").unwrap();
        let _ = s[5];
    }

    #[test]
    fn display() {
        let s = AsciiStr::try_from_bytes(b"hello").unwrap();
        assert_eq!(format!("{}", s), "hello");
    }

    #[test]
    fn debug() {
        let s = AsciiStr::try_from_bytes(b"hi").unwrap();
        let debug = format!("{:?}", s);
        assert!(debug.contains("AsciiStr"));
        assert!(debug.contains("hi"));
    }

    #[test]
    fn as_ref_str() {
        let s = AsciiStr::try_from_bytes(b"test").unwrap();
        let r: &str = s.as_ref();
        assert_eq!(r, "test");
    }

    #[test]
    fn as_ref_bytes() {
        let s = AsciiStr::try_from_bytes(b"test").unwrap();
        let r: &[u8] = s.as_ref();
        assert_eq!(r, b"test");
    }

    #[test]
    fn hash_works() {
        use std::collections::hash_map::DefaultHasher;

        let s1 = AsciiStr::try_from_bytes(b"test").unwrap();
        let s2 = AsciiStr::try_from_bytes(b"test").unwrap();

        let mut h1 = DefaultHasher::new();
        let mut h2 = DefaultHasher::new();
        s1.hash(&mut h1);
        s2.hash(&mut h2);

        assert_eq!(h1.finish(), h2.finish());
    }
}
