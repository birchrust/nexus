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
//! let epoch = Instant::now();
//! let mut id_gen = ClOrdId::new(5, epoch);
//!
//! // In event loop - snap once per tick
//! let now = Instant::now();
//! let id: u64 = id_gen.next(now).unwrap();
//!
//! // Unpack to inspect
//! let (ts, worker, seq) = ClOrdId::unpack(id);
//! assert_eq!(worker, 5);
//! assert_eq!(seq, 0);
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

use core::marker::PhantomData;
use std::time::Instant;

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

/// Snowflake ID generator.
///
/// # Type Parameters
/// - `T`: Output integer type (`u32`, `i32`, `u64`, `i64`)
/// - `TS`: Timestamp bits
/// - `WK`: Worker bits (0 for single-worker systems)
/// - `SQ`: Sequence bits
///
/// # Layout (MSB to LSB)
/// ```text
/// [timestamp: TS bits][worker: WK bits][sequence: SQ bits]
/// ```
///
/// # Compile-Time Validation
/// - `TS + WK + SQ <= T::BITS`
/// - `TS > 0`
/// - `SQ > 0`
///
/// # Example
/// ```rust
/// use std::time::Instant;
/// use nexus_id::Snowflake64;
///
/// // 42 bits timestamp, 6 bits worker, 16 bits sequence
/// type TradingId = Snowflake64<42, 6, 16>;
///
/// let epoch = Instant::now();
/// let mut id_gen = TradingId::new(5, epoch);
///
/// let now = Instant::now();
/// let id: u64 = id_gen.next(now).unwrap();
/// ```
pub struct Snowflake<T: IdInt, const TS: u8, const WK: u8, const SQ: u8> {
    epoch: Instant,
    worker_shifted: u64,
    last_ts: u64,
    sequence: u64,
    _marker: PhantomData<T>,
}

impl<T: IdInt, const TS: u8, const WK: u8, const SQ: u8> Snowflake<T, TS, WK, SQ> {
    /// Compile-time layout validation.
    const _VALIDATE: () = {
        assert!(
            TS as u16 + WK as u16 + SQ as u16 <= T::BITS as u16,
            "layout exceeds integer bits: TS + WK + SQ > T::BITS"
        );
        assert!(TS > 0, "timestamp bits must be > 0");
        assert!(SQ > 0, "sequence bits must be > 0");
    };

    /// Shift amount for timestamp field.
    const TS_SHIFT: u8 = WK + SQ;

    /// Shift amount for worker field.
    const WK_SHIFT: u8 = SQ;

    /// Maximum sequence value for this layout.
    pub const SEQUENCE_MAX: u64 = (1u64 << SQ) - 1;

    /// Maximum worker value for this layout.
    pub const WORKER_MAX: u64 = if WK == 0 { 0 } else { (1u64 << WK) - 1 };

    /// Maximum timestamp value for this layout.
    pub const TIMESTAMP_MAX: u64 = (1u64 << TS) - 1;

    /// Create a new generator.
    ///
    /// # Arguments
    /// * `worker` - Worker ID, must fit in WK bits
    /// * `epoch` - Base instant for timestamp calculation. Timestamps in
    ///   generated IDs are milliseconds since this instant.
    ///
    /// # Panics
    /// Panics if `worker > WORKER_MAX`.
    ///
    /// # Example
    /// ```rust
    /// use std::time::Instant;
    /// use nexus_id::Snowflake64;
    ///
    /// type MyId = Snowflake64<42, 6, 16>;
    ///
    /// // Worker 5, epoch is now
    /// let mut id_gen = MyId::new(5, Instant::now());
    /// ```
    pub fn new(worker: u64, epoch: Instant) -> Self {
        // Trigger compile-time validation
        let () = Self::_VALIDATE;

        assert!(
            worker <= Self::WORKER_MAX,
            "worker {} exceeds max {}",
            worker,
            Self::WORKER_MAX
        );

        Self {
            epoch,
            worker_shifted: worker << Self::WK_SHIFT,
            last_ts: u64::MAX, // Ensures first call takes "new timestamp" branch
            sequence: 0,
            _marker: PhantomData,
        }
    }

