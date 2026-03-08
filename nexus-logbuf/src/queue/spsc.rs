//! Single-producer single-consumer byte ring buffer.
//!
//! # Design
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │ Shared:                                                                 │
//! │   head: CachePadded<AtomicUsize>  ← Consumer writes, producer reads     │
//! │   buffer: *mut u8                                                       │
//! │   capacity: usize                 (power of 2)                          │
//! │   mask: usize                     (capacity - 1)                        │
//! └─────────────────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────┐   ┌─────────────────────────────────┐
//! │ Producer:                       │   │ Consumer:                       │
//! │   tail: usize        (local)    │   │   head: usize        (local)    │
//! │   cached_head: usize (local)    │   │                                 │
//! └─────────────────────────────────┘   └─────────────────────────────────┘
//! ```
//!
//! # Record Layout
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ len: u32                (4 bytes)            │ ← payload length / commit marker
//! ├──────────────────────────────────────────────┤
//! │ payload: [u8; len]      (variable)           │ ← raw bytes
//! ├──────────────────────────────────────────────┤
//! │ padding: [u8; ...]      (0-7 bytes)          │ ← align to 8-byte boundary
//! └──────────────────────────────────────────────┘
//! ```
//!
//! Records are packed contiguously. Total record size is `align8(4 + len)`.
//!
//! # Len Field Encoding
//!
//! - `len == 0`: Not committed, consumer waits
//! - `len > 0, high bit clear`: Committed record, payload is `len` bytes
//! - `len high bit set`: Skip marker, advance by `len & 0x7FFF_FFFF` bytes

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::ops::{Deref, DerefMut};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering, fence};

use crossbeam_utils::CachePadded;

use crate::{LEN_MASK, SKIP_BIT, TryClaimError, align8};

/// Header size in bytes.
const HEADER_SIZE: usize = 4;

/// Creates a bounded SPSC byte ring buffer.
///
/// Capacity is rounded up to the next power of two.
///
/// # Panics
///
/// Panics if `capacity` is zero or less than 16 bytes.
pub fn new(capacity: usize) -> (Producer, Consumer) {
    assert!(capacity >= 16, "capacity must be at least 16 bytes");

    let capacity = capacity.next_power_of_two();
    let mask = capacity - 1;

    // Allocate buffer, zero-initialized, 8-byte aligned for atomic len stamps
    let layout = Layout::from_size_align(capacity, 8).unwrap();
    let buffer_ptr = unsafe { alloc_zeroed(layout) };
    assert!(!buffer_ptr.is_null(), "allocation failed");

    let shared = Arc::new(Shared {
        head: CachePadded::new(AtomicUsize::new(0)),
        buffer: buffer_ptr,
        capacity,
        mask,
    });

    (
        Producer {
            tail: 0,
            cached_head: 0,
            shared: Arc::clone(&shared),
        },
        Consumer { head: 0, shared },
    )
}

struct Shared {
    /// Consumer's read position. Updated by consumer, read by producer.
    head: CachePadded<AtomicUsize>,
    /// Buffer pointer.
    buffer: *mut u8,
    /// Buffer capacity (power of 2).
    capacity: usize,
    /// Mask for wrapping (capacity - 1).
    mask: usize,
}

// Safety: Buffer is only accessed by one producer and one consumer.
// The atomic head provides synchronization.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Drop for Shared {
    fn drop(&mut self) {
        // Safety: buffer was allocated with alloc_zeroed using this layout.
        let layout = Layout::from_size_align(self.capacity, 8).unwrap();
        unsafe { dealloc(self.buffer, layout) };
    }
}

// ============================================================================
// Producer
// ============================================================================

/// Producer endpoint of the SPSC ring buffer.
///
/// Use [`try_claim`](Producer::try_claim) to claim space for writing.
pub struct Producer {
    /// Local tail position (free-running).
    tail: usize,
    /// Cached head position (Rigtorp optimization).
    cached_head: usize,
    /// Shared state.
    shared: Arc<Shared>,
}

// Safety: Producer is only used from one thread.
unsafe impl Send for Producer {}

