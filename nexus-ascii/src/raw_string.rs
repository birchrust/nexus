//! Fixed-capacity raw ASCII string type.
//!
//! `RawAsciiString<CAP>` is a null-terminated ASCII buffer with zero overhead.
//! No header, no hash — just raw bytes. Length is computed via SIMD `find_null_byte`.
//!
//! Designed for wire protocol fields (reject reasons, cancel text) where the hot
//! path is raw buffer I/O and hashing is never needed.

use crate::AsciiError;
use crate::char::AsciiChar;
use crate::simd;
use crate::str_ref::AsciiStr;
use crate::string::{copy_short, find_null_byte};

/// A fixed-capacity, null-terminated ASCII byte buffer.
///
/// `RawAsciiString<CAP>` stores up to `CAP` ASCII bytes inline with no header.
/// Length is determined at runtime by scanning for the first null byte. If no null
/// is found, the entire buffer is content.
///
/// # Design
///
/// - **No header**: Zero bytes of overhead. The buffer IS the data.
/// - **Copy**: Always implements `Copy`. For move semantics, wrap in a newtype.
/// - **Mutable**: `as_raw_mut()` gives direct buffer access for wire writes.
/// - **Full ASCII**: Accepts bytes 0x01-0x7F as content, 0x00 is the terminator.
///   For printable-only, use `RawAsciiText`.
///
/// # Null termination
///
/// - Content bytes: 0x01-0x7F (null is terminator, not content)
/// - If no null found in buffer, content = full `CAP` bytes
/// - `from_static` / `from_static_bytes` reject embedded nulls (compile-time bug)
/// - `try_from_bytes` copies input as-is; embedded nulls act as terminators for `len()`
/// - `as_raw_mut()` gives full buffer access; garbage after null is fine
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct RawAsciiString<const CAP: usize>(pub(crate) [u8; CAP]);

// =============================================================================
// Constructors
// =============================================================================

impl<const CAP: usize> RawAsciiString<CAP> {
    /// Compile-time assertion that CAP is a multiple of 8.
    const _CAP_ASSERT: () = assert!(CAP % 8 == 0, "RawAsciiString CAP must be a multiple of 8");

    /// Creates an empty raw ASCII string (all zeros).
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let s: RawAsciiString<32> = RawAsciiString::empty();
    /// assert!(s.is_empty());
    /// assert_eq!(s.len(), 0);
    /// ```
    #[inline]
    pub const fn empty() -> Self {
        let () = Self::_CAP_ASSERT;
        Self([0u8; CAP])
    }

    /// Creates a raw ASCII string from a static string literal at compile time.
    ///
    /// Validates ASCII and rejects embedded null bytes at compile time.
    /// No `CAP <= 128` restriction (no hash computation needed).
    ///
    /// # Panics
    ///
    /// Panics at compile time if:
    /// - The string contains non-ASCII bytes (> 127)
    /// - The string contains null bytes (0x00)
    /// - The string is longer than `CAP`
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// const BTC: RawAsciiString<16> = RawAsciiString::from_static("BTC-USD");
    /// assert_eq!(BTC.as_str(), "BTC-USD");
    /// ```
    #[inline]
    pub const fn from_static(s: &'static str) -> Self {
        let () = Self::_CAP_ASSERT;

        let bytes = s.as_bytes();
        let len = bytes.len();

        assert!(len <= CAP, "string exceeds capacity");

        // Validate ASCII and reject nulls at compile time
        let mut i = 0;
        while i < len {
            assert!(bytes[i] <= 127, "string contains non-ASCII byte");
            assert!(bytes[i] != 0, "string contains null byte");
            i += 1;
        }

        // Copy bytes into data array
        let mut data = [0u8; CAP];
        let mut j = 0;
        while j < len {
            data[j] = bytes[j];
            j += 1;
        }

        Self(data)
    }

    /// Creates a raw ASCII string from a static byte slice at compile time.
    ///
    /// Validates ASCII and rejects embedded null bytes at compile time.
    ///
    /// # Panics
    ///
    /// Panics at compile time if:
    /// - Any byte is > 127 (non-ASCII)
    /// - Any byte is 0x00 (null)
    /// - The slice is longer than `CAP`
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// const SYMBOL: RawAsciiString<16> = RawAsciiString::from_static_bytes(b"BTC-USD");
    /// assert_eq!(SYMBOL.as_str(), "BTC-USD");
    /// ```
    #[inline]
    pub const fn from_static_bytes(bytes: &'static [u8]) -> Self {
        let () = Self::_CAP_ASSERT;

        let len = bytes.len();

        assert!(len <= CAP, "bytes exceed capacity");

        // Validate ASCII and reject nulls at compile time
        let mut i = 0;
        while i < len {
            assert!(bytes[i] <= 127, "bytes contain non-ASCII byte");
            assert!(bytes[i] != 0, "bytes contain null byte");
            i += 1;
        }

        // Copy bytes into data array
        let mut data = [0u8; CAP];
        let mut j = 0;
        while j < len {
            data[j] = bytes[j];
            j += 1;
        }

        Self(data)
    }

    /// Creates a raw ASCII string from a byte slice.
    ///
    /// Validates that all bytes are ASCII (0x00-0x7F). If the slice contains
    /// a null byte, only bytes before the first null are included.
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::TooLong`] if the slice length exceeds `CAP`.
    /// Returns [`AsciiError::InvalidByte`] if any byte before the first null is > 127.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let s: RawAsciiString<32> = RawAsciiString::try_from_bytes(b"hello").unwrap();
    /// assert_eq!(s.as_str(), "hello");
    ///
    /// // Null terminates early
    /// let s: RawAsciiString<32> = RawAsciiString::try_from_bytes(b"hi\x00world").unwrap();
    /// assert_eq!(s.as_str(), "hi");
    /// ```
    #[inline]
    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, AsciiError> {
        let () = Self::_CAP_ASSERT;

        if bytes.len() > CAP {
            return Err(AsciiError::TooLong {
                len: bytes.len(),
                cap: CAP,
            });
        }

        // Validate ASCII using SIMD (no null scan — input is a known-length slice)
        if let Err((byte, pos)) = simd::validate_ascii_bounded::<CAP>(bytes) {
            return Err(AsciiError::InvalidByte { byte, pos });
        }

        let mut data = [0u8; CAP];
        // SAFETY: bytes.len() <= CAP, buffers don't overlap
        unsafe { copy_short(data.as_mut_ptr(), bytes.as_ptr(), bytes.len()) };

        Ok(Self(data))
    }

    /// Creates a raw ASCII string from a string slice.
    ///
    /// Delegates to [`try_from_bytes`](Self::try_from_bytes).
    #[inline]
    pub fn try_from_str(s: &str) -> Result<Self, AsciiError> {
        Self::try_from_bytes(s.as_bytes())
    }

