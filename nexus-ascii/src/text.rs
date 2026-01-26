//! Printable-only ASCII string type.

use core::hash::{Hash, Hasher};

use crate::AsciiError;
use crate::char::AsciiChar;
use crate::simd;
use crate::str_ref::AsciiStr;
use crate::string::AsciiString;
use crate::text_ref::AsciiTextStr;

// =============================================================================
// Printable Validation (const version for compile-time)
// =============================================================================

/// Check if a byte is printable ASCII (0x20-0x7E).
#[inline(always)]
const fn is_printable(b: u8) -> bool {
    b >= 0x20 && b <= 0x7E
}

/// Const version of printable validation for compile-time checking.
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
// AsciiText
// =============================================================================

/// A fixed-capacity ASCII string containing only printable characters.
///
/// `AsciiText<CAP>` is a newtype over [`AsciiString<CAP>`] that guarantees all
/// characters are printable ASCII (0x20-0x7E, i.e., space through tilde).
/// This excludes control characters (0x00-0x1F) and DEL (0x7F).
///
/// Use `AsciiText` when you need to ensure strings contain only visible,
/// human-readable characters—for example, user-facing identifiers, display
/// names, or text that will be rendered in a UI.
///
/// # Design
///
/// - **Printable only**: Bytes must be in range 0x20-0x7E
/// - **Immutable**: Like `AsciiString`, immutable after creation
/// - **Copy**: Always implements `Copy`
/// - **Zero-cost conversion**: Converting to `AsciiString` is free
///
/// # Example
///
/// ```
/// use nexus_ascii::{AsciiText, AsciiError};
///
/// // Construction validates printable range
/// let text: AsciiText<32> = AsciiText::try_from("Hello, World!")?;
/// assert_eq!(text.as_str(), "Hello, World!");
///
/// // Control characters are rejected
/// let err = AsciiText::<32>::try_from_bytes(b"Hello\x00World");
/// assert!(matches!(err, Err(AsciiError::NonPrintable { .. })));
///
/// // Convert to AsciiString (zero-cost)
/// let s = text.into_ascii_string();
/// # Ok::<(), AsciiError>(())
/// ```
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct AsciiText<const CAP: usize>(AsciiString<CAP>);

// =============================================================================
// Constructors
// =============================================================================