impl Producer {
    /// Attempts to claim space for a record with the given payload length.
    ///
    /// Returns a [`WriteClaim`] that can be written to and then committed.
    ///
    /// # Errors
    ///
    /// - [`TryClaimError::ZeroLength`] if `len` is zero
    /// - [`TryClaimError::Full`] if the buffer is full
    ///
    /// # Safety Contract
    ///
    /// `len` must not exceed `0x7FFF_FFFF` (2GB - 1). This is checked with
    /// `debug_assert!` only.
    #[inline]
    pub fn try_claim(&mut self, len: usize) -> Result<WriteClaim<'_>, TryClaimError> {
        debug_assert!(len <= LEN_MASK as usize, "payload too large");
        if len == 0 {
            return Err(TryClaimError::ZeroLength);
        }

        let record_size = align8(HEADER_SIZE + len);

        // Check if we have space
        let tail = self.tail;
        let available = self.shared.capacity - (tail.wrapping_sub(self.cached_head));

        if available < record_size {
            // Reload head from shared state
            self.cached_head = self.shared.head.load(Ordering::Relaxed);
            fence(Ordering::Acquire);

            let available = self.shared.capacity - (tail.wrapping_sub(self.cached_head));
            if available < record_size {
                return Err(TryClaimError::Full);
            }
        }

        // Check if record fits before buffer end, or needs wrap
        let offset = tail & self.shared.mask;
        let space_to_end = self.shared.capacity - offset;

        if space_to_end < record_size {
            // Need to wrap. First check if we have space for padding + record at start.
            let total_needed = space_to_end + record_size;
            let available = self.shared.capacity - (tail.wrapping_sub(self.cached_head));

            if available < total_needed {
                // Reload and recheck
                self.cached_head = self.shared.head.load(Ordering::Relaxed);
                fence(Ordering::Acquire);

                let available = self.shared.capacity - (tail.wrapping_sub(self.cached_head));
                if available < total_needed {
                    return Err(TryClaimError::Full);
                }
            }

            // Write padding skip marker
            let buffer = self.shared.buffer;
            let skip_len = space_to_end as u32 | SKIP_BIT;
            fence(Ordering::Release);
            let len_ptr = unsafe { buffer.add(offset) }.cast::<AtomicU32>();
            unsafe { &*len_ptr }.store(skip_len, Ordering::Relaxed);

            // Advance tail past padding
            self.tail = tail.wrapping_add(space_to_end);
            let new_offset = 0;

            Ok(WriteClaim {
                producer: self,
                offset: new_offset,
                len,
                record_size,
                committed: false,
            })
        } else {
            // Fits without wrapping
            Ok(WriteClaim {
                producer: self,
                offset,
                len,
                record_size,
                committed: false,
            })
        }
    }

    /// Returns the capacity of the buffer.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.shared.capacity
    }

    /// Returns `true` if the consumer has been dropped.
    #[inline]
    pub fn is_disconnected(&self) -> bool {
        Arc::strong_count(&self.shared) == 1
    }
}

impl std::fmt::Debug for Producer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Producer")
            .field("capacity", &self.capacity())
            .finish_non_exhaustive()
    }
}

// ============================================================================
// WriteClaim
// ============================================================================

/// A claimed region for writing a record.
///
/// Dereferences to `&mut [u8]` for the payload region. Call [`commit`](WriteClaim::commit)
/// when done writing to publish the record. If dropped without committing, a skip
/// marker is written so the consumer can advance past the dead region.
pub struct WriteClaim<'a> {
    producer: &'a mut Producer,
    offset: usize,
    len: usize,
    record_size: usize,
    committed: bool,
}

impl WriteClaim<'_> {
    /// Commits the record, making it visible to the consumer.
    #[inline]
    pub fn commit(mut self) {
        self.do_commit();
        self.committed = true;
    }

    #[inline]
    fn do_commit(&mut self) {
        let buffer = self.producer.shared.buffer;
        let len_ptr = unsafe { buffer.add(self.offset) }.cast::<AtomicU32>();

        // Release fence: ensures payload writes are visible before len store
        fence(Ordering::Release);
        unsafe { &*len_ptr }.store(self.len as u32, Ordering::Relaxed);

        // Advance tail
        self.producer.tail = self.producer.tail.wrapping_add(self.record_size);
    }

    /// Returns the length of the payload region.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the payload is empty (always false, len must be > 0).
    #[inline]
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl Deref for WriteClaim<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        let buffer = self.producer.shared.buffer;
        let payload_ptr = unsafe { buffer.add(self.offset + HEADER_SIZE) };
        unsafe { std::slice::from_raw_parts(payload_ptr, self.len) }
    }
}

