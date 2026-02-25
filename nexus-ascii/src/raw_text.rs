//! Printable-only raw ASCII string type.
//!
//! `RawAsciiText<CAP>` is a `#[repr(transparent)]` newtype over
//! [`RawAsciiString<CAP>`] that guarantees all content bytes are printable
//! ASCII (0x20-0x7E). No header, no hash.

use crate::AsciiError;
use crate::char::AsciiChar;
use crate::raw_string::RawAsciiString;
use crate::simd;
use crate::str_ref::AsciiStr;
use crate::text_ref::AsciiTextStr;

// =============================================================================
// Printable Validation (const version for compile-time)
// =============================================================================

#[inline(always)]
const fn is_printable(b: u8) -> bool {
    b >= 0x20 && b <= 0x7E
}

#[inline]
const fn validate_printable_const(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        if !is_printable(bytes[i]) {
            return false;
        }
        i += 1;
    }
    true
}

// =============================================================================
// RawAsciiText
// =============================================================================

/// A fixed-capacity raw ASCII buffer containing only printable characters.
///
/// `RawAsciiText<CAP>` is a `#[repr(transparent)]` newtype over
/// [`RawAsciiString<CAP>`] that guarantees all content characters are printable
/// ASCII (0x20-0x7E). No header, no hash — just raw printable bytes.
///
/// # Design
///
/// - **Printable only**: Content bytes must be in range 0x20-0x7E
/// - **No header**: Zero bytes of overhead
/// - **Copy**: Always implements `Copy`
/// - **Mutable**: `as_raw_mut()` is NOT inherited via Deref (Deref is `&`).
///   To mutate, convert to `RawAsciiString` first, then re-validate.
///
/// # Example
///
/// ```
/// use nexus_ascii::{RawAsciiText, AsciiError};
///
/// let text: RawAsciiText<32> = RawAsciiText::try_from("Hello, World!")?;
/// assert_eq!(text.as_str(), "Hello, World!");
///
/// // Control characters are rejected
/// let err = RawAsciiText::<32>::try_from_bytes(b"Hello\x01World");
/// assert!(matches!(err, Err(AsciiError::NonPrintable { .. })));
/// # Ok::<(), AsciiError>(())
/// ```
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct RawAsciiText<const CAP: usize>(RawAsciiString<CAP>);

// =============================================================================
// Constructors
// =============================================================================

impl<const CAP: usize> RawAsciiText<CAP> {
    /// Creates an empty printable raw ASCII text (all zeros).
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiText;
    ///
    /// let text: RawAsciiText<32> = RawAsciiText::empty();
    /// assert!(text.is_empty());
    /// ```
    #[inline]
    pub const fn empty() -> Self {
        Self(RawAsciiString::empty())
    }

    /// Creates a printable raw ASCII text from a static string literal at compile time.
    ///
    /// Validates printable range and rejects embedded null bytes at compile time.
    ///
    /// # Panics
    ///
    /// Panics at compile time if:
    /// - The string contains non-printable bytes (< 0x20 or > 0x7E)
    /// - The string is longer than `CAP`
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiText;
    ///
    /// const HELLO: RawAsciiText<16> = RawAsciiText::from_static("Hello!");
    /// assert_eq!(HELLO.as_str(), "Hello!");
    /// ```
    #[inline]
    pub const fn from_static(s: &'static str) -> Self {
        let bytes = s.as_bytes();
        assert!(
            validate_printable_const(bytes),
            "string contains non-printable byte"
        );
        // from_static validates ASCII + no nulls
        Self(RawAsciiString::from_static(s))
    }

    /// Creates a printable raw ASCII text from a static byte slice at compile time.
    ///
    /// # Panics
    ///
    /// Panics at compile time if:
    /// - Any byte is non-printable (< 0x20 or > 0x7E)
    /// - The slice is longer than `CAP`
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiText;
    ///
    /// const SYMBOL: RawAsciiText<16> = RawAsciiText::from_static_bytes(b"BTC-USD");
    /// assert_eq!(SYMBOL.as_str(), "BTC-USD");
    /// ```
    #[inline]
    pub const fn from_static_bytes(bytes: &'static [u8]) -> Self {
        assert!(
            validate_printable_const(bytes),
            "bytes contain non-printable byte"
        );
        // from_static_bytes validates ASCII + no nulls
        Self(RawAsciiString::from_static_bytes(bytes))
    }

