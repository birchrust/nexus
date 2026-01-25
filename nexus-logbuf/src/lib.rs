//! High-performance lock-free ring buffers for variable-length messages.
//!
//! This crate provides bounded SPSC and MPSC byte ring buffers optimized for
//! getting data off the hot path without disturbing it. No allocation, no
//! formatting, no syscalls on the producer side.
//!
//! # Design
//!
//! - **Flat byte buffer** with free-running offsets, power-of-2 capacity
//! - **len-as-commit**: Record's len field is the commit marker (non-zero = ready)
//! - **Skip markers**: High bit of len distinguishes padding/aborted claims
//! - **Consumer zeroing**: Consumer zeros records before releasing space
//! - **Claim-based API**: `WriteClaim`/`ReadClaim` with RAII semantics
//!
//! # Variants
//!
//! - [`spsc`]: Single-producer, single-consumer. No CAS on hot path.
//! - `mpsc`: Multi-producer, single-consumer. CAS on tail for claiming. (TODO)
//!
//! # Example
//!
//! ```
//! use nexus_logbuf::spsc;
//!
//! let (mut producer, mut consumer) = spsc::new(4096);
//!
//! // Producer (hot path)
//! let payload = b"hello world";
//! if let Ok(mut claim) = producer.try_claim(payload.len()) {
//!     claim.copy_from_slice(payload);
//!     claim.commit();
//! }
//!
//! // Consumer (background thread)
//! if let Some(record) = consumer.try_claim() {
//!     assert_eq!(&*record, b"hello world");
//!     // record dropped here -> zeros region, advances head
//! }
//! ```

pub mod mpsc;
pub mod spsc;

/// Error returned from [`spsc::Producer::try_claim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryClaimError {
    /// The buffer is full.
    Full,
    /// The payload length was zero.
    ZeroLength,
}

impl std::fmt::Display for TryClaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "buffer full"),
            Self::ZeroLength => write!(f, "payload length must be non-zero"),
        }
    }
}

impl std::error::Error for TryClaimError {}

/// Align a value up to the next multiple of 8.
#[inline]
const fn align8(n: usize) -> usize {
    (n + 7) & !7
}

/// Record header constants.
///
/// The len field uses the high bit as a skip marker:
/// - `len == 0`: Not committed, consumer waits
/// - `len > 0, high bit clear`: Committed record, payload is `len` bytes
/// - `len high bit set`: Skip marker, advance by `len & LEN_MASK` bytes
const SKIP_BIT: u32 = 0x8000_0000;
const LEN_MASK: u32 = 0x7FFF_FFFF;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align8_works() {
        assert_eq!(align8(0), 0);
        assert_eq!(align8(1), 8);
        assert_eq!(align8(7), 8);
        assert_eq!(align8(8), 8);
        assert_eq!(align8(9), 16);
        assert_eq!(align8(15), 16);
        assert_eq!(align8(16), 16);
    }
}