impl DerefMut for WriteClaim<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        let buffer = self.producer.shared.buffer;
        let payload_ptr = unsafe { buffer.add(self.offset + HEADER_SIZE) };
        unsafe { std::slice::from_raw_parts_mut(payload_ptr, self.len) }
    }
}

impl Drop for WriteClaim<'_> {
    fn drop(&mut self) {
        if !self.committed {
            // Write skip marker so consumer can advance past this region
            let buffer = self.producer.shared.buffer;
            let len_ptr = unsafe { buffer.add(self.offset) }.cast::<AtomicU32>();
            let skip_len = self.record_size as u32 | SKIP_BIT;

            fence(Ordering::Release);
            unsafe { &*len_ptr }.store(skip_len, Ordering::Relaxed);

            // Advance tail past the dead region
            self.producer.tail = self.producer.tail.wrapping_add(self.record_size);
        }
    }
}

// ============================================================================
// Consumer
// ============================================================================

/// Consumer endpoint of the SPSC ring buffer.
///
/// Use [`try_claim`](Consumer::try_claim) to claim the next record for reading.
pub struct Consumer {
    /// Local head position (free-running).
    head: usize,
    /// Shared state.
    shared: Arc<Shared>,
}

// Safety: Consumer is only used from one thread.
unsafe impl Send for Consumer {}

impl Consumer {
    /// Attempts to claim the next record for reading.
    ///
    /// Returns a [`ReadClaim`] if a record is available. The claim dereferences
    /// to `&[u8]` for the payload. When dropped, the record region is zeroed
    /// and the head is advanced.
    ///
    /// Returns `None` if no committed record is available.
    #[inline]
    pub fn try_claim(&mut self) -> Option<ReadClaim<'_>> {
        let buffer = self.shared.buffer;

        loop {
            let offset = self.head & self.shared.mask;
            let len_ptr = unsafe { buffer.add(offset) }.cast::<AtomicU32>();

            // Relaxed atomic load, then Acquire fence for payload visibility
            let len_raw = unsafe { &*len_ptr }.load(Ordering::Relaxed);
            fence(Ordering::Acquire);

            if len_raw == 0 {
                // Not committed yet
                return None;
            }

            if len_raw & SKIP_BIT != 0 {
                // Skip marker: zero the region and advance
                let skip_size = (len_raw & LEN_MASK) as usize;
                // Zero payload first, then stamp last (mirrors write path)
                if skip_size > HEADER_SIZE {
                    unsafe {
                        ptr::write_bytes(buffer.add(offset + HEADER_SIZE), 0, skip_size - HEADER_SIZE);
                    }
                }
                // Ensure payload zeroing completes before clearing stamp
                fence(Ordering::Release);
                unsafe { &*len_ptr }.store(0, Ordering::Relaxed);

                self.head = self.head.wrapping_add(skip_size);

                // Ensure stamp clear completes before head advance
                fence(Ordering::Release);
                self.shared.head.store(self.head, Ordering::Relaxed);

                // Continue to check next position
                continue;
            }

            // Valid record
            let len = len_raw as usize;
            let record_size = align8(HEADER_SIZE + len);

            return Some(ReadClaim {
                consumer: self,
                offset,
                len,
                record_size,
            });
        }
    }

    /// Returns the capacity of the buffer.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.shared.capacity
    }

    /// Returns `true` if the producer has been dropped.
    #[inline]
    pub fn is_disconnected(&self) -> bool {
        Arc::strong_count(&self.shared) == 1
    }
}

impl std::fmt::Debug for Consumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Consumer")
            .field("capacity", &self.capacity())
            .finish_non_exhaustive()
    }
}

// ============================================================================
// ReadClaim
// ============================================================================

/// A claimed record for reading.
///
/// Dereferences to `&[u8]` for the payload. When dropped, the record region
/// is zeroed and the head is advanced, freeing space for the producer.
pub struct ReadClaim<'a> {
    consumer: &'a mut Consumer,
    offset: usize,
    len: usize,
    record_size: usize,
}

impl ReadClaim<'_> {
    /// Returns the length of the payload.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the payload is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Deref for ReadClaim<'_> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        let buffer = self.consumer.shared.buffer;
        let payload_ptr = unsafe { buffer.add(self.offset + HEADER_SIZE) };
        unsafe { std::slice::from_raw_parts(payload_ptr, self.len) }
    }
}

