//! Parse error type for ID string parsing.

use core::fmt;

/// Error returned when parsing an ID string fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub(crate) kind: ParseErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParseErrorKind {
    /// Input length doesn't match expected format length.
    InvalidLength { expected: usize, got: usize },
    /// Invalid character at the given position.
    InvalidChar { position: usize, byte: u8 },
    /// Structural format error (e.g., missing dashes in UUID).
    InvalidFormat,
    /// Prefix validation failed (TypeID).
    InvalidPrefix,
    /// Value would overflow the target type.
    Overflow,
}

impl ParseError {
    #[inline]
    pub(crate) const fn invalid_length(expected: usize, got: usize) -> Self {
        Self {
            kind: ParseErrorKind::InvalidLength { expected, got },
        }
    }

    #[inline]
    pub(crate) const fn invalid_char(position: usize, byte: u8) -> Self {
        Self {
            kind: ParseErrorKind::InvalidChar { position, byte },
        }
    }

    #[inline]
    pub(crate) const fn invalid_format() -> Self {
        Self {
            kind: ParseErrorKind::InvalidFormat,
        }
    }

    #[inline]
    pub(crate) const fn invalid_prefix() -> Self {
        Self {
            kind: ParseErrorKind::InvalidPrefix,
        }
    }

    #[inline]
    pub(crate) const fn overflow() -> Self {
        Self {
            kind: ParseErrorKind::Overflow,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ParseErrorKind::InvalidLength { expected, got } => {
                write!(f, "invalid length: expected {}, got {}", expected, got)
            }
            ParseErrorKind::InvalidChar { position, byte } => {
                write!(
                    f,
                    "invalid character 0x{:02x} at position {}",
                    byte, position
                )
            }
            ParseErrorKind::InvalidFormat => {
                write!(f, "invalid format")
            }
            ParseErrorKind::InvalidPrefix => {
                write!(f, "invalid prefix: must be lowercase ASCII [a-z]")
            }
            ParseErrorKind::Overflow => {
                write!(f, "value overflows target type")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

// =============================================================================
// Validation helpers
// =============================================================================

/// Validate and decode a hex character, returning value or error.
#[inline]
pub(crate) const fn validate_hex(b: u8, position: usize) -> Result<u8, ParseError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(ParseError::invalid_char(position, b)),
    }
}

/// Validate and decode a Crockford Base32 character.
#[inline]
pub(crate) const fn validate_crockford32(b: u8, position: usize) -> Result<u8, ParseError> {
    match b {
        b'0' | b'O' | b'o' => Ok(0),
        b'1' | b'I' | b'i' | b'L' | b'l' => Ok(1),
        b'2' => Ok(2),
        b'3' => Ok(3),
        b'4' => Ok(4),
        b'5' => Ok(5),
        b'6' => Ok(6),
        b'7' => Ok(7),
        b'8' => Ok(8),
        b'9' => Ok(9),
        b'A' | b'a' => Ok(10),
        b'B' | b'b' => Ok(11),
        b'C' | b'c' => Ok(12),
        b'D' | b'd' => Ok(13),
        b'E' | b'e' => Ok(14),
        b'F' | b'f' => Ok(15),
        b'G' | b'g' => Ok(16),
        b'H' | b'h' => Ok(17),
        b'J' | b'j' => Ok(18),
        b'K' | b'k' => Ok(19),
        b'M' | b'm' => Ok(20),
        b'N' | b'n' => Ok(21),
        b'P' | b'p' => Ok(22),
        b'Q' | b'q' => Ok(23),
        b'R' | b'r' => Ok(24),
        b'S' | b's' => Ok(25),
        b'T' | b't' => Ok(26),
        b'V' | b'v' => Ok(27),
        b'W' | b'w' => Ok(28),
        b'X' | b'x' => Ok(29),
        b'Y' | b'y' => Ok(30),
        b'Z' | b'z' => Ok(31),
        _ => Err(ParseError::invalid_char(position, b)),
    }
}

/// Validate and decode a base62 character.
#[inline]
pub(crate) const fn validate_base62(b: u8, position: usize) -> Result<u8, ParseError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'A'..=b'Z' => Ok(b - b'A' + 10),
        b'a'..=b'z' => Ok(b - b'a' + 36),
        _ => Err(ParseError::invalid_char(position, b)),
    }
}

/// Validate and decode a base36 character.
#[inline]
pub(crate) const fn validate_base36(b: u8, position: usize) -> Result<u8, ParseError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'z' => Ok(b - b'a' + 10),
        b'A'..=b'Z' => Ok(b - b'A' + 10),
        _ => Err(ParseError::invalid_char(position, b)),
    }
}
