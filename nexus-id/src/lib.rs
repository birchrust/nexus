//! Snowflake-style ID generation with const generic bit layouts.
//!
//! # Overview
//!
//! `nexus-id` provides high-performance snowflake ID generation with
//! user-configurable bit layouts via const generics. No heap allocation,
//! no dependencies, optimized for trading systems where ID generation
//! is on the critical path.
//!
//! # Example
//!
//! ```rust
//! use std::time::Instant;
//! use nexus_id::Snowflake64;
//!
//! // Layout: 42 bits timestamp, 6 bits worker, 16 bits sequence (65k/ms)
//! type ClOrdId = Snowflake64<42, 6, 16>;
//!
//! fn main() {
//!     let epoch = Instant::now();
//!     let mut id_gen = ClOrdId::new(5, epoch);
//!
//!     // In event loop - snap once per tick
//!     let now = Instant::now();
//!     let id: u64 = id_gen.next(now).unwrap();
//!
//!     // Unpack to inspect
//!     let (ts, worker, seq) = ClOrdId::unpack(id);
//!     assert_eq!(worker, 5);
//!     assert_eq!(seq, 0);
//! }
//! ```
//!
//! # Bit Layout
//!
//! IDs are packed MSB to LSB as:
//!
//! ```text
//! [timestamp: TS bits][worker: WK bits][sequence: SQ bits]
//! ```
//!
//! Common configurations:
//!
//! | TS | WK | SQ | Years @ ms | Workers | IDs/ms |
//! |----|----|----|------------|---------|--------|
//! | 41 | 10 | 12 | 69         | 1024    | 4k     |
//! | 42 | 6  | 16 | 139        | 64      | 65k    |
//! | 39 | 9  | 16 | 17         | 512     | 65k    |
//!
//! # Performance
//!
//! The `next()` method is designed for hot-path usage:
//! - No syscalls (caller provides instant)
//! - No allocation
//! - Predictable branches (same-ms case is common)
//! - ~24-26 cycles p50 (~6ns at 4GHz)

mod snowflake;

pub use snowflake::{Snowflake, Snowflake32, Snowflake64, SnowflakeSigned32, SnowflakeSigned64};

use core::fmt;

/// Integer types usable as snowflake IDs.
///
/// Implemented for `u32`, `i32`, `u64`, `i64`.
///
/// This trait is sealed and cannot be implemented outside this crate.
pub trait IdInt: Copy + private::Sealed {
    /// Number of bits in this integer type.
    const BITS: u8;

    /// Convert from raw u64 representation.
    fn from_raw(v: u64) -> Self;

    /// Convert to raw u64 representation.
    fn to_raw(self) -> u64;
}

mod private {
    pub trait Sealed {}
    impl Sealed for u32 {}
    impl Sealed for i32 {}
    impl Sealed for u64 {}
    impl Sealed for i64 {}
}

macro_rules! impl_id_int {
    ($($ty:ty => $bits:expr),* $(,)?) => {
        $(
            impl IdInt for $ty {
                const BITS: u8 = $bits;

                #[inline(always)]
                fn from_raw(v: u64) -> Self {
                    v as Self
                }

                #[inline(always)]
                fn to_raw(self) -> u64 {
                    self as u64
                }
            }
        )*
    };
}

impl_id_int!(
    u32 => 32,
    i32 => 32,
    u64 => 64,
    i64 => 64,
);

/// Sequence exhausted within current millisecond.
///
/// This is a backpressure signal — the caller generated more IDs
/// in a single millisecond than the sequence bits allow.
///
/// # Handling
///
/// When this error occurs, the caller should either:
/// - Reject the request (backpressure)
/// - Wait for the next millisecond
/// - Log and investigate (misconfigured sequence bits?)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceExhausted {
    /// Timestamp (ms since epoch) when exhaustion occurred.
    pub timestamp_ms: u64,
    /// Maximum sequence value for this generator's layout.
    pub max_sequence: u64,
}

impl fmt::Display for SequenceExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "sequence exhausted at timestamp {}: generated {} IDs in 1ms",
            self.timestamp_ms,
            self.max_sequence + 1
        )
    }
}

impl std::error::Error for SequenceExhausted {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    type TestId = Snowflake64<42, 6, 16>;