    /// Creates a raw ASCII string from bytes without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - All bytes are valid ASCII (0x00-0x7F)
    /// - `bytes.len() <= CAP`
    #[inline]
    pub unsafe fn from_bytes_unchecked(bytes: &[u8]) -> Self {
        let () = Self::_CAP_ASSERT;
        debug_assert!(bytes.len() <= CAP);

        let mut data = [0u8; CAP];
        // SAFETY: caller guarantees len <= CAP
        unsafe { copy_short(data.as_mut_ptr(), bytes.as_ptr(), bytes.len()) };

        Self(data)
    }

    /// Creates a raw ASCII string from a string slice without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - All bytes are valid ASCII (0x00-0x7F)
    /// - `s.len() <= CAP`
    #[inline]
    pub unsafe fn from_str_unchecked(s: &str) -> Self {
        // SAFETY: caller guarantees valid ASCII and length
        unsafe { Self::from_bytes_unchecked(s.as_bytes()) }
    }

    /// Creates a raw ASCII string from null-terminated bytes.
    ///
    /// Scans for the first null byte. Content before the null is validated
    /// as ASCII. The null byte and everything after it are discarded.
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::TooLong`] if there is no null byte within `CAP` bytes.
    /// Returns [`AsciiError::TooLong`] if content length exceeds `CAP`.
    /// Returns [`AsciiError::InvalidByte`] if any content byte is > 127.
    ///
    /// Note: "null-terminated" here means null bytes act as terminators
    /// when present, not that one is required. A full-capacity buffer
    /// with no null byte is valid — the content simply fills the entire buffer.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let s: RawAsciiString<32> = RawAsciiString::try_from_null_terminated(b"hello\x00").unwrap();
    /// assert_eq!(s.as_str(), "hello");
    ///
    /// // No null byte — full buffer is content
    /// let s: RawAsciiString<8> = RawAsciiString::try_from_null_terminated(b"abcdefgh").unwrap();
    /// assert_eq!(s.len(), 8);
    /// ```
    #[inline]
    pub fn try_from_null_terminated(bytes: &[u8]) -> Result<Self, AsciiError> {
        let null_pos = find_null_byte(bytes);
        let content_len = if null_pos < bytes.len() {
            null_pos
        } else {
            bytes.len()
        };

        if content_len > CAP {
            return Err(AsciiError::TooLong {
                len: content_len,
                cap: CAP,
            });
        }

        // Validate ASCII on content bytes using SIMD
        if let Err((byte, pos)) = simd::validate_ascii_bounded::<CAP>(&bytes[..content_len]) {
            return Err(AsciiError::InvalidByte { byte, pos });
        }

        let mut data = [0u8; CAP];
        // SAFETY: content_len <= CAP, buffers don't overlap
        unsafe { copy_short(data.as_mut_ptr(), bytes.as_ptr(), content_len) };

        Ok(Self(data))
    }

    /// Creates a raw ASCII string from a raw buffer, taking ownership.
    ///
    /// Validates that all bytes before the first null are valid ASCII (0x01-0x7F).
    /// The buffer is used as-is (no copy).
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::InvalidByte`] if any content byte is > 127.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let mut buf = [0u8; 32];
    /// buf[0] = b'H';
    /// buf[1] = b'i';
    /// let s: RawAsciiString<32> = RawAsciiString::try_from_raw(buf).unwrap();
    /// assert_eq!(s.as_str(), "Hi");
    /// ```
    #[inline]
    pub fn try_from_raw(buffer: [u8; CAP]) -> Result<Self, AsciiError> {
        let () = Self::_CAP_ASSERT;

        let content_len = find_null_byte(&buffer);

        // Validate ASCII on content bytes using SIMD
        if let Err((byte, pos)) = simd::validate_ascii_bounded::<CAP>(&buffer[..content_len]) {
            return Err(AsciiError::InvalidByte { byte, pos });
        }

        Ok(Self(buffer))
    }

    /// Creates a raw ASCII string from a borrowed raw buffer.
    ///
    /// Validates that all bytes before the first null are valid ASCII.
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::InvalidByte`] if any content byte is > 127.
    #[inline]
    pub fn try_from_raw_ref(buffer: &[u8; CAP]) -> Result<Self, AsciiError> {
        Self::try_from_raw(*buffer)
    }

    /// Creates a raw ASCII string from a raw buffer without validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure all bytes before the first null are valid ASCII (0x01-0x7F).
    #[inline]
    pub const unsafe fn from_raw_unchecked(buffer: [u8; CAP]) -> Self {
        let () = Self::_CAP_ASSERT;
        Self(buffer)
    }

    /// Creates a raw ASCII string from a right-padded buffer.
    ///
    /// Strips trailing `pad` bytes, validates ASCII on the remaining content.
    /// The content is copied into a zeroed buffer.
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::InvalidByte`] if any content byte is > 127.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let mut buf = [b' '; 32];
    /// buf[0] = b'H';
    /// buf[1] = b'i';
    /// let s: RawAsciiString<32> = RawAsciiString::try_from_right_padded(buf, b' ').unwrap();
    /// assert_eq!(s.as_str(), "Hi");
    /// ```
    #[inline]
    pub fn try_from_right_padded(buffer: [u8; CAP], pad: u8) -> Result<Self, AsciiError> {
        let () = Self::_CAP_ASSERT;

        // Strip trailing pad bytes
        let mut content_len = CAP;
        while content_len > 0 && buffer[content_len - 1] == pad {
            content_len -= 1;
        }

        // Validate ASCII on content
        if let Err((byte, pos)) = simd::validate_ascii_bounded::<CAP>(&buffer[..content_len]) {
            return Err(AsciiError::InvalidByte { byte, pos });
        }

        // Also reject embedded nulls in content
        let null_pos = find_null_byte(&buffer[..content_len]);
        if null_pos < content_len {
            // Treat content up to the null only
            let mut data = [0u8; CAP];
            // SAFETY: null_pos <= content_len <= CAP
            unsafe { copy_short(data.as_mut_ptr(), buffer.as_ptr(), null_pos) };
            return Ok(Self(data));
        }

        let mut data = [0u8; CAP];
        // SAFETY: content_len <= CAP
        unsafe { copy_short(data.as_mut_ptr(), buffer.as_ptr(), content_len) };

        Ok(Self(data))
    }
}

// =============================================================================
// Accessors
// =============================================================================

impl<const CAP: usize> RawAsciiString<CAP> {
    /// Returns the length of the string content.
    ///
    /// Scans for the first null byte using SIMD. If no null is found,
    /// returns `CAP`.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
    /// assert_eq!(s.len(), 5);
    /// ```
    #[inline]
    pub fn len(&self) -> usize {
        find_null_byte(&self.0)
    }