    /// Creates a printable raw ASCII text from a byte slice.
    ///
    /// If the slice contains a null byte, only bytes before the first null are
    /// included. All content bytes must be printable (0x20-0x7E).
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::TooLong`] if the slice length exceeds `CAP`.
    /// Returns [`AsciiError::NonPrintable`] if any content byte is non-printable.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiText;
    ///
    /// let text: RawAsciiText<32> = RawAsciiText::try_from_bytes(b"Hello").unwrap();
    /// assert_eq!(text.as_str(), "Hello");
    /// ```
    #[inline]
    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, AsciiError> {
        if bytes.len() > CAP {
            return Err(AsciiError::TooLong {
                len: bytes.len(),
                cap: CAP,
            });
        }

        // Build the raw string first (handles null termination + ASCII validation)
        let raw = RawAsciiString::try_from_bytes(bytes)?;

        // Validate printable on content
        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(raw.as_bytes()) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }

        Ok(Self(raw))
    }

    /// Creates a printable raw ASCII text from a string slice.
    #[inline]
    pub fn try_from_str(s: &str) -> Result<Self, AsciiError> {
        Self::try_from_bytes(s.as_bytes())
    }

    /// Creates a printable raw ASCII text from bytes without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - All content bytes are printable ASCII (0x20-0x7E)
    /// - `bytes.len() <= CAP`
    #[inline]
    pub unsafe fn from_bytes_unchecked(bytes: &[u8]) -> Self {
        // SAFETY: caller guarantees printable ASCII
        Self(unsafe { RawAsciiString::from_bytes_unchecked(bytes) })
    }

    /// Creates a printable raw ASCII text from a string slice without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure all bytes are printable ASCII (0x20-0x7E)
    /// and `s.len() <= CAP`.
    #[inline]
    pub unsafe fn from_str_unchecked(s: &str) -> Self {
        // SAFETY: caller guarantees printable ASCII
        unsafe { Self::from_bytes_unchecked(s.as_bytes()) }
    }

    /// Creates a printable raw ASCII text from null-terminated bytes.
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::TooLong`] if there is no null within `CAP` bytes.
    /// Returns [`AsciiError::NonPrintable`] if any content byte is non-printable.
    #[inline]
    pub fn try_from_null_terminated(bytes: &[u8]) -> Result<Self, AsciiError> {
        let raw = RawAsciiString::try_from_null_terminated(bytes)?;

        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(raw.as_bytes()) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }

        Ok(Self(raw))
    }

    /// Creates a printable raw ASCII text from a raw buffer.
    ///
    /// Validates that all bytes before the first null are printable ASCII.
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::InvalidByte`] if any content byte is > 127.
    /// Returns [`AsciiError::NonPrintable`] if any content byte is non-printable.
    #[inline]
    pub fn try_from_raw(buffer: [u8; CAP]) -> Result<Self, AsciiError> {
        let raw = RawAsciiString::try_from_raw(buffer)?;

        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(raw.as_bytes()) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }

        Ok(Self(raw))
    }

    /// Creates a printable raw ASCII text from a borrowed raw buffer.
    #[inline]
    pub fn try_from_raw_ref(buffer: &[u8; CAP]) -> Result<Self, AsciiError> {
        Self::try_from_raw(*buffer)
    }

    /// Creates a printable raw ASCII text from a raw buffer without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure all bytes before the first null are printable ASCII (0x20-0x7E).
    #[inline]
    pub const unsafe fn from_raw_unchecked(buffer: [u8; CAP]) -> Self {
        // SAFETY: caller guarantees printable ASCII
        Self(unsafe { RawAsciiString::from_raw_unchecked(buffer) })
    }

    /// Creates a printable raw ASCII text from a right-padded buffer.
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::InvalidByte`] if any content byte is > 127.
    /// Returns [`AsciiError::NonPrintable`] if any content byte is non-printable.
    #[inline]
    pub fn try_from_right_padded(buffer: [u8; CAP], pad: u8) -> Result<Self, AsciiError> {
        let raw = RawAsciiString::try_from_right_padded(buffer, pad)?;

        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(raw.as_bytes()) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }

        Ok(Self(raw))
    }

    /// Creates a printable raw ASCII text from a `RawAsciiString`.
    ///
    /// Validates that all content bytes are printable (0x20-0x7E).
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::NonPrintable`] if any content byte is non-printable.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{RawAsciiString, RawAsciiText};
    ///
    /// let raw: RawAsciiString<32> = RawAsciiString::try_from("Hello").unwrap();
    /// let text: RawAsciiText<32> = RawAsciiText::try_from_raw_ascii_string(raw).unwrap();
    /// assert_eq!(text.as_str(), "Hello");
    /// ```
    #[inline]
    pub fn try_from_raw_ascii_string(s: RawAsciiString<CAP>) -> Result<Self, AsciiError> {
        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(s.as_bytes()) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }
        Ok(Self(s))
    }

    /// Creates a printable raw ASCII text from a `RawAsciiString` without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure all content bytes are printable ASCII (0x20-0x7E).
    #[inline]
    pub const unsafe fn from_raw_ascii_string_unchecked(s: RawAsciiString<CAP>) -> Self {
        Self(s)
    }
}