    #[test]
    fn basic_generation() {
        let epoch = Instant::now();
        let mut id_gen = TestId::new(5, epoch);

        let id = id_gen.next(epoch).unwrap();
        let (ts, worker, seq) = TestId::unpack(id);

        assert_eq!(ts, 0);
        assert_eq!(worker, 5);
        assert_eq!(seq, 0);
    }

    #[test]
    fn sequence_increments_same_ms() {
        let epoch = Instant::now();
        let mut id_gen = TestId::new(5, epoch);

        let id1 = id_gen.next(epoch).unwrap();
        let id2 = id_gen.next(epoch).unwrap();
        let id3 = id_gen.next(epoch).unwrap();

        let (_, _, seq1) = TestId::unpack(id1);
        let (_, _, seq2) = TestId::unpack(id2);
        let (_, _, seq3) = TestId::unpack(id3);

        assert_eq!(seq1, 0);
        assert_eq!(seq2, 1);
        assert_eq!(seq3, 2);
    }

    #[test]
    fn sequence_resets_new_ms() {
        let epoch = Instant::now();
        let mut id_gen = TestId::new(5, epoch);

        // Generate at epoch
        let _ = id_gen.next(epoch).unwrap();
        let _ = id_gen.next(epoch).unwrap();

        // Jump forward 1ms
        let later = epoch + Duration::from_millis(1);
        let id = id_gen.next(later).unwrap();

        let (ts, _, seq) = TestId::unpack(id);
        assert_eq!(ts, 1);
        assert_eq!(seq, 0);
    }

    #[test]
    fn worker_encoded_correctly() {
        let epoch = Instant::now();

        for worker in [0, 1, 31, 63] {
            let mut id_gen = TestId::new(worker, epoch);
            let id = id_gen.next(epoch).unwrap();
            let (_, w, _) = TestId::unpack(id);
            assert_eq!(w, worker);
        }
    }

    #[test]
    fn ids_are_unique() {
        let epoch = Instant::now();
        let mut id_gen = TestId::new(5, epoch);

        let mut ids = Vec::new();
        for i in 0..1000 {
            let now = epoch + Duration::from_micros(i * 100);
            ids.push(id_gen.next(now).unwrap());
        }

        // Check uniqueness
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn sequence_exhaustion() {
        // Tiny sequence: 4 bits = max 15
        type TinySeq = Snowflake64<42, 6, 4>;

        let epoch = Instant::now();
        let mut id_gen = TinySeq::new(5, epoch);

        // Generate 16 IDs (0-15)
        for _ in 0..16 {
            id_gen.next(epoch).unwrap();
        }

        // 17th should fail
        let result = id_gen.next(epoch);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.max_sequence, 15);
    }

    #[test]
    #[should_panic(expected = "worker 100 exceeds max 63")]
    fn worker_overflow_panics() {
        let epoch = Instant::now();
        let _id_gen = TestId::new(100, epoch); // 6 bits = max 63
    }

    #[test]
    fn signed_output() {
        type SignedId = SnowflakeSigned64<42, 6, 16>;

        let epoch = Instant::now();
        let mut id_gen = SignedId::new(5, epoch);

        let id: i64 = id_gen.next(epoch).unwrap();
        let (ts, worker, seq) = SignedId::unpack(id);

        assert_eq!(ts, 0);
        assert_eq!(worker, 5);
        assert_eq!(seq, 0);
    }

    #[test]
    fn small_layout_32bit() {
        // 20 bits timestamp, 4 bits worker, 8 bits sequence
        type SmallId = Snowflake32<20, 4, 8>;

        let epoch = Instant::now();
        let mut id_gen = SmallId::new(7, epoch);

        let id: u32 = id_gen.next(epoch).unwrap();
        let (ts, worker, seq) = SmallId::unpack(id);

        assert_eq!(ts, 0);
        assert_eq!(worker, 7);
        assert_eq!(seq, 0);
    }

    #[test]
    fn zero_worker_bits() {
        // Single-worker system: no worker bits
        type SingleWorker = Snowflake64<48, 0, 16>;

        let epoch = Instant::now();
        let mut id_gen = SingleWorker::new(0, epoch);

        let id = id_gen.next(epoch).unwrap();
        let (ts, worker, seq) = SingleWorker::unpack(id);

        assert_eq!(ts, 0);
        assert_eq!(worker, 0);
        assert_eq!(seq, 0);
    }
}