    /// Returns `true` if the string is empty (first byte is null).
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let empty: RawAsciiString<32> = RawAsciiString::empty();
    /// assert!(empty.is_empty());
    ///
    /// let nonempty: RawAsciiString<32> = RawAsciiString::try_from("hi").unwrap();
    /// assert!(!nonempty.is_empty());
    /// ```
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0[0] == 0
    }

    /// Returns the capacity of the string.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// assert_eq!(RawAsciiString::<32>::capacity(), 32);
    /// ```
    #[inline]
    pub const fn capacity() -> usize {
        CAP
    }

    /// Returns the string content as a byte slice.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
    /// assert_eq!(s.as_bytes(), b"hello");
    /// ```
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0[..self.len()]
    }

    /// Returns the string content as a `&str`.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
    /// assert_eq!(s.as_str(), "hello");
    /// ```
    #[inline]
    pub fn as_str(&self) -> &str {
        let bytes = self.as_bytes();
        debug_assert!(
            bytes.iter().all(|&b| b <= 127),
            "RawAsciiString buffer contains non-ASCII bytes"
        );
        // SAFETY: ASCII bytes are valid UTF-8. Invariant: content bytes are
        // 0x01-0x7F. Violations caught by debug_assert above.
        unsafe { core::str::from_utf8_unchecked(bytes) }
    }

    /// Returns the string content as an `&AsciiStr`.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{RawAsciiString, AsciiStr};
    ///
    /// let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
    /// let ascii_str: &AsciiStr = s.as_ascii_str();
    /// assert_eq!(ascii_str.as_str(), "hello");
    /// ```
    #[inline]
    pub fn as_ascii_str(&self) -> &AsciiStr {
        // SAFETY: content has been validated as ASCII
        unsafe { AsciiStr::from_bytes_unchecked(self.as_bytes()) }
    }

    /// Returns the entire raw buffer as a byte array.
    ///
    /// Includes null terminator and any trailing zeros.
    #[inline]
    pub const fn into_raw(self) -> [u8; CAP] {
        self.0
    }

    /// Returns a reference to the entire raw buffer.
    #[inline]
    pub const fn as_raw(&self) -> &[u8; CAP] {
        &self.0
    }

    /// Returns a mutable reference to the entire raw buffer.
    ///
    /// This gives direct write access to the underlying buffer for wire I/O.
    /// After modification, bytes before the first null must be valid ASCII
    /// (0x01-0x7F). Violations are caught by `debug_assert` in `as_str()`.
    /// Garbage after the first null byte is ignored by all methods.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let mut s: RawAsciiString<32> = RawAsciiString::empty();
    /// let buf = s.as_raw_mut();
    /// buf[0] = b'H';
    /// buf[1] = b'i';
    /// assert_eq!(s.as_str(), "Hi");
    /// ```
    #[inline]
    pub fn as_raw_mut(&mut self) -> &mut [u8; CAP] {
        &mut self.0
    }

    /// Returns the character at the given index, or `None` if out of bounds.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{RawAsciiString, AsciiChar};
    ///
    /// let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
    /// assert_eq!(s.get(0), Some(AsciiChar::h));
    /// assert_eq!(s.get(5), None);
    /// ```
    #[inline]
    pub fn get(&self, index: usize) -> Option<AsciiChar> {
        let len = self.len();
        if index < len {
            // SAFETY: index is within bounds, byte is valid ASCII
            Some(unsafe { AsciiChar::new_unchecked(self.0[index]) })
        } else {
            None
        }
    }

    /// Returns the character at the given index without bounds checking.
    ///
    /// # Safety
    ///
    /// The caller must ensure `index < self.len()`.
    #[inline]
    pub unsafe fn get_unchecked(&self, index: usize) -> AsciiChar {
        debug_assert!(index < self.len());
        // SAFETY: caller guarantees index is within bounds
        unsafe { AsciiChar::new_unchecked(*self.0.get_unchecked(index)) }
    }

    /// Returns an iterator over the ASCII characters.
    #[inline]
    pub fn chars(&self) -> impl Iterator<Item = AsciiChar> + '_ {
        self.as_bytes()
            .iter()
            // SAFETY: all content bytes are valid ASCII
            .map(|&b| unsafe { AsciiChar::new_unchecked(b) })
    }

    /// Returns an iterator over the bytes.
    #[inline]
    pub fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.as_bytes().iter().copied()
    }

    /// Returns the first character, or `None` if empty.
    #[inline]
    pub fn first(&self) -> Option<AsciiChar> {
        if self.is_empty() {
            None
        } else {
            // SAFETY: string is not empty, first byte is valid ASCII
            Some(unsafe { AsciiChar::new_unchecked(self.0[0]) })
        }
    }

    /// Returns the last character, or `None` if empty.
    #[inline]
    pub fn last(&self) -> Option<AsciiChar> {
        let len = self.len();
        if len == 0 {
            None
        } else {
            // SAFETY: len > 0, byte at len-1 is valid ASCII
            Some(unsafe { AsciiChar::new_unchecked(self.0[len - 1]) })
        }
    }
}

// =============================================================================
// Capacity Conversion
// =============================================================================

impl<const CAP: usize> RawAsciiString<CAP> {
    /// Widens the string to a larger capacity.
    ///
    /// # Panics
    ///
    /// Panics at compile time if `NEW_CAP < CAP` or `NEW_CAP % 8 != 0`.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let s: RawAsciiString<8> = RawAsciiString::try_from("hello").unwrap();
    /// let wide: RawAsciiString<32> = s.widen();
    /// assert_eq!(wide.as_str(), "hello");
    /// ```
    #[inline]
    pub fn widen<const NEW_CAP: usize>(self) -> RawAsciiString<NEW_CAP> {
        const { assert!(NEW_CAP % 8 == 0, "NEW_CAP must be a multiple of 8") };

        // Runtime check (can't do CAP comparison in const block with two generics)
        // This will be optimized away since both are const
        assert!(NEW_CAP >= CAP, "NEW_CAP must be >= CAP");

        let mut data = [0u8; NEW_CAP];
        // SAFETY: CAP <= NEW_CAP, buffers don't overlap
        unsafe {
            core::ptr::copy_nonoverlapping(self.0.as_ptr(), data.as_mut_ptr(), CAP);
        }

        RawAsciiString(data)
    }

    /// Tightens the string to a smaller capacity.
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::TooLong`] if the content doesn't fit in `NEW_CAP`.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
    /// let tight: RawAsciiString<8> = s.tighten().unwrap();
    /// assert_eq!(tight.as_str(), "hello");
    /// ```
    #[inline]
    pub fn tighten<const NEW_CAP: usize>(self) -> Result<RawAsciiString<NEW_CAP>, AsciiError> {
        const { assert!(NEW_CAP % 8 == 0, "NEW_CAP must be a multiple of 8") };

        let len = self.len();
        if len > NEW_CAP {
            return Err(AsciiError::TooLong { len, cap: NEW_CAP });
        }

        let mut data = [0u8; NEW_CAP];
        // SAFETY: len <= NEW_CAP, buffers don't overlap
        unsafe { copy_short(data.as_mut_ptr(), self.0.as_ptr(), len) };

        Ok(RawAsciiString(data))
    }
}

// =============================================================================
// Search & Compare
// =============================================================================