impl<const CAP: usize> AsciiText<CAP> {
    /// Creates an empty printable ASCII text.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// let text: AsciiText<32> = AsciiText::empty();
    /// assert!(text.is_empty());
    /// ```
    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self(AsciiString::empty())
    }

    /// Creates a printable ASCII text from a static string literal at compile time.
    ///
    /// This is a `const fn` that validates the input at compile time.
    /// Invalid input (non-printable or too long) causes a compile-time panic.
    ///
    /// # Panics
    ///
    /// Panics at compile time if:
    /// - The string contains non-printable bytes (< 0x20 or > 0x7E)
    /// - The string is longer than `CAP`
    /// - `CAP > 128` (const hash limitation)
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// // Compile-time construction
    /// const HELLO: AsciiText<16> = AsciiText::from_static("Hello!");
    /// const SYMBOL: AsciiText<16> = AsciiText::from_static("BTC-USD");
    ///
    /// assert_eq!(HELLO.as_str(), "Hello!");
    /// ```
    #[inline]
    #[must_use]
    pub const fn from_static(s: &'static str) -> Self {
        let bytes = s.as_bytes();

        // Validate printable at compile time
        assert!(
            validate_printable_const(bytes),
            "string contains non-printable characters"
        );

        // Delegate to AsciiString::from_static which handles length and ASCII validation
        // Since printable ⊂ ASCII, this will succeed
        Self(AsciiString::from_static(s))
    }

    /// Creates a printable ASCII text from a static byte slice at compile time.
    ///
    /// Validates at compile time that all bytes are printable ASCII (0x20-0x7E).
    ///
    /// # Panics
    ///
    /// Panics at compile time if:
    /// - Any byte is not printable ASCII
    /// - The slice exceeds `CAP`
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// const SYMBOL: AsciiText<16> = AsciiText::from_static_bytes(b"BTC-USD");
    /// assert_eq!(SYMBOL.as_str(), "BTC-USD");
    /// ```
    #[inline]
    #[must_use]
    pub const fn from_static_bytes(bytes: &'static [u8]) -> Self {
        // Validate printable at compile time
        assert!(
            validate_printable_const(bytes),
            "bytes contain non-printable characters"
        );

        Self(AsciiString::from_static_bytes(bytes))
    }

    /// Attempts to create a printable ASCII text from a byte slice.
    ///
    /// # Errors
    ///
    /// - [`AsciiError::TooLong`] if the slice exceeds `CAP`
    /// - [`AsciiError::NonPrintable`] if any byte is < 0x20 or > 0x7E
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiText, AsciiError};
    ///
    /// let text: AsciiText<32> = AsciiText::try_from_bytes(b"Hello")?;
    /// assert_eq!(text.as_str(), "Hello");
    ///
    /// // Control characters rejected
    /// let err = AsciiText::<32>::try_from_bytes(b"\x00").unwrap_err();
    /// assert!(matches!(err, AsciiError::NonPrintable { byte: 0, pos: 0 }));
    ///
    /// // High ASCII rejected (also non-printable)
    /// let err = AsciiText::<32>::try_from_bytes(b"\x7F").unwrap_err();
    /// assert!(matches!(err, AsciiError::NonPrintable { byte: 0x7F, pos: 0 }));
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

        // Use bounded version since we know len <= CAP
        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(bytes) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }

        // SAFETY: Printable ASCII is a subset of ASCII, so from_bytes_unchecked is safe
        Ok(Self(unsafe { AsciiString::from_bytes_unchecked(bytes) }))
    }

    /// Attempts to create a printable ASCII text from a string slice.
    ///
    /// # Errors
    ///
    /// - [`AsciiError::TooLong`] if the string exceeds `CAP`
    /// - [`AsciiError::NonPrintable`] if any byte is < 0x20 or > 0x7E
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiText, AsciiError};
    ///
    /// let text: AsciiText<32> = AsciiText::try_from_str("Hello, World!")?;
    /// assert_eq!(text.as_str(), "Hello, World!");
    /// # Ok::<(), AsciiError>(())
    /// ```
    #[inline]
    pub fn try_from_str(s: &str) -> Result<Self, AsciiError> {
        Self::try_from_bytes(s.as_bytes())
    }

    /// Creates a printable ASCII text from a `&str` without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - All bytes are printable ASCII (0x20-0x7E)
    /// - The string length does not exceed `CAP`
    ///
    /// Violating these invariants causes undefined behavior.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// // SAFETY: "hello" is printable ASCII and fits in capacity
    /// let text: AsciiText<16> = unsafe { AsciiText::from_str_unchecked("hello") };
    /// assert_eq!(text.as_str(), "hello");
    /// ```
    #[inline]
    #[must_use]
    pub unsafe fn from_str_unchecked(s: &str) -> Self {
        // SAFETY: Caller guarantees printable ASCII and length
        unsafe { Self::from_bytes_unchecked(s.as_bytes()) }
    }

    /// Creates a printable ASCII text from a byte slice without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - All bytes are printable ASCII (0x20-0x7E)
    /// - The slice length does not exceed `CAP`
    ///
    /// Violating these invariants causes undefined behavior.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// // SAFETY: b"hello" is printable ASCII and fits in capacity
    /// let text: AsciiText<16> = unsafe { AsciiText::from_bytes_unchecked(b"hello") };
    /// assert_eq!(text.as_str(), "hello");
    /// ```
    #[inline]
    #[must_use]
    pub unsafe fn from_bytes_unchecked(bytes: &[u8]) -> Self {
        // SAFETY: Caller guarantees printable ASCII (which is a subset of ASCII)
        unsafe { Self(AsciiString::from_bytes_unchecked(bytes)) }
    }

    /// Creates a printable ASCII text from an [`AsciiString`] without validation.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that all bytes in the string are printable
    /// ASCII (0x20-0x7E). Violating this invariant may cause undefined behavior
    /// in code that relies on the printable guarantee.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiString, AsciiText};
    ///
    /// let s: AsciiString<32> = AsciiString::try_from("Hello").unwrap();
    /// // SAFETY: "Hello" contains only printable characters
    /// let text: AsciiText<32> = unsafe { AsciiText::from_ascii_string_unchecked(s) };
    /// assert_eq!(text.as_str(), "Hello");
    /// ```
    #[inline]
    #[must_use]
    pub const unsafe fn from_ascii_string_unchecked(s: AsciiString<CAP>) -> Self {
        Self(s)
    }

    /// Attempts to create a printable ASCII text from an [`AsciiString`].
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::NonPrintable`] if any byte is < 0x20 or > 0x7E.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiString, AsciiText, AsciiError};
    ///
    /// let s: AsciiString<32> = AsciiString::try_from("Hello").unwrap();
    /// let text: AsciiText<32> = AsciiText::try_from_ascii_string(s)?;
    /// assert_eq!(text.as_str(), "Hello");
    ///
    /// // Strings with control characters are rejected
    /// let s_ctrl: AsciiString<32> = AsciiString::try_from_bytes(b"Hello\x00").unwrap();
    /// let err = AsciiText::try_from_ascii_string(s_ctrl).unwrap_err();
    /// assert!(matches!(err, AsciiError::NonPrintable { .. }));
    /// # Ok::<(), AsciiError>(())
    /// ```
    #[inline]
    pub fn try_from_ascii_string(s: AsciiString<CAP>) -> Result<Self, AsciiError> {
        // Use bounded version since AsciiString<CAP> is bounded by CAP
        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(s.as_bytes()) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }
        Ok(Self(s))
    }

    /// Creates a printable ASCII text from a fixed-size raw buffer.
    ///
    /// The string length is determined by the position of the first null byte
    /// (0x00). If no null byte is found, the entire buffer is used.
    ///
    /// # Errors
    ///
    /// - [`AsciiError::NonPrintable`] if any byte before the first null is not
    ///   printable ASCII (< 0x20 or > 0x7E)
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// // Null-terminated buffer
    /// let buffer: [u8; 16] = *b"Hello\0\0\0\0\0\0\0\0\0\0\0";
    /// let text: AsciiText<16> = AsciiText::try_from_raw(buffer)?;
    /// assert_eq!(text.as_str(), "Hello");
    /// assert_eq!(text.len(), 5);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn try_from_raw(buffer: [u8; CAP]) -> Result<Self, AsciiError> {
        let s = AsciiString::try_from_raw(buffer)?;
        // Validate printable (AsciiString only validates ASCII, not printable)
        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(s.as_bytes()) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }
        Ok(Self(s))
    }

    /// Creates a printable ASCII text from a fixed-size raw buffer without validation.
    ///
    /// The string length is determined by the position of the first null byte
    /// (0x00). If no null byte is found, the entire buffer is used.
    ///
    /// # Safety
    ///
    /// The caller must ensure that all bytes before the first null byte are
    /// printable ASCII (0x20-0x7E). Violating this causes undefined behavior
    /// in code that assumes the printable guarantee.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// let buffer: [u8; 16] = *b"Hello\0\0\0\0\0\0\0\0\0\0\0";
    /// // SAFETY: "Hello" contains only printable characters
    /// let text: AsciiText<16> = unsafe { AsciiText::from_raw_unchecked(buffer) };
    /// assert_eq!(text.as_str(), "Hello");
    /// ```
    #[inline]
    #[must_use]
    pub unsafe fn from_raw_unchecked(buffer: [u8; CAP]) -> Self {
        // SAFETY: Caller guarantees printable ASCII
        unsafe { Self(AsciiString::from_raw_unchecked(buffer)) }
    }

    /// Creates a printable ASCII text from a null-terminated byte slice.
    ///
    /// Finds the first null byte (0x00) and uses content before it.
    /// If no null byte is found, uses the entire slice (up to `CAP`).
    ///
    /// This is useful when you have a reference to a fixed-size buffer
    /// (e.g., `&[u8; 40]`) and don't want to copy to an owned array.
    ///
    /// # Errors
    ///
    /// - [`AsciiError::NonPrintable`] if any byte before the null is not printable
    /// - [`AsciiError::TooLong`] if content length exceeds `CAP`
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// // Reference to a fixed-size buffer (like from a wire format)
    /// let buffer: &[u8; 16] = b"Hello\0\0\0\0\0\0\0\0\0\0\0";
    /// let text: AsciiText<16> = AsciiText::try_from_null_terminated(buffer)?;
    /// assert_eq!(text.as_str(), "Hello");
    /// assert_eq!(text.len(), 5);
    ///
    /// // Also works with regular slices
    /// let slice: &[u8] = b"World\0padding";
    /// let text: AsciiText<16> = AsciiText::try_from_null_terminated(slice)?;
    /// assert_eq!(text.as_str(), "World");
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn try_from_null_terminated(bytes: &[u8]) -> Result<Self, AsciiError> {
        let s = AsciiString::try_from_null_terminated(bytes)?;
        // Validate printable (AsciiString only validates ASCII, not printable)
        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(s.as_bytes()) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }
        Ok(Self(s))
    }

    /// Creates a printable ASCII text from a reference to a fixed-size buffer.
    ///
    /// Similar to [`try_from_null_terminated`](Self::try_from_null_terminated),
    /// but takes `&[u8; CAP]` instead of `&[u8]`. This allows the compiler to
    /// skip bounds checking since the buffer size matches the capacity.
    ///
    /// # Errors
    ///
    /// - [`AsciiError::NonPrintable`] if any byte before the null is not printable
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// let buffer: &[u8; 16] = b"Hello\0\0\0\0\0\0\0\0\0\0\0";
    /// let text: AsciiText<16> = AsciiText::try_from_ref(buffer)?;
    /// assert_eq!(text.as_str(), "Hello");
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn try_from_ref(buffer: &[u8; CAP]) -> Result<Self, AsciiError> {
        let s = AsciiString::try_from_ref(buffer)?;
        // Validate printable (AsciiString only validates ASCII, not printable)
        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(s.as_bytes()) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }
        Ok(Self(s))
    }

    /// Creates a printable ASCII text from a right-padded buffer.
    ///
    /// Strips trailing padding bytes from the end of the buffer.
    /// Common padding characters are space (0x20) or null (0x00).
    ///
    /// # Errors
    ///
    /// - [`AsciiError::NonPrintable`] if any content byte is not printable
    /// - [`AsciiError::InvalidByte`] if any content byte is not ASCII
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// // Space-padded buffer (common in FIX, SBE)
    /// let buffer: [u8; 16] = *b"Hello           ";
    /// let text: AsciiText<16> = AsciiText::try_from_right_padded(buffer, b' ')?;
    /// assert_eq!(text.as_str(), "Hello");
    /// assert_eq!(text.len(), 5);
    ///
    /// // Null-padded buffer
    /// let buffer: [u8; 16] = *b"World\0\0\0\0\0\0\0\0\0\0\0";
    /// let text: AsciiText<16> = AsciiText::try_from_right_padded(buffer, 0)?;
    /// assert_eq!(text.as_str(), "World");
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn try_from_right_padded(buffer: [u8; CAP], pad: u8) -> Result<Self, AsciiError> {
        let s = AsciiString::try_from_right_padded(buffer, pad)?;
        // Validate printable (AsciiString only validates ASCII, not printable)
        if let Err((byte, pos)) = simd::validate_printable_bounded::<CAP>(s.as_bytes()) {
            return Err(AsciiError::NonPrintable { byte, pos });
        }
        Ok(Self(s))
    }
}