    /// Generate the next ID.
    ///
    /// # Arguments
    /// * `now` - Current instant, snapped by caller. The generator computes
    ///   `now.duration_since(epoch).as_millis()` for the timestamp.
    ///
    /// # Errors
    /// Returns [`SequenceExhausted`] if more than `SEQUENCE_MAX` IDs are
    /// generated within the same millisecond. This is a backpressure signal.
    ///
    /// # Performance
    /// This method is designed for hot-path usage:
    /// - No syscalls (caller provides instant)
    /// - No allocation
    /// - Single predictable branch in the common case
    ///
    /// # Example
    /// ```rust
    /// use std::time::Instant;
    /// use nexus_id::Snowflake64;
    ///
    /// type MyId = Snowflake64<42, 6, 16>;
    ///
    /// let epoch = Instant::now();
    /// let mut id_gen = MyId::new(5, epoch);
    ///
    /// // Snap time once per event loop iteration
    /// let now = Instant::now();
    ///
    /// // Generate IDs using that instant
    /// let id1 = id_gen.next(now).unwrap();
    /// let id2 = id_gen.next(now).unwrap();
    /// ```
    pub fn next(&mut self, now: Instant) -> Result<T, SequenceExhausted> {
        // Note: We tested `(as_nanos() as u64) >> 20` to avoid division, but
        // as_millis() is already well-optimized (multiply-shift) and performed
        // identically at ~24-26 cycles. Keeping as_millis() for clarity.
        let ts = now.duration_since(self.epoch).as_millis() as u64;

        if ts == self.last_ts {
            self.sequence += 1;
            if self.sequence > Self::SEQUENCE_MAX {
                return Err(SequenceExhausted {
                    timestamp_ms: ts,
                    max_sequence: Self::SEQUENCE_MAX,
                });
            }
        } else {
            self.last_ts = ts;
            self.sequence = 0;
        }

        Ok(T::from_raw(
            (ts << Self::TS_SHIFT) | self.worker_shifted | self.sequence,
        ))
    }

    /// Generate a new ID with good hash distribution.
    ///
    /// This applies a Stafford mix (bijective permutation) to the raw Snowflake ID,
    /// producing output suitable for use with identity hashers. The mixing preserves
    /// uniqueness guarantees while providing uniform bit distribution.
    ///
    /// # When to Use
    ///
    /// Use `mixed()` instead of `next()` when:
    /// - You want to use identity hashers (e.g., `nohash`) for maximum HashMap performance
    /// - You don't need to extract timestamp/worker/sequence from the ID
    ///
    /// Use `next()` instead when:
    /// - You need to call `unpack()` on the ID later
    /// - You're using FxHash/AHash anyway (mixing would be redundant)
    ///
    /// # Performance
    ///
    /// Adds ~7 cycles to ID generation (~33-35 total), but saves ~20+ cycles per
    /// HashMap operation by enabling identity hashers.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::time::Instant;
    /// use nexus_id::Snowflake64;
    ///
    /// type OrderId = Snowflake64<42, 6, 16>;
    ///
    /// let epoch = Instant::now();
    /// let mut id_gen = OrderId::new(0, epoch);
    ///
    /// // Generate mixed IDs - safe with identity hasher
    /// let id = id_gen.mixed(Instant::now()).unwrap();
    /// ```
    pub fn mixed(&mut self, now: Instant) -> Result<T, SequenceExhausted> {
        let raw = self.next(now)?;
        Ok(T::from_raw(stafford_mix(raw.to_raw())))
    }

    /// Unpack an ID into (timestamp, worker, sequence).
    ///
    /// # Example
    /// ```rust
    /// use std::time::Instant;
    /// use nexus_id::Snowflake64;
    ///
    /// type MyId = Snowflake64<42, 6, 16>;
    ///
    /// let epoch = Instant::now();
    /// let mut id_gen = MyId::new(5, epoch);
    ///
    /// let id = id_gen.next(epoch).unwrap();
    /// let (ts, worker, seq) = MyId::unpack(id);
    ///
    /// assert_eq!(worker, 5);
    /// assert_eq!(seq, 0);
    /// ```
    pub fn unpack(id: T) -> (u64, u64, u64) {
        let raw = id.to_raw();
        (
            raw >> Self::TS_SHIFT,
            (raw >> Self::WK_SHIFT) & Self::WORKER_MAX,
            raw & Self::SEQUENCE_MAX,
        )
    }

    /// Epoch instant for this generator.
    #[inline]
    pub fn epoch(&self) -> Instant {
        self.epoch
    }

    /// Worker ID for this generator.
    #[inline]
    pub const fn worker(&self) -> u64 {
        self.worker_shifted >> Self::WK_SHIFT
    }

    /// Current sequence within this millisecond.
    #[inline]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Last timestamp used (ms since epoch).
    #[inline]
    pub const fn last_timestamp(&self) -> u64 {
        self.last_ts
    }
}

// Type aliases for convenience

/// 32-bit snowflake generator.
pub type Snowflake32<const TS: u8, const WK: u8, const SQ: u8> = Snowflake<u32, TS, WK, SQ>;

/// 64-bit snowflake generator.
pub type Snowflake64<const TS: u8, const WK: u8, const SQ: u8> = Snowflake<u64, TS, WK, SQ>;

/// Signed 32-bit snowflake generator.
pub type SnowflakeSigned32<const TS: u8, const WK: u8, const SQ: u8> = Snowflake<i32, TS, WK, SQ>;

/// Signed 64-bit snowflake generator.
pub type SnowflakeSigned64<const TS: u8, const WK: u8, const SQ: u8> = Snowflake<i64, TS, WK, SQ>;

/// Stafford mix function - bijective permutation with good avalanche.
///
/// This is a 1-round variant of the Murmur3 finalizer. It's fast (~7 cycles)
/// and provides sufficient bit mixing for hash table distribution.
#[inline(always)]
const fn stafford_mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x
}

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
        sorted.sort_unstable();
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