impl<const CAP: usize> RawAsciiString<CAP> {
    /// Compares two strings for equality, ignoring ASCII case.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::RawAsciiString;
    ///
    /// let a: RawAsciiString<32> = RawAsciiString::try_from("Hello").unwrap();
    /// let b: RawAsciiString<32> = RawAsciiString::try_from("HELLO").unwrap();
    /// assert!(a.eq_ignore_ascii_case(&b));
    /// ```
    #[inline]
    pub fn eq_ignore_ascii_case(&self, other: &Self) -> bool {
        let a = self.as_bytes();
        let b = other.as_bytes();
        if a.len() != b.len() {
            return false;
        }
        simd::eq_ignore_ascii_case(a, b)
    }

    /// Returns `true` if the string starts with the given prefix.
    #[inline]
    pub fn starts_with(&self, prefix: &[u8]) -> bool {
        self.as_bytes().starts_with(prefix)
    }

    /// Returns `true` if the string ends with the given suffix.
    #[inline]
    pub fn ends_with(&self, suffix: &[u8]) -> bool {
        self.as_bytes().ends_with(suffix)
    }

    /// Returns `true` if the string contains the given pattern.
    #[inline]
    pub fn contains(&self, pattern: &[u8]) -> bool {
        if pattern.is_empty() {
            return true;
        }
        self.as_bytes().windows(pattern.len()).any(|w| w == pattern)
    }

    /// Finds the position of the first occurrence of a byte.
    #[inline]
    pub fn find_byte(&self, byte: u8) -> Option<usize> {
        self.as_bytes().iter().position(|&b| b == byte)
    }

    /// Finds the position of the first occurrence of an ASCII character.
    #[inline]
    pub fn find_char(&self, ch: AsciiChar) -> Option<usize> {
        self.find_byte(ch.as_u8())
    }

    /// Finds the position of the first occurrence of a byte pattern.
    #[inline]
    pub fn find(&self, pattern: &[u8]) -> Option<usize> {
        self.as_bytes()
            .windows(pattern.len())
            .position(|w| w == pattern)
    }

    /// Finds the position of the last occurrence of a byte.
    #[inline]
    pub fn rfind_byte(&self, byte: u8) -> Option<usize> {
        self.as_bytes().iter().rposition(|&b| b == byte)
    }

    /// Finds the position of the last occurrence of an ASCII character.
    #[inline]
    pub fn rfind_char(&self, ch: AsciiChar) -> Option<usize> {
        self.rfind_byte(ch.as_u8())
    }

    /// Finds the position of the last occurrence of a byte pattern.
    #[inline]
    pub fn rfind(&self, pattern: &[u8]) -> Option<usize> {
        self.as_bytes()
            .windows(pattern.len())
            .rposition(|w| w == pattern)
    }

    /// Returns the string with the given prefix removed, or `None`.
    #[inline]
    pub fn strip_prefix(&self, prefix: &[u8]) -> Option<&AsciiStr> {
        let bytes = self.as_bytes();
        if bytes.starts_with(prefix) {
            // SAFETY: bytes after prefix are valid ASCII
            Some(unsafe { AsciiStr::from_bytes_unchecked(&bytes[prefix.len()..]) })
        } else {
            None
        }
    }

    /// Returns the string with the given suffix removed, or `None`.
    #[inline]
    pub fn strip_suffix(&self, suffix: &[u8]) -> Option<&AsciiStr> {
        let bytes = self.as_bytes();
        if bytes.ends_with(suffix) {
            // SAFETY: bytes before suffix are valid ASCII
            Some(unsafe { AsciiStr::from_bytes_unchecked(&bytes[..bytes.len() - suffix.len()]) })
        } else {
            None
        }
    }
}

// =============================================================================
// Trim (borrowed → &AsciiStr)
// =============================================================================

impl<const CAP: usize> RawAsciiString<CAP> {
    /// Returns the string with leading and trailing whitespace removed.
    #[inline]
    pub fn trim(&self) -> &AsciiStr {
        let bytes = self.as_bytes();
        let start = bytes.iter().position(|&b| b != b' ').unwrap_or(bytes.len());
        let end = bytes
            .iter()
            .rposition(|&b| b != b' ')
            .map_or(start, |p| p + 1);
        // SAFETY: trimmed bytes are a subset of valid ASCII
        unsafe { AsciiStr::from_bytes_unchecked(&bytes[start..end]) }
    }

    /// Returns the string with leading whitespace removed.
    #[inline]
    pub fn trim_start(&self) -> &AsciiStr {
        let bytes = self.as_bytes();
        let start = bytes.iter().position(|&b| b != b' ').unwrap_or(bytes.len());
        // SAFETY: trimmed bytes are valid ASCII
        unsafe { AsciiStr::from_bytes_unchecked(&bytes[start..]) }
    }

    /// Returns the string with trailing whitespace removed.
    #[inline]
    pub fn trim_end(&self) -> &AsciiStr {
        let bytes = self.as_bytes();
        let end = bytes.iter().rposition(|&b| b != b' ').map_or(0, |p| p + 1);
        // SAFETY: trimmed bytes are valid ASCII
        unsafe { AsciiStr::from_bytes_unchecked(&bytes[..end]) }
    }
}

// =============================================================================
// Split
// =============================================================================

impl<const CAP: usize> RawAsciiString<CAP> {
    /// Returns an iterator over substrings separated by the given delimiter byte.
    #[inline]
    pub fn split(&self, delimiter: u8) -> crate::string::Split<'_> {
        crate::string::Split {
            remainder: self.as_bytes(),
            delimiter,
            finished: false,
        }
    }

    /// Splits the string at the first occurrence of the delimiter.
    ///
    /// Returns `None` if the delimiter is not found.
    #[inline]
    pub fn split_once(&self, delimiter: u8) -> Option<(&AsciiStr, &AsciiStr)> {
        let bytes = self.as_bytes();
        let pos = bytes.iter().position(|&b| b == delimiter)?;
        // SAFETY: both halves are subsets of valid ASCII bytes
        unsafe {
            let left = AsciiStr::from_bytes_unchecked(&bytes[..pos]);
            let right = AsciiStr::from_bytes_unchecked(&bytes[pos + 1..]);
            Some((left, right))
        }
    }
}

// =============================================================================
// Transformations (owned)
// =============================================================================

impl<const CAP: usize> RawAsciiString<CAP> {
    /// Returns a copy with all ASCII letters uppercased.
    ///
    /// Processes the full buffer (safe: 0x00 is not a letter).
    #[inline]
    pub fn to_ascii_uppercase(self) -> Self {
        let mut data = self.0;
        simd::make_uppercase(&mut data);
        Self(data)
    }

    /// Returns a copy with all ASCII letters lowercased.
    ///
    /// Processes the full buffer (safe: 0x00 is not a letter).
    #[inline]
    pub fn to_ascii_lowercase(self) -> Self {
        let mut data = self.0;
        simd::make_lowercase(&mut data);
        Self(data)
    }