// =============================================================================
// Conversion
// =============================================================================

impl<const CAP: usize> RawAsciiText<CAP> {
    /// Returns the inner `RawAsciiString`.
    #[inline]
    pub const fn into_raw_ascii_string(self) -> RawAsciiString<CAP> {
        self.0
    }

    /// Returns a reference to the inner `RawAsciiString`.
    #[inline]
    pub const fn as_raw_ascii_string(&self) -> &RawAsciiString<CAP> {
        &self.0
    }

    /// Promotes this raw text to an `AsciiText` with precomputed hash.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{RawAsciiText, AsciiText};
    ///
    /// let raw: RawAsciiText<32> = RawAsciiText::try_from("Hello").unwrap();
    /// let hashed: AsciiText<32> = raw.to_ascii_text();
    /// assert_eq!(hashed.as_str(), "Hello");
    /// ```
    #[inline]
    pub fn to_ascii_text(self) -> crate::AsciiText<CAP> {
        let ascii_string = self.0.to_ascii_string();
        // SAFETY: content has already been validated as printable
        unsafe { crate::AsciiText::from_ascii_string_unchecked(ascii_string) }
    }

    /// Returns this text as a borrowed `&AsciiTextStr`.
    #[inline]
    pub fn as_ascii_text_str(&self) -> &AsciiTextStr {
        // SAFETY: content has been validated as printable
        unsafe { AsciiTextStr::from_bytes_unchecked(self.as_bytes()) }
    }

    /// Returns the string with the given prefix removed as `&AsciiTextStr`.
    #[inline]
    pub fn strip_prefix_text(&self, prefix: &[u8]) -> Option<&AsciiTextStr> {
        let bytes = self.as_bytes();
        if bytes.starts_with(prefix) {
            // SAFETY: bytes after prefix are valid printable ASCII
            Some(unsafe { AsciiTextStr::from_bytes_unchecked(&bytes[prefix.len()..]) })
        } else {
            None
        }
    }

    /// Returns the string with the given suffix removed as `&AsciiTextStr`.
    #[inline]
    pub fn strip_suffix_text(&self, suffix: &[u8]) -> Option<&AsciiTextStr> {
        let bytes = self.as_bytes();
        if bytes.ends_with(suffix) {
            // SAFETY: bytes before suffix are valid printable ASCII
            Some(unsafe {
                AsciiTextStr::from_bytes_unchecked(&bytes[..bytes.len() - suffix.len()])
            })
        } else {
            None
        }
    }

    /// Splits the string at the first occurrence of the delimiter,
    /// returning `&AsciiTextStr` slices.
    #[inline]
    pub fn split_once_text(&self, delimiter: u8) -> Option<(&AsciiTextStr, &AsciiTextStr)> {
        let bytes = self.as_bytes();
        let pos = bytes.iter().position(|&b| b == delimiter)?;
        // SAFETY: both halves are subsets of valid printable ASCII
        unsafe {
            let left = AsciiTextStr::from_bytes_unchecked(&bytes[..pos]);
            let right = AsciiTextStr::from_bytes_unchecked(&bytes[pos + 1..]);
            Some((left, right))
        }
    }
}

