//! Fixed-capacity ASCII strings for high-performance systems.
//!
//! This crate provides stack-allocated, fixed-capacity ASCII string types
//! optimized for trading systems and other latency-sensitive applications.
//!
//! # Design Principles
//!
//! - **Immutable**: Strings are immutable after creation. Hash is computed once.
//! - **Copy**: All string types are `Copy`. Use newtypes for move semantics.
//! - **Performance**: Single 64-bit comparison for equality fast path.
//! - **Full ASCII**: Supports 0x00-0x7F. Use `AsciiText` for printable-only.
//!
//! # Example
//!
//! ```
//! use nexus_ascii::{AsciiString, AsciiError};
//!
//! // Construction
//! let s: AsciiString<32> = AsciiString::try_from("BTC-USD")?;
//!
//! // Equality is fast (header comparison first)
//! let s2: AsciiString<32> = AsciiString::try_from("BTC-USD")?;
//! assert_eq!(s, s2);
//!
//! // Access underlying data
//! assert_eq!(s.as_str(), "BTC-USD");
//! assert_eq!(s.len(), 7);
//! # Ok::<(), AsciiError>(())
//! ```

mod char;
mod str_ref;
mod string;

pub mod hash;

pub use char::{AsciiChar, InvalidAsciiChar};
pub use str_ref::AsciiStr;
pub use string::AsciiString;

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur when constructing ASCII types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsciiError {
    /// Input exceeds the string's capacity.
    TooLong {
        /// Actual length of the input.
        len: usize,
        /// Maximum capacity of the target string.
        cap: usize,
    },
    /// Byte is not valid ASCII (value > 127).
    InvalidByte {
        /// The invalid byte value.
        byte: u8,
        /// Position in the input where the invalid byte was found.
        pos: usize,
    },
    /// Byte is not printable ASCII (< 32 or > 126). Used by `AsciiText`.
    NonPrintable {
        /// The non-printable byte value.
        byte: u8,
        /// Position in the input where the non-printable byte was found.
        pos: usize,
    },
}

impl core::fmt::Display for AsciiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AsciiError::TooLong { len, cap } => {
                write!(f, "input length {} exceeds capacity {}", len, cap)
            }
            AsciiError::InvalidByte { byte, pos } => {
                write!(f, "invalid ASCII byte 0x{:02X} at position {}", byte, pos)
            }
            AsciiError::NonPrintable { byte, pos } => {
                write!(
                    f,
                    "non-printable ASCII byte 0x{:02X} at position {}",
                    byte, pos
                )
            }
        }
    }
}

impl std::error::Error for AsciiError {}