    /// Returns a copy truncated to `new_len` bytes.
    ///
    /// If `new_len >= self.len()`, returns a copy unchanged.
    /// If `new_len < CAP`, sets `data[new_len] = 0`.
    ///
    /// # Panics
    ///
    /// Panics if `new_len > CAP`.
    #[inline]
    pub fn truncated(self, new_len: usize) -> Self {
        assert!(new_len <= CAP, "truncation length exceeds capacity");

        let len = self.len();
        if new_len >= len {
            return self;
        }

        let mut data = self.0;
        // Zero from new_len onward
        // SAFETY: new_len <= CAP
        unsafe {
            core::ptr::write_bytes(data.as_mut_ptr().add(new_len), 0, CAP - new_len);
        }
        Self(data)
    }

    /// Returns a copy truncated to `new_len` bytes, or `None` if `new_len > CAP`.
    #[inline]
    pub fn try_truncated(self, new_len: usize) -> Option<Self> {
        if new_len > CAP {
            None
        } else {
            Some(self.truncated(new_len))
        }
    }

    /// Returns a copy with leading and trailing spaces removed.
    #[inline]
    pub fn trimmed(self) -> Self {
        let trimmed = self.trim();
        let trimmed_bytes = trimmed.as_bytes();
        let mut data = [0u8; CAP];
        // SAFETY: trimmed_bytes.len() <= self.len() <= CAP
        unsafe {
            copy_short(
                data.as_mut_ptr(),
                trimmed_bytes.as_ptr(),
                trimmed_bytes.len(),
            );
        }
        Self(data)
    }

    /// Returns a copy with leading spaces removed.
    #[inline]
    pub fn trimmed_start(self) -> Self {
        let trimmed = self.trim_start();
        let trimmed_bytes = trimmed.as_bytes();
        let mut data = [0u8; CAP];
        // SAFETY: trimmed_bytes.len() <= self.len() <= CAP
        unsafe {
            copy_short(
                data.as_mut_ptr(),
                trimmed_bytes.as_ptr(),
                trimmed_bytes.len(),
            );
        }
        Self(data)
    }

    /// Returns a copy with trailing spaces removed.
    #[inline]
    pub fn trimmed_end(self) -> Self {
        let trimmed = self.trim_end();
        let trimmed_bytes = trimmed.as_bytes();
        let mut data = [0u8; CAP];
        // SAFETY: trimmed_bytes.len() <= self.len() <= CAP
        unsafe {
            copy_short(
                data.as_mut_ptr(),
                trimmed_bytes.as_ptr(),
                trimmed_bytes.len(),
            );
        }
        Self(data)
    }

    /// Returns a copy with all occurrences of `from` replaced with `to`.
    ///
    /// Both arguments are `AsciiChar`, guaranteeing the result contains valid ASCII.
    /// Operates on content bytes only (`[..len()]`).
    #[inline]
    pub fn replaced_char(self, from: AsciiChar, to: AsciiChar) -> Self {
        let len = self.len();
        let mut data = self.0;
        let from = from.as_u8();
        let to = to.as_u8();
        for byte in &mut data[..len] {
            if *byte == from {
                *byte = to;
            }
        }
        Self(data)
    }

    /// Returns a copy with all occurrences of `from` replaced with `to` (raw bytes).
    ///
    /// # Safety
    ///
    /// The caller must ensure `to` is valid ASCII (0x00-0x7F). A non-ASCII `to`
    /// corrupts the buffer — subsequent `as_str()` calls are UB.
    #[inline]
    pub unsafe fn replaced_byte(self, from: u8, to: u8) -> Self {
        let len = self.len();
        let mut data = self.0;
        for byte in &mut data[..len] {
            if *byte == from {
                *byte = to;
            }
        }
        Self(data)
    }

    /// Returns a copy with the first occurrence of `from` replaced with `to`.
    #[inline]
    pub fn replace_first_char(self, from: AsciiChar, to: AsciiChar) -> Self {
        let len = self.len();
        let mut data = self.0;
        let from = from.as_u8();
        let to = to.as_u8();
        for byte in &mut data[..len] {
            if *byte == from {
                *byte = to;
                break;
            }
        }
        Self(data)
    }

    /// Returns a copy with the first occurrence of `from` replaced with `to` (raw bytes).
    ///
    /// # Safety
    ///
    /// The caller must ensure `to` is valid ASCII (0x00-0x7F).
    #[inline]
    pub unsafe fn replace_first_byte(self, from: u8, to: u8) -> Self {
        let len = self.len();
        let mut data = self.0;
        for byte in &mut data[..len] {
            if *byte == from {
                *byte = to;
                break;
            }
        }
        Self(data)
    }

    /// Returns a copy with all occurrences of `from` pattern replaced with `to`.
    ///
    /// If the result would exceed capacity, the output is truncated.
    /// Returns `self` unchanged if `from` is empty.
    ///
    /// # Safety
    ///
    /// The caller must ensure all bytes in `to` are valid ASCII (0x00-0x7F).
    #[inline]
    pub unsafe fn replaced(self, from: &[u8], to: &[u8]) -> Self {
        if from.is_empty() {
            return self;
        }

        let content = self.as_bytes();
        let mut data = [0u8; CAP];
        let mut wi = 0;
        let mut ri = 0;

        while ri < content.len() && wi < CAP {
            if ri + from.len() <= content.len() && &content[ri..ri + from.len()] == from {
                let copy_len = to.len().min(CAP - wi);
                data[wi..wi + copy_len].copy_from_slice(&to[..copy_len]);
                wi += copy_len;
                ri += from.len();
            } else {
                data[wi] = content[ri];
                wi += 1;
                ri += 1;
            }
        }

        Self(data)
    }

    /// Returns a copy with the first occurrence of `from` pattern replaced with `to`.
    ///
    /// If the result would exceed capacity, the output is truncated.
    /// Returns `self` unchanged if `from` is empty or not found.
    ///
    /// # Safety
    ///
    /// The caller must ensure all bytes in `to` are valid ASCII (0x00-0x7F).
    #[inline]
    pub unsafe fn replace_first(self, from: &[u8], to: &[u8]) -> Self {
        if from.is_empty() {
            return self;
        }

        let content = self.as_bytes();

        let Some(pos) = content.windows(from.len()).position(|w| w == from) else {
            return self;
        };

        let mut data = [0u8; CAP];
        let mut wi = 0;

        // Copy prefix
        let prefix_len = pos.min(CAP);
        data[..prefix_len].copy_from_slice(&content[..prefix_len]);
        wi += prefix_len;

        // Copy replacement
        let copy_len = to.len().min(CAP - wi);
        data[wi..wi + copy_len].copy_from_slice(&to[..copy_len]);
        wi += copy_len;

        // Copy suffix
        let suffix_start = pos + from.len();
        if suffix_start < content.len() {
            let suffix_len = (content.len() - suffix_start).min(CAP - wi);
            data[wi..wi + suffix_len]
                .copy_from_slice(&content[suffix_start..suffix_start + suffix_len]);
        }

        Self(data)
    }
}

// =============================================================================
// Classification
// =============================================================================

impl<const CAP: usize> RawAsciiString<CAP> {
    /// Returns `true` if all content characters are printable ASCII (0x20-0x7E).
    #[inline]
    pub fn is_all_printable(&self) -> bool {
        simd::is_all_printable(self.as_bytes())
    }