// =============================================================================
// Conversion
// =============================================================================

impl<const CAP: usize> AsciiText<CAP> {
    /// Converts this printable text into an [`AsciiString`].
    ///
    /// This is a zero-cost conversion since `AsciiText` is a newtype wrapper.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiString, AsciiText};
    ///
    /// let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
    /// let s: AsciiString<32> = text.into_ascii_string();
    /// assert_eq!(s.as_str(), "Hello");
    /// ```
    #[inline]
    #[must_use]
    pub const fn into_ascii_string(self) -> AsciiString<CAP> {
        self.0
    }

    /// Returns a reference to the inner [`AsciiString`].
    #[inline]
    #[must_use]
    pub const fn as_ascii_string(&self) -> &AsciiString<CAP> {
        &self.0
    }

    /// Returns this text as a borrowed [`AsciiTextStr`].
    ///
    /// This is a zero-copy conversion that provides a DST view into the
    /// validated printable ASCII data.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiText, AsciiTextStr};
    ///
    /// let text: AsciiText<32> = AsciiText::try_from("Hello")?;
    /// let text_str: &AsciiTextStr = text.as_ascii_text_str();
    /// assert_eq!(text_str.len(), 5);
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    #[must_use]
    pub fn as_ascii_text_str(&self) -> &AsciiTextStr {
        // SAFETY: AsciiText guarantees all bytes are printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(self.0.as_bytes()) }
    }

    /// Returns the full fixed-size buffer.
    ///
    /// The first `self.len()` bytes contain the string content.
    /// Remaining bytes are zero-padded. Useful for wire formats that
    /// require fixed-size fields.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// let text: AsciiText<16> = AsciiText::try_from("Hello")?;
    /// let raw: [u8; 16] = text.into_raw();
    /// assert_eq!(&raw[..5], b"Hello");
    /// assert_eq!(&raw[5..], &[0u8; 11]); // zero-padded
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    #[must_use]
    pub const fn into_raw(self) -> [u8; CAP] {
        self.0.into_raw()
    }
}

// =============================================================================
// Capacity Conversion
// =============================================================================

impl<const CAP: usize> AsciiText<CAP> {
    /// Converts to a larger capacity `AsciiText`.
    ///
    /// The hash is preserved since it's computed from content, not capacity.
    /// This is a data copy, not a reference.
    ///
    /// # Compile-time Checks
    ///
    /// - `NEW_CAP >= CAP` (must be widening, not narrowing)
    /// - `NEW_CAP % 8 == 0` (alignment requirement)
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::AsciiText;
    ///
    /// let small: AsciiText<16> = AsciiText::try_from("hello")?;
    /// let large: AsciiText<32> = small.widen();
    /// assert_eq!(small.as_str(), large.as_str());
    /// # Ok::<(), nexus_ascii::AsciiError>(())
    /// ```
    #[inline]
    pub fn widen<const NEW_CAP: usize>(self) -> AsciiText<NEW_CAP> {
        // SAFETY: Printable guarantee is preserved since content is unchanged
        AsciiText(self.0.widen())
    }

