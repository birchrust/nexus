//! Snowflake ID generator.

use core::marker::PhantomData;
use std::time::Instant;

use crate::{IdInt, SequenceExhausted};

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
/// fn main() {
///     let epoch = Instant::now();
///     let mut id_gen = TradingId::new(5, epoch);
///
///     let now = Instant::now();
///     let id: u64 = id_gen.next(now).unwrap();
/// }
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
    /// fn main() {
    ///     // Worker 5, epoch is now
    ///     let mut id_gen = MyId::new(5, Instant::now());
    /// }
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
    /// fn main() {
    ///     let epoch = Instant::now();
    ///     let mut id_gen = MyId::new(5, epoch);
    ///
    ///     // Snap time once per event loop iteration
    ///     let now = Instant::now();
    ///
    ///     // Generate IDs using that instant
    ///     let id1 = id_gen.next(now).unwrap();
    ///     let id2 = id_gen.next(now).unwrap();
    /// }
    /// ```
    #[inline(always)]
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

    /// Unpack an ID into (timestamp, worker, sequence).
    ///
    /// # Example
    /// ```rust
    /// use std::time::Instant;
    /// use nexus_id::Snowflake64;
    ///
    /// type MyId = Snowflake64<42, 6, 16>;
    ///
    /// fn main() {
    ///     let epoch = Instant::now();
    ///     let mut id_gen = MyId::new(5, epoch);
    ///
    ///     let id = id_gen.next(epoch).unwrap();
    ///     let (ts, worker, seq) = MyId::unpack(id);
    ///
    ///     assert_eq!(worker, 5);
    ///     assert_eq!(seq, 0);
    /// }
    /// ```
    #[inline(always)]
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