    /// Returns `true` if the content contains any control characters (< 0x20 or 0x7F).
    #[inline]
    pub fn contains_control_chars(&self) -> bool {
        crate::simd::contains_control_chars(self.as_bytes())
    }

    /// Returns `true` if all characters are ASCII digits (0-9).
    ///
    /// An empty string returns `true`.
    #[inline]
    pub fn is_numeric(&self) -> bool {
        crate::simd::is_all_numeric(self.as_bytes())
    }

    /// Returns `true` if all characters are ASCII alphanumeric (A-Z, a-z, 0-9).
    ///
    /// An empty string returns `true`.
    #[inline]
    pub fn is_alphanumeric(&self) -> bool {
        crate::simd::is_all_alphanumeric(self.as_bytes())
    }
}

// =============================================================================
// Conversion
// =============================================================================

impl<const CAP: usize> RawAsciiString<CAP> {
    /// Attempts to convert this string into a `RawAsciiText`.
    ///
    /// `RawAsciiText` only allows printable ASCII (0x20-0x7E).
    ///
    /// # Errors
    ///
    /// Returns [`AsciiError::NonPrintable`] if any content byte is non-printable.
    #[inline]
    pub fn try_into_raw_text(self) -> Result<crate::RawAsciiText<CAP>, AsciiError> {
        crate::RawAsciiText::try_from_raw_ascii_string(self)
    }

    /// Promotes this raw string to an `AsciiString` with precomputed hash.
    ///
    /// # Example
    ///
    /// ```
    /// use nexus_ascii::{RawAsciiString, AsciiString};
    ///
    /// let raw: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
    /// let hashed: AsciiString<32> = raw.to_ascii_string();
    /// assert_eq!(hashed.as_str(), "hello");
    /// ```
    #[inline]
    pub fn to_ascii_string(self) -> crate::AsciiString<CAP> {
        let len = self.len();
        // from_parts_unchecked computes the hash from data[..len].
        // Content is valid ASCII, length is correct.
        crate::AsciiString::from_parts_unchecked(len, self.0)
    }
}

// =============================================================================
// Integer Parsing
// =============================================================================

crate::parse::impl_parse_int_generic!(RawAsciiString, as_str);

// =============================================================================
// Integer Formatting
// =============================================================================

crate::format::impl_format_int_generic!(RawAsciiString, from_bytes_unchecked);

// =============================================================================
// Trait Implementations
// =============================================================================

impl<const CAP: usize> Default for RawAsciiString<CAP> {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl<const CAP: usize> core::fmt::Debug for RawAsciiString<CAP> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let len = self.len();
        // SAFETY: content bytes are valid ASCII (valid UTF-8)
        let value = unsafe { core::str::from_utf8_unchecked(&self.0[..len]) };
        f.debug_struct("RawAsciiString")
            .field("value", &value)
            .field("len", &len)
            .field("cap", &CAP)
            .finish()
    }
}

impl<const CAP: usize> core::fmt::Display for RawAsciiString<CAP> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const CAP: usize> core::ops::Deref for RawAsciiString<CAP> {
    type Target = AsciiStr;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ascii_str()
    }
}