    /// Converts to a smaller capacity `AsciiText`.
    ///
    /// Returns `Err(AsciiError::TooLong)` if the content doesn't fit.
    /// The hash is preserved since it's computed from content.
    ///
    /// # Compile-time Checks
    ///
    /// - `NEW_CAP <= CAP` (must be tightening, not widening)
    /// - `NEW_CAP % 8 == 0` (alignment requirement)
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{AsciiText, AsciiError};
    ///
    /// let large: AsciiText<32> = AsciiText::try_from("hello")?;
    /// let small: AsciiText<16> = large.tighten()?;
    /// assert_eq!(large.as_str(), small.as_str());
    ///
    /// // Content too long for target capacity
    /// let long: AsciiText<32> = AsciiText::try_from("this is a longer string")?;
    /// assert!(matches!(long.tighten::<16>(), Err(AsciiError::TooLong { .. })));
    /// # Ok::<(), AsciiError>(())
    /// ```
    #[inline]
    pub fn tighten<const NEW_CAP: usize>(self) -> Result<AsciiText<NEW_CAP>, crate::AsciiError> {
        // SAFETY: Printable guarantee is preserved since content is unchanged
        Ok(AsciiText(self.0.tighten()?))
    }
}

// =============================================================================
// Deref to AsciiString
// =============================================================================

impl<const CAP: usize> core::ops::Deref for AsciiText<CAP> {
    type Target = AsciiString<CAP>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl<const CAP: usize> Default for AsciiText<CAP> {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl<const CAP: usize> PartialEq for AsciiText<CAP> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<const CAP: usize> Eq for AsciiText<CAP> {}

impl<const CAP: usize> PartialOrd for AsciiText<CAP> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<const CAP: usize> Ord for AsciiText<CAP> {
    #[inline]
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl<const CAP: usize> Hash for AsciiText<CAP> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<const CAP: usize> core::fmt::Debug for AsciiText<CAP> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("AsciiText").field(&self.as_str()).finish()
    }
}

impl<const CAP: usize> core::fmt::Display for AsciiText<CAP> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

// =============================================================================
// Cross-type Equality
// =============================================================================

impl<const CAP: usize> PartialEq<AsciiString<CAP>> for AsciiText<CAP> {
    #[inline]
    fn eq(&self, other: &AsciiString<CAP>) -> bool {
        self.0 == *other
    }
}

impl<const CAP: usize> PartialEq<AsciiText<CAP>> for AsciiString<CAP> {
    #[inline]
    fn eq(&self, other: &AsciiText<CAP>) -> bool {
        *self == other.0
    }
}

impl<const CAP: usize> PartialEq<&str> for AsciiText<CAP> {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl<const CAP: usize> PartialEq<str> for AsciiText<CAP> {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl<const CAP: usize> PartialEq<&[u8]> for AsciiText<CAP> {
    #[inline]
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_bytes() == *other
    }
}

// =============================================================================
// TryFrom implementations
// =============================================================================

impl<const CAP: usize> TryFrom<&str> for AsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::try_from_str(s)
    }
}

impl<const CAP: usize> TryFrom<&[u8]> for AsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::try_from_bytes(bytes)
    }
}

#[cfg(feature = "std")]
impl<const CAP: usize> TryFrom<std::string::String> for AsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: std::string::String) -> Result<Self, Self::Error> {
        Self::try_from_str(&s)
    }
}

impl<const CAP: usize> TryFrom<AsciiString<CAP>> for AsciiText<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: AsciiString<CAP>) -> Result<Self, Self::Error> {
        Self::try_from_ascii_string(s)
    }
}

// =============================================================================
// AsRef implementations
// =============================================================================

impl<const CAP: usize> AsRef<str> for AsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const CAP: usize> AsRef<[u8]> for AsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<const CAP: usize> AsRef<AsciiStr> for AsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &AsciiStr {
        self.as_ascii_str()
    }
}

impl<const CAP: usize> AsRef<AsciiString<CAP>> for AsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &AsciiString<CAP> {
        &self.0
    }
}

impl<const CAP: usize> AsRef<AsciiTextStr> for AsciiText<CAP> {
    #[inline]
    fn as_ref(&self) -> &AsciiTextStr {
        self.as_ascii_text_str()
    }
}

// =============================================================================
// Borrow Implementation
// =============================================================================

impl<const CAP: usize> core::borrow::Borrow<AsciiTextStr> for AsciiText<CAP> {
    /// Borrows the text as an `&AsciiTextStr`.
    ///
    /// This enables using `AsciiText` as a key in `HashMap`/`HashSet` while
    /// looking up with `&AsciiTextStr`.
    #[inline]
    fn borrow(&self) -> &AsciiTextStr {
        self.as_ascii_text_str()
    }
}

// =============================================================================
// Index Implementations
// =============================================================================