// =============================================================================
// Replacement Methods (printable-safe)
// =============================================================================

impl<const CAP: usize> RawAsciiText<CAP> {
    /// Returns a copy with all occurrences of `from` replaced with `to`.
    ///
    /// Validates that `to` is a printable ASCII character (0x20-0x7E).
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::NonPrintable`] if `to` is not printable.
    #[inline]
    pub fn replaced_char(self, from: AsciiChar, to: AsciiChar) -> Result<Self, AsciiError> {
        if !to.is_printable() {
            return Err(AsciiError::NonPrintable {
                byte: to.as_u8(),
                pos: 0,
            });
        }
        Ok(Self(self.0.replaced_char(from, to)))
    }

    /// Returns a copy with all occurrences of `from` replaced with `to`.
    ///
    /// # Safety
    ///
    /// The caller must ensure `to` is printable ASCII (0x20-0x7E).
    #[inline]
    pub unsafe fn replaced_char_unchecked(self, from: AsciiChar, to: AsciiChar) -> Self {
        Self(self.0.replaced_char(from, to))
    }

    /// Returns a copy with the first occurrence of `from` replaced with `to`.
    ///
    /// Validates that `to` is a printable ASCII character (0x20-0x7E).
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::NonPrintable`] if `to` is not printable.
    #[inline]
    pub fn replace_first_char(self, from: AsciiChar, to: AsciiChar) -> Result<Self, AsciiError> {
        if !to.is_printable() {
            return Err(AsciiError::NonPrintable {
                byte: to.as_u8(),
                pos: 0,
            });
        }
        Ok(Self(self.0.replace_first_char(from, to)))
    }

    /// Returns a copy with the first occurrence of `from` replaced with `to`.
    ///
    /// # Safety
    ///
    /// The caller must ensure `to` is printable ASCII (0x20-0x7E).
    #[inline]
    pub unsafe fn replace_first_char_unchecked(self, from: AsciiChar, to: AsciiChar) -> Self {
        Self(self.0.replace_first_char(from, to))
    }
}

// =============================================================================
// Integer Parsing
// =============================================================================

crate::parse::impl_parse_int_generic!(RawAsciiText, as_str);

// =============================================================================
// Integer Formatting
// =============================================================================

crate::format::impl_format_int_generic!(RawAsciiText, from_bytes_unchecked);

// =============================================================================
// Trait Implementations
// =============================================================================

impl<const CAP: usize> Default for RawAsciiText<CAP> {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl<const CAP: usize> core::fmt::Debug for RawAsciiText<CAP> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RawAsciiText")
            .field("value", &self.as_str())
            .field("len", &self.len())
            .field("cap", &CAP)
            .finish()
    }
}

impl<const CAP: usize> core::fmt::Display for RawAsciiText<CAP> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const CAP: usize> core::ops::Deref for RawAsciiText<CAP> {
    type Target = RawAsciiString<CAP>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const CAP: usize> core::ops::Index<usize> for RawAsciiText<CAP> {
    type Output = AsciiChar;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::Range<usize>> for RawAsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, range: core::ops::Range<usize>) -> &Self::Output {
        assert!(range.start <= range.end, "range start > end");
        assert!(range.end <= self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, data contains valid printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(&self.0.as_raw()[range]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeFrom<usize>> for RawAsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, range: core::ops::RangeFrom<usize>) -> &Self::Output {
        assert!(range.start <= self.len(), "range start out of bounds");
        // SAFETY: range is within bounds, data contains valid printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(&self.0.as_raw()[range.start..self.len()]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeTo<usize>> for RawAsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, range: core::ops::RangeTo<usize>) -> &Self::Output {
        assert!(range.end <= self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, data contains valid printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(&self.0.as_raw()[range]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeFull> for RawAsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, _range: core::ops::RangeFull) -> &Self::Output {
        self.as_ascii_text_str()
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeInclusive<usize>> for RawAsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, range: core::ops::RangeInclusive<usize>) -> &Self::Output {
        let start = *range.start();
        let end = *range.end();
        assert!(start <= end, "range start > end");
        assert!(end < self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, data contains valid printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(&self.0.as_raw()[start..=end]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeToInclusive<usize>> for RawAsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, range: core::ops::RangeToInclusive<usize>) -> &Self::Output {
        assert!(range.end < self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, data contains valid printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(&self.0.as_raw()[range]) }
    }
}

// =============================================================================
// TryFrom Implementations
// =============================================================================

impl<const CAP: usize> TryFrom<&str> for RawAsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::try_from_str(s)
    }
}

impl<const CAP: usize> TryFrom<&[u8]> for RawAsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::try_from_bytes(bytes)
    }
}

#[cfg(feature = "std")]
impl<const CAP: usize> TryFrom<std::string::String> for RawAsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: std::string::String) -> Result<Self, Self::Error> {
        Self::try_from_str(&s)
    }
}