impl<const CAP: usize> core::ops::Index<usize> for RawAsciiString<CAP> {
    type Output = AsciiChar;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        assert!(index < self.len(), "index out of bounds");
        // SAFETY: index is within bounds, data contains valid ASCII.
        // AsciiChar is #[repr(transparent)] over u8.
        unsafe { &*(self.0.get_unchecked(index) as *const u8 as *const AsciiChar) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::Range<usize>> for RawAsciiString<CAP> {
    type Output = AsciiStr;

    #[inline]
    fn index(&self, range: core::ops::Range<usize>) -> &Self::Output {
        assert!(range.start <= range.end, "range start > end");
        assert!(range.end <= self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, data contains valid ASCII
        unsafe { AsciiStr::from_bytes_unchecked(&self.0[range]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeFrom<usize>> for RawAsciiString<CAP> {
    type Output = AsciiStr;

    #[inline]
    fn index(&self, range: core::ops::RangeFrom<usize>) -> &Self::Output {
        assert!(range.start <= self.len(), "range start out of bounds");
        // SAFETY: range is within bounds, data contains valid ASCII
        unsafe { AsciiStr::from_bytes_unchecked(&self.0[range.start..self.len()]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeTo<usize>> for RawAsciiString<CAP> {
    type Output = AsciiStr;

    #[inline]
    fn index(&self, range: core::ops::RangeTo<usize>) -> &Self::Output {
        assert!(range.end <= self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, data contains valid ASCII
        unsafe { AsciiStr::from_bytes_unchecked(&self.0[range]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeFull> for RawAsciiString<CAP> {
    type Output = AsciiStr;

    #[inline]
    fn index(&self, _range: core::ops::RangeFull) -> &Self::Output {
        self.as_ascii_str()
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeInclusive<usize>> for RawAsciiString<CAP> {
    type Output = AsciiStr;

    #[inline]
    fn index(&self, range: core::ops::RangeInclusive<usize>) -> &Self::Output {
        let start = *range.start();
        let end = *range.end();
        assert!(start <= end, "range start > end");
        assert!(end < self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, data contains valid ASCII
        unsafe { AsciiStr::from_bytes_unchecked(&self.0[start..=end]) }
    }
}

impl<const CAP: usize> core::ops::Index<core::ops::RangeToInclusive<usize>>
    for RawAsciiString<CAP>
{
    type Output = AsciiStr;

    #[inline]
    fn index(&self, range: core::ops::RangeToInclusive<usize>) -> &Self::Output {
        assert!(range.end < self.len(), "range end out of bounds");
        // SAFETY: range is within bounds, data contains valid ASCII
        unsafe { AsciiStr::from_bytes_unchecked(&self.0[range]) }
    }
}

// =============================================================================
// TryFrom Implementations
// =============================================================================

impl<const CAP: usize> TryFrom<&str> for RawAsciiString<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::try_from_str(s)
    }
}

impl<const CAP: usize> TryFrom<&[u8]> for RawAsciiString<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::try_from_bytes(bytes)
    }
}

#[cfg(feature = "std")]
impl<const CAP: usize> TryFrom<std::string::String> for RawAsciiString<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: std::string::String) -> Result<Self, Self::Error> {
        Self::try_from_str(&s)
    }
}

#[cfg(feature = "std")]
impl<const CAP: usize> TryFrom<&std::string::String> for RawAsciiString<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(s: &std::string::String) -> Result<Self, Self::Error> {
        Self::try_from_str(s)
    }
}

// =============================================================================
// AsRef Implementations
// =============================================================================

impl<const CAP: usize> AsRef<str> for RawAsciiString<CAP> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const CAP: usize> AsRef<[u8]> for RawAsciiString<CAP> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<const CAP: usize> AsRef<AsciiStr> for RawAsciiString<CAP> {
    #[inline]
    fn as_ref(&self) -> &AsciiStr {
        self.as_ascii_str()
    }
}

impl<const CAP: usize> AsRef<[u8; CAP]> for RawAsciiString<CAP> {
    #[inline]
    fn as_ref(&self) -> &[u8; CAP] {
        self.as_raw()
    }
}

// =============================================================================
// Serde Support (feature-gated)
// =============================================================================

#[cfg(feature = "serde")]
impl<const CAP: usize> serde::Serialize for RawAsciiString<CAP> {
    #[inline]
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de, const CAP: usize> serde::Deserialize<'de> for RawAsciiString<CAP> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RawAsciiStringVisitor<const CAP: usize>;

        impl<const CAP: usize> serde::de::Visitor<'_> for RawAsciiStringVisitor<CAP> {
            type Value = RawAsciiString<CAP>;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(formatter, "an ASCII string with at most {} bytes", CAP)
            }

            #[inline]
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                RawAsciiString::try_from_str(v).map_err(|e| match e {
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
                RawAsciiString::try_from_bytes(v).map_err(|e| match e {
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

        deserializer.deserialize_str(RawAsciiStringVisitor)
    }
}

// =============================================================================
// Bytes Crate Support (feature-gated)
// =============================================================================

#[cfg(feature = "bytes")]
impl<const CAP: usize> From<RawAsciiString<CAP>> for bytes::Bytes {
    #[inline]
    fn from(s: RawAsciiString<CAP>) -> Self {
        bytes::Bytes::copy_from_slice(s.as_bytes())
    }
}

#[cfg(feature = "bytes")]
impl<const CAP: usize> From<&RawAsciiString<CAP>> for bytes::Bytes {
    #[inline]
    fn from(s: &RawAsciiString<CAP>) -> Self {
        bytes::Bytes::copy_from_slice(s.as_bytes())
    }
}

#[cfg(feature = "bytes")]
impl<const CAP: usize> TryFrom<bytes::Bytes> for RawAsciiString<CAP> {
    type Error = AsciiError;

    #[inline]
    fn try_from(bytes: bytes::Bytes) -> Result<Self, Self::Error> {
        Self::try_from_bytes(&bytes)
    }
}

#[cfg(feature = "bytes")]
impl<const CAP: usize> TryFrom<&bytes::Bytes> for RawAsciiString<CAP> {
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
    fn empty_string() {
        let s: RawAsciiString<32> = RawAsciiString::empty();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s.as_str(), "");
        assert_eq!(s.as_bytes(), b"");
    }

    #[test]
    fn from_str() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
        assert_eq!(s.len(), 5);
        assert_eq!(s.as_str(), "hello");
    }

    #[test]
    fn from_bytes() {
        let s: RawAsciiString<32> = RawAsciiString::try_from_bytes(b"world").unwrap();
        assert_eq!(s.len(), 5);
        assert_eq!(s.as_str(), "world");
    }

    #[test]
    fn too_long() {
        let result = RawAsciiString::<8>::try_from("hello world");
        assert!(matches!(
            result,
            Err(AsciiError::TooLong { len: 11, cap: 8 })
        ));
    }

    #[test]
    fn invalid_ascii() {
        let result = RawAsciiString::<32>::try_from_bytes(&[0x80]);
        assert!(matches!(
            result,
            Err(AsciiError::InvalidByte { byte: 0x80, pos: 0 })
        ));
    }

    #[test]
    fn null_termination() {
        let s: RawAsciiString<32> = RawAsciiString::try_from_bytes(b"hi\x00world").unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s.as_str(), "hi");
    }

    #[test]
    fn full_buffer_no_null() {
        let s: RawAsciiString<8> = RawAsciiString::try_from("abcdefgh").unwrap();
        assert_eq!(s.len(), 8);
        assert_eq!(s.as_str(), "abcdefgh");
    }

    #[test]
    fn from_static_const() {
        const S: RawAsciiString<16> = RawAsciiString::from_static("BTC-USD");
        assert_eq!(S.as_str(), "BTC-USD");
        assert_eq!(S.len(), 7);
    }

    #[test]
    fn from_static_bytes_const() {
        const S: RawAsciiString<16> = RawAsciiString::from_static_bytes(b"ETH-USD");
        assert_eq!(S.as_str(), "ETH-USD");
    }

    #[test]
    fn as_raw_mut_write() {
        let mut s: RawAsciiString<32> = RawAsciiString::empty();
        let buf = s.as_raw_mut();
        buf[0] = b'H';
        buf[1] = b'i';
        assert_eq!(s.as_str(), "Hi");
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn try_from_raw() {
        let mut buf = [0u8; 32];
        buf[0] = b'A';
        buf[1] = b'B';
        buf[2] = b'C';
        let s: RawAsciiString<32> = RawAsciiString::try_from_raw(buf).unwrap();
        assert_eq!(s.as_str(), "ABC");
    }

    #[test]
    fn try_from_raw_invalid() {
        let mut buf = [0u8; 32];
        buf[0] = 0x80;
        let result = RawAsciiString::<32>::try_from_raw(buf);
        assert!(result.is_err());
    }

    #[test]
    fn try_from_right_padded() {
        let mut buf = [b' '; 32];
        buf[0] = b'H';
        buf[1] = b'i';
        let s: RawAsciiString<32> = RawAsciiString::try_from_right_padded(buf, b' ').unwrap();
        assert_eq!(s.as_str(), "Hi");
    }

    #[test]
    fn try_from_null_terminated() {
        let s: RawAsciiString<32> = RawAsciiString::try_from_null_terminated(b"hello\x00").unwrap();
        assert_eq!(s.as_str(), "hello");
    }

    #[test]
    fn widen() {
        let s: RawAsciiString<8> = RawAsciiString::try_from("hello").unwrap();
        let wide: RawAsciiString<32> = s.widen();
        assert_eq!(wide.as_str(), "hello");
    }

    #[test]
    fn tighten() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
        let tight: RawAsciiString<8> = s.tighten().unwrap();
        assert_eq!(tight.as_str(), "hello");
    }

    #[test]
    fn tighten_too_long() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("hello world").unwrap();
        let result: Result<RawAsciiString<8>, _> = s.tighten();
        assert!(result.is_err());
    }

    #[test]
    fn uppercase_lowercase() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("Hello World").unwrap();
        assert_eq!(s.to_ascii_uppercase().as_str(), "HELLO WORLD");
        assert_eq!(s.to_ascii_lowercase().as_str(), "hello world");
    }

    #[test]
    fn truncated() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("hello world").unwrap();
        let t = s.truncated(5);
        assert_eq!(t.as_str(), "hello");
    }

    #[test]
    fn trimmed() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("  hello  ").unwrap();
        assert_eq!(s.trimmed().as_str(), "hello");
        assert_eq!(s.trimmed_start().as_str(), "hello  ");
        assert_eq!(s.trimmed_end().as_str(), "  hello");
    }

    #[test]
    fn eq_ignore_ascii_case() {
        let a: RawAsciiString<32> = RawAsciiString::try_from("Hello").unwrap();
        let b: RawAsciiString<32> = RawAsciiString::try_from("HELLO").unwrap();
        assert!(a.eq_ignore_ascii_case(&b));
    }

    #[test]
    fn find_and_contains() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("BTC-USD").unwrap();
        assert!(s.contains(b"BTC"));
        assert!(s.starts_with(b"BTC"));
        assert!(s.ends_with(b"USD"));
        assert_eq!(s.find_byte(b'-'), Some(3));
        assert_eq!(s.find(b"USD"), Some(4));
    }

    #[test]
    fn split() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("a,b,c").unwrap();
        let parts: Vec<&str> = s.split(b',').map(|p| p.as_str()).collect();
        assert_eq!(parts, vec!["a", "b", "c"]);
    }

    #[test]
    fn split_once() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("key=value").unwrap();
        let (k, v) = s.split_once(b'=').unwrap();
        assert_eq!(k.as_str(), "key");
        assert_eq!(v.as_str(), "value");
    }

    #[test]
    fn replaced_char() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("a-b-c").unwrap();
        let minus = AsciiChar::try_new(b'-').unwrap();
        let underscore = AsciiChar::try_new(b'_').unwrap();
        assert_eq!(s.replaced_char(minus, underscore).as_str(), "a_b_c");
    }

    #[test]
    fn replaced_byte() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("a-b-c").unwrap();
        // SAFETY: b'_' is valid ASCII
        assert_eq!(unsafe { s.replaced_byte(b'-', b'_') }.as_str(), "a_b_c");
    }

    #[test]
    fn replace_first_char() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("a-b-c").unwrap();
        let minus = AsciiChar::try_new(b'-').unwrap();
        let underscore = AsciiChar::try_new(b'_').unwrap();
        assert_eq!(s.replace_first_char(minus, underscore).as_str(), "a_b-c");
    }

    #[test]
    fn replace_first_byte() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("a-b-c").unwrap();
        // SAFETY: b'_' is valid ASCII
        assert_eq!(
            unsafe { s.replace_first_byte(b'-', b'_') }.as_str(),
            "a_b-c"
        );
    }