impl<const CAP: usize> core::ops::Index<usize> for AsciiText<CAP> {
    type Output = AsciiChar;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::Range<usize>> for AsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, range: core::ops::Range<usize>) -> &Self::Output {
        assert!(range.start <= range.end, "range start > end");
        assert!(range.end <= self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, AsciiText guarantees printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(&self.as_bytes()[range]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeFrom<usize>> for AsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, range: core::ops::RangeFrom<usize>) -> &Self::Output {
        assert!(range.start <= self.len(), "range start out of bounds");
        // SAFETY: range is within bounds, AsciiText guarantees printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(&self.as_bytes()[range]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeTo<usize>> for AsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, range: core::ops::RangeTo<usize>) -> &Self::Output {
        assert!(range.end <= self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, AsciiText guarantees printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(&self.as_bytes()[range]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeFull> for AsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, _range: core::ops::RangeFull) -> &Self::Output {
        self.as_ascii_text_str()
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeInclusive<usize>> for AsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, range: core::ops::RangeInclusive<usize>) -> &Self::Output {
        let start = *range.start();
        let end = *range.end();
        assert!(start <= end, "range start > end");
        assert!(end < self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, AsciiText guarantees printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(&self.as_bytes()[start..=end]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeToInclusive<usize>> for AsciiText<CAP> {
    type Output = AsciiTextStr;

    #[inline]
    fn index(&self, range: core::ops::RangeToInclusive<usize>) -> &Self::Output {
        assert!(range.end < self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, AsciiText guarantees printable ASCII
        unsafe { AsciiTextStr::from_bytes_unchecked(&self.as_bytes()[range]) }
    }
}

// =============================================================================
// Serde Support (feature-gated)
// =============================================================================

#[cfg(feature = "serde")]
impl<const CAP: usize> serde::Serialize for AsciiText<CAP> {
    /// Serializes the ASCII text as a string.
    #[inline]
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de, const CAP: usize> serde::Deserialize<'de> for AsciiText<CAP> {
    /// Deserializes a string into an ASCII text.
    ///
    /// Returns an error if:
    /// - The string is longer than `CAP`
    /// - The string contains non-printable ASCII (< 0x20 or > 0x7E)
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AsciiTextVisitor<const CAP: usize>;

        impl<const CAP: usize> serde::de::Visitor<'_> for AsciiTextVisitor<CAP> {
            type Value = AsciiText<CAP>;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(
                    formatter,
                    "a printable ASCII string with at most {} bytes",
                    CAP
                )
            }

            #[inline]
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                AsciiText::try_from_str(v).map_err(|e| match e {
                    crate::AsciiError::TooLong { len, cap } => E::custom(format_args!(
                        "string length {} exceeds capacity {}",
                        len, cap
                    )),
                    crate::AsciiError::InvalidByte { byte, pos } => E::custom(format_args!(
                        "invalid ASCII byte 0x{:02X} at position {}",
                        byte, pos
                    )),
                    crate::AsciiError::NonPrintable { byte, pos } => E::custom(format_args!(
                        "non-printable byte 0x{:02X} at position {}",
                        byte, pos
                    )),
                })
            }

            #[inline]
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                AsciiText::try_from_bytes(v).map_err(|e| match e {
                    crate::AsciiError::TooLong { len, cap } => E::custom(format_args!(
                        "byte slice length {} exceeds capacity {}",
                        len, cap
                    )),
                    crate::AsciiError::InvalidByte { byte, pos } => E::custom(format_args!(
                        "invalid ASCII byte 0x{:02X} at position {}",
                        byte, pos
                    )),
                    crate::AsciiError::NonPrintable { byte, pos } => E::custom(format_args!(
                        "non-printable byte 0x{:02X} at position {}",
                        byte, pos
                    )),
                })
            }
        }

        deserializer.deserialize_str(AsciiTextVisitor)
    }
}

// =============================================================================
// Bytes Crate Support (feature-gated)
// =============================================================================

#[cfg(feature = "bytes")]
impl<const CAP: usize> From<AsciiText<CAP>> for bytes::Bytes {
    /// Converts an ASCII text into `Bytes`.
    #[inline]
    fn from(s: AsciiText<CAP>) -> Self {
        bytes::Bytes::copy_from_slice(s.as_bytes())
    }
}

#[cfg(feature = "bytes")]
impl<const CAP: usize> From<&AsciiText<CAP>> for bytes::Bytes {
    /// Converts a reference to an ASCII text into `Bytes`.
    #[inline]
    fn from(s: &AsciiText<CAP>) -> Self {
        bytes::Bytes::copy_from_slice(s.as_bytes())
    }
}

#[cfg(feature = "bytes")]
impl<const CAP: usize> TryFrom<bytes::Bytes> for AsciiText<CAP> {
    type Error = crate::AsciiError;

    /// Attempts to create an ASCII text from `Bytes`.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes exceed capacity or contain non-printable characters.
    #[inline]
    fn try_from(bytes: bytes::Bytes) -> Result<Self, Self::Error> {
        Self::try_from_bytes(&bytes)
    }
}

#[cfg(feature = "bytes")]
impl<const CAP: usize> TryFrom<&bytes::Bytes> for AsciiText<CAP> {
    type Error = crate::AsciiError;

    /// Attempts to create an ASCII text from a `Bytes` reference.
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
    fn test_empty() {
        let text: AsciiText<32> = AsciiText::empty();
        assert!(text.is_empty());
        assert_eq!(text.len(), 0);
    }

    #[test]
    fn test_from_static() {
        const HELLO: AsciiText<16> = AsciiText::from_static("Hello!");
        assert_eq!(HELLO.as_str(), "Hello!");
        assert_eq!(HELLO.len(), 6);
    }

    #[test]
    fn test_from_static_empty() {
        const EMPTY: AsciiText<16> = AsciiText::from_static("");
        assert!(EMPTY.is_empty());
    }

    #[test]
    fn test_from_static_with_space() {
        const SPACED: AsciiText<32> = AsciiText::from_static("Hello World");
        assert_eq!(SPACED.as_str(), "Hello World");
    }

    #[test]
    fn test_try_from_bytes_valid() {
        let text: AsciiText<32> = AsciiText::try_from_bytes(b"Hello, World!").unwrap();
        assert_eq!(text.as_str(), "Hello, World!");
    }

    #[test]
    fn test_try_from_bytes_with_space() {
        let text: AsciiText<32> = AsciiText::try_from_bytes(b" ").unwrap();
        assert_eq!(text.as_str(), " ");
    }

    #[test]
    fn test_try_from_bytes_with_tilde() {
        let text: AsciiText<32> = AsciiText::try_from_bytes(b"~").unwrap();
        assert_eq!(text.as_str(), "~");
    }

    #[test]
    fn test_try_from_bytes_null_rejected() {
        let err = AsciiText::<32>::try_from_bytes(b"\x00").unwrap_err();
        assert!(matches!(err, AsciiError::NonPrintable { byte: 0, pos: 0 }));
    }

    #[test]
    fn test_try_from_bytes_control_rejected() {
        let err = AsciiText::<32>::try_from_bytes(b"\x1F").unwrap_err();
        assert!(matches!(
            err,
            AsciiError::NonPrintable { byte: 0x1F, pos: 0 }
        ));
    }

    #[test]
    fn test_try_from_bytes_del_rejected() {
        let err = AsciiText::<32>::try_from_bytes(b"\x7F").unwrap_err();
        assert!(matches!(
            err,
            AsciiError::NonPrintable { byte: 0x7F, pos: 0 }
        ));
    }

    #[test]
    fn test_try_from_bytes_high_ascii_rejected() {
        let err = AsciiText::<32>::try_from_bytes(b"\x80").unwrap_err();
        assert!(matches!(
            err,
            AsciiError::NonPrintable { byte: 0x80, pos: 0 }
        ));
    }

    #[test]
    fn test_try_from_bytes_too_long() {
        let err = AsciiText::<8>::try_from_bytes(b"Hello Wor").unwrap_err();
        assert!(matches!(err, AsciiError::TooLong { len: 9, cap: 8 }));
    }

    #[test]
    fn test_try_from_bytes_control_in_middle() {
        let err = AsciiText::<32>::try_from_bytes(b"Hello\x00World").unwrap_err();
        assert!(matches!(err, AsciiError::NonPrintable { byte: 0, pos: 5 }));
    }

    #[test]
    fn test_try_from_str() {
        let text: AsciiText<32> = AsciiText::try_from_str("Hello").unwrap();
        assert_eq!(text.as_str(), "Hello");
    }

    #[test]
    fn test_try_from_ascii_string() {
        let s: AsciiString<32> = AsciiString::try_from("Hello").unwrap();
        let text: AsciiText<32> = AsciiText::try_from_ascii_string(s).unwrap();
        assert_eq!(text.as_str(), "Hello");
    }

    #[test]
    fn test_try_from_ascii_string_with_control() {
        let s: AsciiString<32> = AsciiString::try_from_bytes(b"Hello\x00").unwrap();
        let err = AsciiText::try_from_ascii_string(s).unwrap_err();
        assert!(matches!(err, AsciiError::NonPrintable { byte: 0, pos: 5 }));
    }

    #[test]
    fn test_from_ascii_string_unchecked() {
        let s: AsciiString<32> = AsciiString::try_from("Hello").unwrap();
        let text: AsciiText<32> = unsafe { AsciiText::from_ascii_string_unchecked(s) };
        assert_eq!(text.as_str(), "Hello");
    }

    #[test]
    fn test_into_ascii_string() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let s: AsciiString<32> = text.into_ascii_string();
        assert_eq!(s.as_str(), "Hello");
    }

    #[test]
    fn test_as_ascii_string() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let s: &AsciiString<32> = text.as_ascii_string();
        assert_eq!(s.as_str(), "Hello");
    }

    #[test]
    fn test_deref() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        // Access AsciiString methods via Deref
        assert_eq!(text.len(), 5);
        assert_eq!(text.as_str(), "Hello");
        assert_eq!(text.as_bytes(), b"Hello");
    }

    #[test]
    fn test_default() {
        let text: AsciiText<32> = AsciiText::default();
        assert!(text.is_empty());
    }

    #[test]
    fn test_equality() {
        let t1: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let t2: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let t3: AsciiText<32> = AsciiText::try_from("World").unwrap();

        assert_eq!(t1, t2);
        assert_ne!(t1, t3);
    }

    #[test]
    fn test_equality_with_ascii_string() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let s: AsciiString<32> = AsciiString::try_from("Hello").unwrap();

        assert_eq!(text, s);
        assert_eq!(s, text);
    }

    #[test]
    fn test_equality_with_str() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        assert_eq!(text, "Hello");
        assert!(text == "Hello");
    }

    #[test]
    fn test_equality_with_bytes() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        assert!(text == &b"Hello"[..]);
    }

    #[test]
    fn test_ordering() {
        let a: AsciiText<32> = AsciiText::try_from("AAA").unwrap();
        let b: AsciiText<32> = AsciiText::try_from("BBB").unwrap();
        let c: AsciiText<32> = AsciiText::try_from("AAA").unwrap();

        assert!(a < b);
        assert!(b > a);
        assert!(a == c);
        assert!(a <= c);
        assert!(a >= c);
    }

    #[test]
    fn test_hash() {
        use std::collections::HashMap;

        let text: AsciiText<32> = AsciiText::try_from("key").unwrap();
        let mut map = HashMap::new();
        map.insert(text, 42);

        assert_eq!(map.get(&text), Some(&42));
    }

    #[test]
    fn test_debug() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let debug = format!("{:?}", text);
        assert!(debug.contains("AsciiText"));
        assert!(debug.contains("Hello"));
    }

    #[test]
    fn test_display() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        assert_eq!(format!("{}", text), "Hello");
    }

    #[test]
    fn test_clone() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let cloned = text.clone();
        assert_eq!(text, cloned);
    }

    #[test]
    fn test_copy() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let copied = text; // Copy, not move
        assert_eq!(text.as_str(), "Hello");
        assert_eq!(copied.as_str(), "Hello");
    }

    #[test]
    fn test_try_from_trait_str() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        assert_eq!(text.as_str(), "Hello");
    }

    #[test]
    fn test_try_from_trait_bytes() {
        let text: AsciiText<32> = AsciiText::try_from(&b"Hello"[..]).unwrap();
        assert_eq!(text.as_str(), "Hello");
    }

    #[test]
    fn test_try_from_trait_string() {
        let text: AsciiText<32> = AsciiText::try_from(String::from("Hello")).unwrap();
        assert_eq!(text.as_str(), "Hello");
    }

    #[test]
    fn test_try_from_trait_ascii_string() {
        let s: AsciiString<32> = AsciiString::try_from("Hello").unwrap();
        let text: AsciiText<32> = AsciiText::try_from(s).unwrap();
        assert_eq!(text.as_str(), "Hello");
    }

    #[test]
    fn test_as_ref_str() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let s: &str = text.as_ref();
        assert_eq!(s, "Hello");
    }

    #[test]
    fn test_as_ref_bytes() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let bytes: &[u8] = text.as_ref();
        assert_eq!(bytes, b"Hello");
    }

    #[test]
    fn test_as_ref_ascii_str() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let s: &AsciiStr = text.as_ref();
        assert_eq!(s.as_str(), "Hello");
    }

    #[test]
    fn test_as_ref_ascii_string() {
        let text: AsciiText<32> = AsciiText::try_from("Hello").unwrap();
        let s: &AsciiString<32> = text.as_ref();
        assert_eq!(s.as_str(), "Hello");
    }

    #[test]
    fn test_printable_boundary_low() {
        // 0x1F is not printable, 0x20 (space) is
        let err = AsciiText::<32>::try_from_bytes(&[0x1F]).unwrap_err();
        assert!(matches!(err, AsciiError::NonPrintable { .. }));

        let text: AsciiText<32> = AsciiText::try_from_bytes(&[0x20]).unwrap();
        assert_eq!(text.as_str(), " ");
    }

    #[test]
    fn test_printable_boundary_high() {
        // 0x7E (~) is printable, 0x7F (DEL) is not
        let text: AsciiText<32> = AsciiText::try_from_bytes(&[0x7E]).unwrap();
        assert_eq!(text.as_str(), "~");

        let err = AsciiText::<32>::try_from_bytes(&[0x7F]).unwrap_err();
        assert!(matches!(err, AsciiError::NonPrintable { .. }));
    }

    #[test]
    fn test_all_printable_chars() {
        // Test that all printable characters (0x20-0x7E) are accepted
        let mut bytes = Vec::new();
        for b in 0x20u8..=0x7E {
            bytes.push(b);
        }
        let text: AsciiText<128> = AsciiText::try_from_bytes(&bytes).unwrap();
        assert_eq!(text.len(), 95); // 0x7E - 0x20 + 1 = 95 characters
    }

    // =========================================================================
    // Raw buffer tests
    // =========================================================================

    #[test]
    fn try_from_raw_basic() {
        let buffer: [u8; 16] = *b"Hello\0\0\0\0\0\0\0\0\0\0\0";
        let text: AsciiText<16> = AsciiText::try_from_raw(buffer).unwrap();
        assert_eq!(text.as_str(), "Hello");
        assert_eq!(text.len(), 5);
    }

    #[test]
    fn try_from_raw_full_buffer() {
        let buffer: [u8; 8] = *b"ABCDEFGH";
        let text: AsciiText<8> = AsciiText::try_from_raw(buffer).unwrap();
        assert_eq!(text.as_str(), "ABCDEFGH");
        assert_eq!(text.len(), 8);
    }

    #[test]
    fn try_from_raw_rejects_non_printable() {
        // Control character before null
        let buffer: [u8; 16] = *b"Hello\x01\0\0\0\0\0\0\0\0\0\0";
        let result = AsciiText::<16>::try_from_raw(buffer);
        assert!(matches!(result, Err(AsciiError::NonPrintable { .. })));
    }

    #[test]
    fn from_raw_unchecked_basic() {
        let buffer: [u8; 16] = *b"Hello\0\0\0\0\0\0\0\0\0\0\0";
        // SAFETY: "Hello" is printable ASCII
        let text: AsciiText<16> = unsafe { AsciiText::from_raw_unchecked(buffer) };
        assert_eq!(text.as_str(), "Hello");
    }

    #[test]
    fn into_raw_basic() {
        let text: AsciiText<16> = AsciiText::try_from("Hello").unwrap();
        let raw: [u8; 16] = text.into_raw();
        assert_eq!(&raw[..5], b"Hello");
        assert_eq!(&raw[5..], &[0u8; 11]);
    }

    #[test]
    fn into_raw_empty() {
        let text: AsciiText<16> = AsciiText::empty();
        let raw: [u8; 16] = text.into_raw();
        assert_eq!(raw, [0u8; 16]);
    }

    #[test]
    fn into_raw_roundtrip() {
        let original: AsciiText<16> = AsciiText::try_from("Test").unwrap();
        let raw: [u8; 16] = original.into_raw();
        let recovered: AsciiText<16> = AsciiText::try_from_raw(raw).unwrap();
        assert_eq!(original, recovered);
    }

    // =========================================================================
    // Capacity conversion tests
    // =========================================================================

    #[test]
    fn widen_basic() {
        let small: AsciiText<16> = AsciiText::try_from("hello").unwrap();
        let large: AsciiText<32> = small.widen();
        assert_eq!(small.as_str(), large.as_str());
        assert_eq!(small.len(), large.len());
    }

    #[test]
    fn widen_preserves_hash() {
        let small: AsciiText<16> = AsciiText::try_from("BTC-USD").unwrap();
        let large: AsciiText<64> = small.widen();
        // Header contains hash + len, both should be identical
        assert_eq!(small.header(), large.header());
    }

    #[test]
    fn widen_empty() {
        let small: AsciiText<8> = AsciiText::empty();
        let large: AsciiText<32> = small.widen();
        assert!(large.is_empty());
        assert_eq!(small.header(), large.header());
    }

    #[test]
    fn tighten_basic() {
        let large: AsciiText<32> = AsciiText::try_from("hello").unwrap();
        let small: AsciiText<16> = large.tighten().unwrap();
        assert_eq!(large.as_str(), small.as_str());
        assert_eq!(large.len(), small.len());
    }

    #[test]
    fn tighten_preserves_hash() {
        let large: AsciiText<64> = AsciiText::try_from("BTC-USD").unwrap();
        let small: AsciiText<16> = large.tighten().unwrap();
        // Header contains hash + len, both should be identical
        assert_eq!(large.header(), small.header());
    }

    #[test]
    fn tighten_empty() {
        let large: AsciiText<32> = AsciiText::empty();
        let small: AsciiText<8> = large.tighten().unwrap();
        assert!(small.is_empty());
        assert_eq!(large.header(), small.header());
    }

    #[test]
    fn tighten_too_long() {
        let large: AsciiText<32> = AsciiText::try_from("this is too long").unwrap();
        let result = large.tighten::<8>();
        assert!(matches!(
            result,
            Err(AsciiError::TooLong { len: 16, cap: 8 })
        ));
    }

    #[test]
    fn widen_tighten_roundtrip() {
        let original: AsciiText<16> = AsciiText::try_from("roundtrip").unwrap();
        let widened: AsciiText<64> = original.widen();
        let tightened: AsciiText<16> = widened.tighten().unwrap();
        assert_eq!(original, tightened);
        assert_eq!(original.header(), tightened.header());
    }

    // =========================================================================
    // from_static_bytes tests
    // =========================================================================

    #[test]
    fn from_static_bytes_basic() {
        const TEXT: AsciiText<16> = AsciiText::from_static_bytes(b"Hello");
        assert_eq!(TEXT.as_str(), "Hello");
    }

    // =========================================================================
    // from_str_unchecked tests
    // =========================================================================

    #[test]
    fn from_str_unchecked_basic() {
        let text: AsciiText<16> = unsafe { AsciiText::from_str_unchecked("hello") };
        assert_eq!(text.as_str(), "hello");
        assert_eq!(text.len(), 5);
    }

    #[test]
    fn from_str_unchecked_matches_checked() {
        let unchecked: AsciiText<16> = unsafe { AsciiText::from_str_unchecked("test123") };
        let checked: AsciiText<16> = AsciiText::try_from_str("test123").unwrap();
        assert_eq!(unchecked, checked);
        assert_eq!(unchecked.header(), checked.header());
    }

    // =========================================================================
    // from_bytes_unchecked tests
    // =========================================================================

    #[test]
    fn from_bytes_unchecked_basic() {
        let text: AsciiText<16> = unsafe { AsciiText::from_bytes_unchecked(b"hello") };
        assert_eq!(text.as_str(), "hello");
    }

    #[test]
    fn from_bytes_unchecked_matches_checked() {
        let unchecked: AsciiText<16> = unsafe { AsciiText::from_bytes_unchecked(b"test") };
        let checked: AsciiText<16> = AsciiText::try_from_bytes(b"test").unwrap();
        assert_eq!(unchecked, checked);
    }

    // =========================================================================
    // try_from_null_terminated tests
    // =========================================================================

    #[test]
    fn try_from_null_terminated_basic() {
        let buffer: &[u8; 16] = b"Hello\0\0\0\0\0\0\0\0\0\0\0";
        let text: AsciiText<16> = AsciiText::try_from_null_terminated(buffer).unwrap();
        assert_eq!(text.as_str(), "Hello");
        assert_eq!(text.len(), 5);
    }

    #[test]
    fn try_from_null_terminated_slice() {
        let slice: &[u8] = b"World\0garbage";
        let text: AsciiText<16> = AsciiText::try_from_null_terminated(slice).unwrap();
        assert_eq!(text.as_str(), "World");
    }

    #[test]
    fn try_from_null_terminated_no_null() {
        let buffer: &[u8] = b"Test";
        let text: AsciiText<16> = AsciiText::try_from_null_terminated(buffer).unwrap();
        assert_eq!(text.as_str(), "Test");
    }

    #[test]
    fn try_from_null_terminated_empty() {
        let buffer: &[u8] = b"\0garbage";
        let text: AsciiText<16> = AsciiText::try_from_null_terminated(buffer).unwrap();
        assert!(text.is_empty());
    }

    #[test]
    fn try_from_null_terminated_too_long() {
        let buffer: &[u8] = b"this is way too long";
        let result = AsciiText::<8>::try_from_null_terminated(buffer);
        assert!(matches!(result, Err(AsciiError::TooLong { .. })));
    }

    #[test]
    fn try_from_null_terminated_non_printable() {
        let buffer: &[u8] = b"hello\x01\0";
        let result = AsciiText::<16>::try_from_null_terminated(buffer);
        assert!(matches!(result, Err(AsciiError::NonPrintable { .. })));
    }

    #[test]
    fn try_from_null_terminated_roundtrip() {
        let original: AsciiText<16> = AsciiText::try_from("test").unwrap();
        let raw = original.into_raw();
        let recovered: AsciiText<16> = AsciiText::try_from_null_terminated(&raw).unwrap();
        assert_eq!(original, recovered);
    }

    // =========================================================================
    // try_from_ref tests
    // =========================================================================

    #[test]
    fn try_from_ref_basic() {
        let buffer: &[u8; 16] = b"Hello\0\0\0\0\0\0\0\0\0\0\0";
        let text: AsciiText<16> = AsciiText::try_from_ref(buffer).unwrap();
        assert_eq!(text.as_str(), "Hello");
        assert_eq!(text.len(), 5);
    }

    #[test]
    fn try_from_ref_no_null() {
        let buffer: &[u8; 8] = b"FullText";
        let text: AsciiText<8> = AsciiText::try_from_ref(buffer).unwrap();
        assert_eq!(text.as_str(), "FullText");
        assert_eq!(text.len(), 8);
    }

    #[test]
    fn try_from_ref_empty() {
        let buffer: &[u8; 16] = b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let text: AsciiText<16> = AsciiText::try_from_ref(buffer).unwrap();
        assert!(text.is_empty());
    }

    #[test]
    fn try_from_ref_non_printable() {
        let buffer: &[u8; 16] = b"hello\x01\0\0\0\0\0\0\0\0\0\0";
        let result = AsciiText::<16>::try_from_ref(buffer);
        assert!(matches!(result, Err(AsciiError::NonPrintable { .. })));
    }

    #[test]
    fn try_from_ref_matches_try_from_null_terminated() {
        let buffer: &[u8; 16] = b"Test123\0\0\0\0\0\0\0\0\0";
        let from_ref: AsciiText<16> = AsciiText::try_from_ref(buffer).unwrap();
        let from_slice: AsciiText<16> = AsciiText::try_from_null_terminated(buffer).unwrap();
        assert_eq!(from_ref, from_slice);
        assert_eq!(from_ref.header(), from_slice.header());
    }

    #[test]
    fn try_from_ref_roundtrip() {
        let original: AsciiText<16> = AsciiText::try_from("test").unwrap();
        let raw = original.into_raw();
        let recovered: AsciiText<16> = AsciiText::try_from_ref(&raw).unwrap();
        assert_eq!(original, recovered);
    }

    // =========================================================================
    // try_from_right_padded tests
    // =========================================================================

    #[test]
    fn try_from_right_padded_space() {
        let buffer: [u8; 16] = *b"Hello           ";
        let text: AsciiText<16> = AsciiText::try_from_right_padded(buffer, b' ').unwrap();
        assert_eq!(text.as_str(), "Hello");
        assert_eq!(text.len(), 5);
    }

    #[test]
    fn try_from_right_padded_null() {
        let buffer: [u8; 16] = *b"World\0\0\0\0\0\0\0\0\0\0\0";
        let text: AsciiText<16> = AsciiText::try_from_right_padded(buffer, 0).unwrap();
        assert_eq!(text.as_str(), "World");
    }

    #[test]
    fn try_from_right_padded_full() {
        let buffer: [u8; 8] = *b"FullText";
        let text: AsciiText<8> = AsciiText::try_from_right_padded(buffer, b' ').unwrap();
        assert_eq!(text.as_str(), "FullText");
        assert_eq!(text.len(), 8);
    }

    #[test]
    fn try_from_right_padded_all_padding() {
        let buffer: [u8; 8] = *b"        ";
        let text: AsciiText<8> = AsciiText::try_from_right_padded(buffer, b' ').unwrap();
        assert!(text.is_empty());
    }

    #[test]
    fn try_from_right_padded_non_printable() {
        let buffer: [u8; 16] = *b"hello\x01          ";
        let result = AsciiText::<16>::try_from_right_padded(buffer, b' ');
        assert!(matches!(result, Err(AsciiError::NonPrintable { .. })));
    }
}