impl Drop for ReadClaim<'_> {
    fn drop(&mut self) {
        let buffer = self.consumer.shared.buffer;

        // Zero payload first, then stamp last (mirrors write path)
        if self.record_size > HEADER_SIZE {
            unsafe {
                ptr::write_bytes(buffer.add(self.offset + HEADER_SIZE), 0, self.record_size - HEADER_SIZE);
            }
        }
        // Ensure payload zeroing completes before clearing stamp
        fence(Ordering::Release);
        let len_ptr = unsafe { buffer.add(self.offset) }.cast::<AtomicU32>();
        unsafe { &*len_ptr }.store(0, Ordering::Relaxed);

        // Advance head
        self.consumer.head = self.consumer.head.wrapping_add(self.record_size);

        // Ensure stamp clear completes before head advance
        fence(Ordering::Release);
        self.consumer
            .shared
            .head
            .store(self.consumer.head, Ordering::Relaxed);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_write_read() {
        let (mut prod, mut cons) = new(1024);

        let payload = b"hello world";
        let mut claim = prod.try_claim(payload.len()).unwrap();
        claim.copy_from_slice(payload);
        claim.commit();

        let record = cons.try_claim().unwrap();
        assert_eq!(&*record, payload);
    }

    #[test]
    fn empty_returns_none() {
        let (_, mut cons) = new(1024);
        assert!(cons.try_claim().is_none());
    }

    #[test]
    fn multiple_records() {
        let (mut prod, mut cons) = new(1024);

        for i in 0..10 {
            let payload = format!("message {}", i);
            let mut claim = prod.try_claim(payload.len()).unwrap();
            claim.copy_from_slice(payload.as_bytes());
            claim.commit();
        }

        for i in 0..10 {
            let record = cons.try_claim().unwrap();
            let expected = format!("message {}", i);
            assert_eq!(&*record, expected.as_bytes());
        }

        assert!(cons.try_claim().is_none());
    }

    #[test]
    fn aborted_claim_creates_skip() {
        let (mut prod, mut cons) = new(1024);

        // Claim and drop without committing
        {
            let mut claim = prod.try_claim(10).unwrap();
            claim.copy_from_slice(b"0123456789");
            // drop without commit
        }

        // Write another record
        {
            let mut claim = prod.try_claim(5).unwrap();
            claim.copy_from_slice(b"hello");
            claim.commit();
        }

        // Consumer should skip the aborted record and read the committed one
        let record = cons.try_claim().unwrap();
        assert_eq!(&*record, b"hello");
    }

    #[test]
    fn wrap_around() {
        let (mut prod, mut cons) = new(64);

        // Fill with messages that will cause wrap-around
        for i in 0..20 {
            let payload = format!("msg{:02}", i);
            loop {
                match prod.try_claim(payload.len()) {
                    Ok(mut claim) => {
                        claim.copy_from_slice(payload.as_bytes());
                        claim.commit();
                        break;
                    }
                    Err(_) => {
                        // Drain some
                        while cons.try_claim().is_some() {}
                    }
                }
            }
        }
    }

    #[test]
    fn full_returns_error() {
        let (mut prod, _cons) = new(64);

        // Fill the buffer
        let mut count = 0;
        loop {
            match prod.try_claim(8) {
                Ok(mut claim) => {
                    claim.copy_from_slice(b"12345678");
                    claim.commit();
                    count += 1;
                }
                Err(_) => break,
            }
        }

        assert!(count > 0);
        assert!(prod.try_claim(8).is_err());
    }

    #[test]
    fn cross_thread() {
        use std::thread;

        let (mut prod, mut cons) = new(4096);

        let producer = thread::spawn(move || {
            for i in 0..10_000u64 {
                let payload = i.to_le_bytes();
                loop {
                    match prod.try_claim(payload.len()) {
                        Ok(mut claim) => {
                            claim.copy_from_slice(&payload);
                            claim.commit();
                            break;
                        }
                        Err(_) => std::hint::spin_loop(),
                    }
                }
            }
        });

        let consumer = thread::spawn(move || {
            let mut received = 0u64;
            while received < 10_000 {
                if let Some(record) = cons.try_claim() {
                    let value = u64::from_le_bytes((*record).try_into().unwrap());
                    assert_eq!(value, received);
                    received += 1;
                } else {
                    std::hint::spin_loop();
                }
            }
        });

        producer.join().unwrap();
        consumer.join().unwrap();
    }

    #[test]
    fn disconnection_detection() {
        let (prod, cons) = new(1024);

        assert!(!prod.is_disconnected());
        assert!(!cons.is_disconnected());

        drop(cons);
        assert!(prod.is_disconnected());
    }

    #[test]
    #[should_panic(expected = "capacity must be at least 16")]
    fn tiny_capacity_panics() {
        let _ = new(8);
    }

    #[test]
    fn zero_len_returns_error() {
        let (mut prod, _) = new(1024);
        assert!(matches!(prod.try_claim(0), Err(TryClaimError::ZeroLength)));
    }

    #[test]
    fn capacity_rounds_to_power_of_two() {
        let (prod, _) = new(100);
        assert_eq!(prod.capacity(), 128);

        let (prod, _) = new(1000);
        assert_eq!(prod.capacity(), 1024);
    }

    #[test]
    fn variable_length_records() {
        let (mut prod, mut cons) = new(4096);

        let messages = [
            "a",
            "hello",
            "this is a longer message",
            "x",
            "medium length",
        ];

        for msg in &messages {
            let mut claim = prod.try_claim(msg.len()).unwrap();
            claim.copy_from_slice(msg.as_bytes());
            claim.commit();
        }

        for msg in &messages {
            let record = cons.try_claim().unwrap();
            assert_eq!(&*record, msg.as_bytes());
        }
    }

    /// High-volume stress test with variable-length messages.
    ///
    /// Tests correctness under sustained load with wrap-around.
    #[test]
    fn stress_high_volume() {
        use std::thread;

        const COUNT: u64 = 1_000_000;
        const BUFFER_SIZE: usize = 64 * 1024; // 64KB

        let (mut prod, mut cons) = new(BUFFER_SIZE);

        let producer = thread::spawn(move || {
            for i in 0..COUNT {
                // Variable length: 8-64 bytes based on sequence
                let len = 8 + ((i % 8) * 8) as usize;
                let mut payload = vec![0u8; len];
                // Write sequence number at start
                payload[..8].copy_from_slice(&i.to_le_bytes());

                loop {
                    match prod.try_claim(len) {
                        Ok(mut claim) => {
                            claim.copy_from_slice(&payload);
                            claim.commit();
                            break;
                        }
                        Err(_) => std::hint::spin_loop(),
                    }
                }
            }
        });

        let consumer = thread::spawn(move || {
            let mut received = 0u64;
            while received < COUNT {
                if let Some(record) = cons.try_claim() {
                    // Verify sequence number
                    let seq = u64::from_le_bytes(record[..8].try_into().unwrap());
                    assert_eq!(seq, received, "sequence mismatch at {}", received);

                    // Verify expected length
                    let expected_len = 8 + ((received % 8) * 8) as usize;
                    assert_eq!(
                        record.len(),
                        expected_len,
                        "length mismatch at {}",
                        received
                    );

                    received += 1;
                } else {
                    std::hint::spin_loop();
                }
            }
            received
        });

        producer.join().unwrap();
        let received = consumer.join().unwrap();
        assert_eq!(received, COUNT);
    }

    /// Stress test with maximum contention - tiny buffer, high throughput.
    #[test]
    fn stress_high_contention() {
        use std::thread;

        const COUNT: u64 = 100_000;
        const BUFFER_SIZE: usize = 256; // Tiny buffer forces constant wrap-around

        let (mut prod, mut cons) = new(BUFFER_SIZE);

        let producer = thread::spawn(move || {
            for i in 0..COUNT {
                let payload = i.to_le_bytes();
                loop {
                    match prod.try_claim(payload.len()) {
                        Ok(mut claim) => {
                            claim.copy_from_slice(&payload);
                            claim.commit();
                            break;
                        }
                        Err(_) => std::hint::spin_loop(),
                    }
                }
            }
        });

        let consumer = thread::spawn(move || {
            let mut received = 0u64;
            let mut sum = 0u64;
            while received < COUNT {
                if let Some(record) = cons.try_claim() {
                    let value = u64::from_le_bytes((*record).try_into().unwrap());
                    assert_eq!(value, received);
                    sum = sum.wrapping_add(value);
                    received += 1;
                } else {
                    std::hint::spin_loop();
                }
            }
            sum
        });

        producer.join().unwrap();
        let sum = consumer.join().unwrap();
        // Sum of 0..COUNT = COUNT * (COUNT-1) / 2
        let expected = COUNT * (COUNT - 1) / 2;
        assert_eq!(sum, expected);
    }
}