    #[test]
    fn classification() {
        let printable: RawAsciiString<32> = RawAsciiString::try_from("Hello").unwrap();
        assert!(printable.is_all_printable());
        assert!(!printable.contains_control_chars());

        let digits: RawAsciiString<32> = RawAsciiString::try_from("12345").unwrap();
        assert!(digits.is_numeric());
        assert!(digits.is_alphanumeric());

        let alpha: RawAsciiString<32> = RawAsciiString::try_from("abc123").unwrap();
        assert!(!alpha.is_numeric());
        assert!(alpha.is_alphanumeric());
    }

    #[test]
    fn to_ascii_string_promotion() {
        let raw: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
        let hashed = raw.to_ascii_string();
        assert_eq!(hashed.as_str(), "hello");
        assert_eq!(hashed.len(), 5);
    }

    #[test]
    fn default_is_empty() {
        let s: RawAsciiString<32> = Default::default();
        assert!(s.is_empty());
    }

    #[test]
    fn display() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
        assert_eq!(format!("{}", s), "hello");
    }

    #[test]
    fn debug() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("hi").unwrap();
        let debug = format!("{:?}", s);
        assert!(debug.contains("RawAsciiString"));
        assert!(debug.contains("hi"));
        assert!(debug.contains("32"));
    }

    #[test]
    fn deref_to_ascii_str() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
        let ascii_str: &AsciiStr = &*s;
        assert_eq!(ascii_str.as_str(), "hello");
    }

    #[test]
    fn index_usize() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
        assert_eq!(s[0], AsciiChar::h);
        assert_eq!(s[4], AsciiChar::o);
    }

    #[test]
    fn index_range() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("BTC-USD").unwrap();
        assert_eq!(&s[0..3], "BTC");
        assert_eq!(&s[4..7], "USD");
    }

    #[test]
    fn strip_prefix_suffix() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("BTC-USD").unwrap();
        assert_eq!(s.strip_prefix(b"BTC-").unwrap().as_str(), "USD");
        assert_eq!(s.strip_suffix(b"-USD").unwrap().as_str(), "BTC");
        assert!(s.strip_prefix(b"ETH").is_none());
    }

    #[test]
    fn get_first_last() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
        assert_eq!(s.get(0), Some(AsciiChar::h));
        assert_eq!(s.get(5), None);
        assert_eq!(s.first(), Some(AsciiChar::h));
        assert_eq!(s.last(), Some(AsciiChar::o));

        let empty: RawAsciiString<32> = RawAsciiString::empty();
        assert_eq!(empty.first(), None);
        assert_eq!(empty.last(), None);
    }

    #[test]
    fn replaced() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("foo bar foo").unwrap();
        // SAFETY: b"baz" is valid ASCII
        assert_eq!(
            unsafe { s.replaced(b"foo", b"baz") }.as_str(),
            "baz bar baz"
        );
    }

    #[test]
    fn replaced_empty_from_is_noop() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("hello").unwrap();
        // SAFETY: to is valid ASCII; empty from returns self unchanged
        assert_eq!(unsafe { s.replaced(b"", b"x") }.as_str(), "hello");
    }

    #[test]
    fn replace_first() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("foo bar foo").unwrap();
        // SAFETY: b"baz" is valid ASCII
        assert_eq!(
            unsafe { s.replace_first(b"foo", b"baz") }.as_str(),
            "baz bar foo"
        );
    }

    #[test]
    fn try_from_null_terminated_full_buffer() {
        let s: RawAsciiString<8> = RawAsciiString::try_from_null_terminated(b"abcdefgh").unwrap();
        assert_eq!(s.len(), 8);
        assert_eq!(s.as_str(), "abcdefgh");
    }

    #[test]
    fn try_from_null_terminated_too_long() {
        let result = RawAsciiString::<8>::try_from_null_terminated(b"123456789");
        assert!(matches!(
            result,
            Err(AsciiError::TooLong { len: 9, cap: 8 })
        ));
    }

    #[test]
    fn try_from_raw_ref() {
        let buf: [u8; 8] = *b"hello\0\0\0";
        let s: RawAsciiString<8> = RawAsciiString::try_from_raw_ref(&buf).unwrap();
        assert_eq!(s.as_str(), "hello");
    }

    #[test]
    fn contains_empty_pattern() {
        let s: RawAsciiString<16> = RawAsciiString::try_from("hello").unwrap();
        assert!(s.contains(b""));
    }

    #[test]
    fn replace_first_only_first() {
        let s: RawAsciiString<32> = RawAsciiString::try_from("aaa").unwrap();
        // SAFETY: b"x" is valid ASCII
        let r = unsafe { s.replace_first(b"a", b"x") };
        assert_eq!(r.as_str(), "xaa");
    }

    #[test]
    fn capacity() {
        assert_eq!(RawAsciiString::<8>::capacity(), 8);
        assert_eq!(RawAsciiString::<32>::capacity(), 32);
        assert_eq!(RawAsciiString::<256>::capacity(), 256);
    }
}
