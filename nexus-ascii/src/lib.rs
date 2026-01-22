//! Fixed-capacity ASCII strings for high-performance systems.
//!
//! This crate provides stack-allocated, fixed-capacity ASCII string types
//! optimized for trading systems and other latency-sensitive applications.
//!
//! # `no_std` Support
//!
//! This crate is `no_std` compatible by default. Enable the `std` feature
//! for `Error` trait implementations.
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

#![cfg_attr(not(any(feature = "std", test)), no_std)]

mod builder;
mod char;
mod str_ref;
mod string;
mod text;
mod text_ref;

pub mod hash;

pub use builder::AsciiStringBuilder;
pub use char::{AsciiChar, InvalidAsciiChar};
pub use str_ref::AsciiStr;
pub use string::AsciiString;
pub use text::AsciiText;
pub use text_ref::AsciiTextStr;

// =============================================================================
// Type Aliases
// =============================================================================

/// 8-byte capacity ASCII string.
pub type AsciiString8 = AsciiString<8>;
/// 16-byte capacity ASCII string.
pub type AsciiString16 = AsciiString<16>;
/// 32-byte capacity ASCII string.
pub type AsciiString32 = AsciiString<32>;
/// 64-byte capacity ASCII string.
pub type AsciiString64 = AsciiString<64>;
/// 128-byte capacity ASCII string.
pub type AsciiString128 = AsciiString<128>;
/// 256-byte capacity ASCII string.
pub type AsciiString256 = AsciiString<256>;

/// 8-byte capacity printable ASCII text.
pub type AsciiText8 = AsciiText<8>;
/// 16-byte capacity printable ASCII text.
pub type AsciiText16 = AsciiText<16>;
/// 32-byte capacity printable ASCII text.
pub type AsciiText32 = AsciiText<32>;
/// 64-byte capacity printable ASCII text.
pub type AsciiText64 = AsciiText<64>;
/// 128-byte capacity printable ASCII text.
pub type AsciiText128 = AsciiText<128>;

/// 8-byte capacity ASCII string builder.
pub type AsciiStringBuilder8 = AsciiStringBuilder<8>;
/// 16-byte capacity ASCII string builder.
pub type AsciiStringBuilder16 = AsciiStringBuilder<16>;
/// 32-byte capacity ASCII string builder.
pub type AsciiStringBuilder32 = AsciiStringBuilder<32>;
/// 64-byte capacity ASCII string builder.
pub type AsciiStringBuilder64 = AsciiStringBuilder<64>;
/// 128-byte capacity ASCII string builder.
pub type AsciiStringBuilder128 = AsciiStringBuilder<128>;

// =============================================================================
// NoHash Support (feature-gated)
// =============================================================================

// AsciiString and AsciiText store a precomputed 48-bit XXH3 hash in their header.
// This makes them ideal candidates for identity hashing with nohash-hasher,
// avoiding redundant hash computation in HashMap/HashSet lookups.

#[cfg(feature = "nohash")]
impl<const CAP: usize> nohash_hasher::IsEnabled for AsciiString<CAP> {}
#[cfg(feature = "nohash")]
impl<const CAP: usize> nohash_hasher::IsEnabled for AsciiText<CAP> {}

/// A `HashMap` using `AsciiString` keys with identity hashing.
///
/// Since `AsciiString` stores a precomputed hash in its header, this avoids
/// the overhead of SipHash or other hash functions during lookup.
///
/// # Example
///
/// ```
/// use nexus_ascii::{AsciiString, AsciiHashMap};
///
/// let mut map: AsciiHashMap<32, u64> = AsciiHashMap::default();
/// let key: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
/// map.insert(key, 42);
/// assert_eq!(map.get(&key), Some(&42));
/// ```
#[cfg(feature = "nohash")]
pub type AsciiHashMap<const CAP: usize, V> =
    std::collections::HashMap<AsciiString<CAP>, V, nohash_hasher::BuildNoHashHasher<u64>>;

/// A `HashSet` using `AsciiString` values with identity hashing.
///
/// Since `AsciiString` stores a precomputed hash in its header, this avoids
/// the overhead of SipHash or other hash functions during lookup.
///
/// # Example
///
/// ```
/// use nexus_ascii::{AsciiString, AsciiHashSet};
///
/// let mut set: AsciiHashSet<32> = AsciiHashSet::default();
/// let key: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
/// set.insert(key);
/// assert!(set.contains(&key));
/// ```
#[cfg(feature = "nohash")]
pub type AsciiHashSet<const CAP: usize> =
    std::collections::HashSet<AsciiString<CAP>, nohash_hasher::BuildNoHashHasher<u64>>;

/// A `HashMap` using `AsciiText` keys with identity hashing.
#[cfg(feature = "nohash")]
pub type AsciiTextHashMap<const CAP: usize, V> =
    std::collections::HashMap<AsciiText<CAP>, V, nohash_hasher::BuildNoHashHasher<u64>>;

/// A `HashSet` using `AsciiText` values with identity hashing.
#[cfg(feature = "nohash")]
pub type AsciiTextHashSet<const CAP: usize> =
    std::collections::HashSet<AsciiText<CAP>, nohash_hasher::BuildNoHashHasher<u64>>;

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

#[cfg(feature = "std")]
impl std::error::Error for AsciiError {}