#[cfg(feature = "std")]
impl<const CAP: usize> TryFrom<&std::string::String> for RawAsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: &std::string::String) -> Result<Self, Self::Error> {
        Self::try_from_str(s)
    }
}

impl<const CAP: usize> TryFrom<RawAsciiString<CAP>> for RawAsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: RawAsciiString<CAP>) -> Result<Self, Self::Error> {
        Self::try_from_raw_ascii_string(s)
    }
}

// =============================================================================
// AsRef Implementations
// =============================================================================

impl<const CAP: usize> AsRef<str> for RawAsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const CAP: usize> AsRef<[u8]> for RawAsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<const CAP: usize> AsRef<AsciiStr> for RawAsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &AsciiStr {
        self.as_ascii_str()
    }
}

impl<const CAP: usize> AsRef<[u8; CAP]> for RawAsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &[u8; CAP] {
        self.as_raw()
    }
}

impl<const CAP: usize> AsRef<AsciiTextStr> for RawAsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &AsciiTextStr {
        self.as_ascii_text_str()
    }
}

impl<const CAP: usize> AsRef<RawAsciiString<CAP>> for RawAsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &RawAsciiString<CAP> {
        &self.0
    }
}

// =============================================================================
// Serde Support (feature-gated)
// =============================================================================

#[cfg(feature = "serde")]
impl<const CAP: usize> serde::Serialize for RawAsciiText<CAP> {
    #[inline]
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de, const CAP: usize> serde::Deserialize<'de> for RawAsciiText<CAP> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RawAsciiTextVisitor<const CAP: usize>;

        impl<const CAP: usize> serde::de::Visitor<'_> for RawAsciiTextVisitor<CAP> {
            type Value = RawAsciiText<CAP>;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(
                    formatter,
                    "a printable ASCII string with at most {} bytes",
                    CAP
                )
            }

            #[inline]
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                RawAsciiText::try_from_str(v).map_err(|e| match e {
                    AsciiError::TooLong { len, cap } => E::custom(format_args!(
                        "string length {} exceeds capacity {}",
                        len, cap
                    )),
                    AsciiError::InvalidByte { byte, pos } => E::custom(format_args!(
                        "invalid ASCII byte 0x{:02X} at position {}",
                        byte, pos
                    )),
                    AsciiError::NonPrintable { byte, pos } => E::custom(format_args!(
                        "non-printable byte 0x{:02X} at position {}",
                        byte, pos
                    )),
                })
            }

            #[inline]
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                RawAsciiText::try_from_bytes(v).map_err(|e| match e {
                    AsciiError::TooLong { len, cap } => E::custom(format_args!(
                        "byte slice length {} exceeds capacity {}",
                        len, cap
                    )),
                    AsciiError::InvalidByte { byte, pos } => E::custom(format_args!(
                        "invalid ASCII byte 0x{:02X} at position {}",
                        byte, pos
                    )),
                    AsciiError::NonPrintable { byte, pos } => E::custom(format_args!(
                        "non-printable byte 0x{:02X} at position {}",
                        byte, pos
                    )),
                })
            }
        }

        deserializer.deserialize_str(RawAsciiTextVisitor)
    }
}

// =============================================================================
// Bytes Crate Support (feature-gated)
// =============================================================================

#[cfg(feature = "bytes")]
impl<const CAP: usize> From<RawAsciiText<CAP>> for bytes::Bytes {
    #[inline]
    fn from(s: RawAsciiText<CAP>) -> Self {
        bytes::Bytes::copy_from_slice(s.as_bytes())
    }
}

#[cfg(feature = "bytes")]
impl<const CAP: usize> From<&RawAsciiText<CAP>> for bytes::Bytes {
    #[inline]
    fn from(s: &RawAsciiText<CAP>) -> Self {
        bytes::Bytes::copy_from_slice(s.as_bytes())
    }
}

#[cfg(feature = "bytes")]
impl<const CAP: usize> TryFrom<bytes::Bytes> for RawAsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(bytes: bytes::Bytes) -> Result<Self, Self::Error> {
        Self::try_from_bytes(&bytes)
    }
}

#[cfg(feature = "bytes")]
impl<const CAP: usize> TryFrom<&bytes::Bytes> for RawAsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(bytes: &bytes::Bytes) -> Result<Self, Self::Error> {
        Self::try_from_bytes(bytes.as_ref())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text() {
        let t: RawAsciiText<32> = RawAsciiText::empty();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert_eq!(t.as_str(), "");
    }

    #[test]
    fn from_str_printable() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("Hello, World!").unwrap();
        assert_eq!(t.as_str(), "Hello, World!");
    }

    #[test]
    fn rejects_control_chars() {
        let result = RawAsciiText::<32>::try_from_bytes(b"Hello\x01World");
        assert!(matches!(result, Err(AsciiError::NonPrintable { .. })));
    }

    #[test]
    fn rejects_null_in_content() {
        // Null is a control character, but try_from_bytes on RawAsciiString
        // will truncate at null. So "Hello\x00World" becomes "Hello" which IS printable.
        let t: RawAsciiText<32> = RawAsciiText::try_from_bytes(b"Hello\x00World").unwrap();
        assert_eq!(t.as_str(), "Hello");
    }

    #[test]
    fn from_static_const() {
        const T: RawAsciiText<16> = RawAsciiText::from_static("BTC-USD");
        assert_eq!(T.as_str(), "BTC-USD");
    }

    #[test]
    fn from_static_bytes_const() {
        const T: RawAsciiText<16> = RawAsciiText::from_static_bytes(b"ETH-USD");
        assert_eq!(T.as_str(), "ETH-USD");
    }

    #[test]
    fn try_from_raw_ascii_string() {
        let raw: RawAsciiString<32> = RawAsciiString::try_from("Hello").unwrap();
        let text: RawAsciiText<32> = RawAsciiText::try_from_raw_ascii_string(raw).unwrap();
        assert_eq!(text.as_str(), "Hello");
    }

    #[test]
    fn try_from_raw_ascii_string_with_control() {
        let raw: RawAsciiString<32> = RawAsciiString::try_from_bytes(b"\x01Hello").unwrap();
        let result = RawAsciiText::try_from_raw_ascii_string(raw);
        assert!(matches!(result, Err(AsciiError::NonPrintable { .. })));
    }

    #[test]
    fn deref_to_raw_ascii_string() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("hello").unwrap();
        let raw: &RawAsciiString<32> = &*t;
        assert_eq!(raw.as_str(), "hello");
    }

    #[test]
    fn into_raw_ascii_string() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("hello").unwrap();
        let raw = t.into_raw_ascii_string();
        assert_eq!(raw.as_str(), "hello");
    }

    #[test]
    fn to_ascii_text_promotion() {
        let raw: RawAsciiText<32> = RawAsciiText::try_from("Hello").unwrap();
        let hashed = raw.to_ascii_text();
        assert_eq!(hashed.as_str(), "Hello");
    }

    #[test]
    fn index_range_returns_text_str() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("BTC-USD").unwrap();
        let slice: &AsciiTextStr = &t[0..3];
        assert_eq!(slice.as_str(), "BTC");
    }

    #[test]
    fn split_once_text() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("key=value").unwrap();
        let (k, v) = t.split_once_text(b'=').unwrap();
        assert_eq!(k.as_str(), "key");
        assert_eq!(v.as_str(), "value");
    }

    #[test]
    fn default_is_empty() {
        let t: RawAsciiText<32> = Default::default();
        assert!(t.is_empty());
    }

    #[test]
    fn display() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("hello").unwrap();
        assert_eq!(format!("{}", t), "hello");
    }

    #[test]
    fn debug() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("hi").unwrap();
        let debug = format!("{:?}", t);
        assert!(debug.contains("RawAsciiText"));
        assert!(debug.contains("hi"));
        assert!(debug.contains("32"));
    }

    #[test]
    fn try_from_raw_buffer() {
        let mut buf = [0u8; 32];
        buf[0] = b'A';
        buf[1] = b'B';
        let t: RawAsciiText<32> = RawAsciiText::try_from_raw(buf).unwrap();
        assert_eq!(t.as_str(), "AB");
    }

    #[test]
    fn try_from_right_padded() {
        let mut buf = [b' '; 32];
        buf[0] = b'H';
        buf[1] = b'i';
        let t: RawAsciiText<32> = RawAsciiText::try_from_right_padded(buf, b' ').unwrap();
        assert_eq!(t.as_str(), "Hi");
    }

    #[test]
    fn classification_via_deref() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("12345").unwrap();
        // These methods come from RawAsciiString via Deref
        assert!(t.is_numeric());
        assert!(t.is_alphanumeric());
    }

    #[test]
    fn try_from_raw_ascii_string_trait() {
        let raw: RawAsciiString<32> = RawAsciiString::try_from("Hello").unwrap();
        let text: RawAsciiText<32> = raw.try_into().unwrap();
        assert_eq!(text.as_str(), "Hello");
    }

    #[test]
    fn replaced_char_checked() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("a-b-c").unwrap();
        let minus = AsciiChar::try_new(b'-').unwrap();
        let underscore = AsciiChar::try_new(b'_').unwrap();
        let result = t.replaced_char(minus, underscore).unwrap();
        assert_eq!(result.as_str(), "a_b_c");
    }

    #[test]
    fn replaced_char_rejects_non_printable() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("hello").unwrap();
        let h = AsciiChar::try_new(b'h').unwrap();
        let ctrl = AsciiChar::try_new(0x01).unwrap();
        let result = t.replaced_char(h, ctrl);
        assert!(matches!(result, Err(AsciiError::NonPrintable { .. })));
    }

    #[test]
    fn replaced_char_unchecked() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("a-b-c").unwrap();
        let minus = AsciiChar::try_new(b'-').unwrap();
        let underscore = AsciiChar::try_new(b'_').unwrap();
        // SAFETY: underscore is printable
        let result = unsafe { t.replaced_char_unchecked(minus, underscore) };
        assert_eq!(result.as_str(), "a_b_c");
    }

    #[test]
    fn replace_first_char_checked() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("a-b-c").unwrap();
        let minus = AsciiChar::try_new(b'-').unwrap();
        let underscore = AsciiChar::try_new(b'_').unwrap();
        let result = t.replace_first_char(minus, underscore).unwrap();
        assert_eq!(result.as_str(), "a_b-c");
    }

    #[test]
    fn replace_first_char_rejects_non_printable() {
        let t: RawAsciiText<32> = RawAsciiText::try_from("hello").unwrap();
        let h = AsciiChar::try_new(b'h').unwrap();
        let ctrl = AsciiChar::try_new(0x01).unwrap();
        let result = t.replace_first_char(h, ctrl);
        assert!(matches!(result, Err(AsciiError::NonPrintable { .. })));
    }

    #[test]
    fn try_from_null_terminated_printable() {
        let t: RawAsciiText<16> =
            RawAsciiText::try_from_null_terminated(b"Hello\0").unwrap();
        assert_eq!(t.as_str(), "Hello");
    }

    #[test]
    fn try_from_null_terminated_rejects_control() {
        let result = RawAsciiText::<16>::try_from_null_terminated(b"Hi\x01\0");
        assert!(matches!(result, Err(AsciiError::NonPrintable { .. })));
    }

    #[test]
    fn try_from_null_terminated_full_buffer() {
        let t: RawAsciiText<8> =
            RawAsciiText::try_from_null_terminated(b"abcdefgh").unwrap();
        assert_eq!(t.len(), 8);
        assert_eq!(t.as_str(), "abcdefgh");
    }
}
