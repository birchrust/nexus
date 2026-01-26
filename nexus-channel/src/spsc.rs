//! Single-producer single-consumer bounded channel.
//!
//! A blocking channel wrapping [`nexus_queue::spsc`] with an optimized parking
//! strategy that minimizes syscall overhead.
//!
//! # Example
//!
//! ```
//! use nexus_channel::spsc;
//!
//! let (mut tx, mut rx) = spsc::channel::<u64>(1024);
//!
//! tx.send(42).unwrap();
//! assert_eq!(rx.recv().unwrap(), 42);
//! ```

use core::fmt;
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_utils::sync::{Parker, Unparker};
use crossbeam_utils::{Backoff, CachePadded};
use nexus_queue::Full;
use nexus_queue::spsc::{Consumer, Producer, ring_buffer};

use crate::{RecvError, RecvTimeoutError, SendError, TryRecvError, TrySendError};

/// Default number of backoff snooze iterations before parking.
const DEFAULT_SNOOZE_ITERS: usize = 8;

/// Shared state between sender and receiver.
struct Shared {
    sender_parked: CachePadded<AtomicBool>,
    receiver_parked: CachePadded<AtomicBool>,
}

/// Creates a bounded SPSC channel with the given capacity.
///
/// Returns a `(Sender, Receiver)` pair. The actual capacity will be rounded
/// up to the next power of two.
///
/// # Panics
///
/// Panics if `capacity` is 0.
///
/// # Example
///
/// ```
/// use nexus_channel::spsc;
///
/// let (mut tx, mut rx) = spsc::channel::<String>(100);
///
/// tx.send("hello".to_string()).unwrap();
/// assert_eq!(rx.recv().unwrap(), "hello");
/// ```
pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    channel_with_config(capacity, DEFAULT_SNOOZE_ITERS)
}

/// Creates a bounded SPSC channel with custom backoff configuration.
///
/// # Arguments
///
/// * `capacity` - Maximum number of messages the channel can hold (rounded to power of 2)
/// * `snooze_iters` - Number of backoff iterations before parking. Higher values
///   burn more CPU but reduce latency for bursty workloads.
///
/// # Panics
///
/// Panics if `capacity` is 0.
pub fn channel_with_config<T>(capacity: usize, snooze_iters: usize) -> (Sender<T>, Receiver<T>) {
    let (producer, consumer) = ring_buffer(capacity);

    let shared = Arc::new(Shared {
        sender_parked: CachePadded::new(AtomicBool::new(false)),
        receiver_parked: CachePadded::new(AtomicBool::new(false)),
    });

    let sender_parker = Parker::new();
    let sender_unparker = sender_parker.unparker().clone();

    let receiver_parker = Parker::new();
    let receiver_unparker = receiver_parker.unparker().clone();

    (
        Sender {
            producer: ManuallyDrop::new(producer),
            shared: Arc::clone(&shared),
            parker: sender_parker,
            receiver_unparker,
            snooze_iters,
        },
        Receiver {
            consumer: ManuallyDrop::new(consumer),
            shared,
            parker: receiver_parker,
            sender_unparker,
            snooze_iters,
        },
    )
}

/// The sending half of an SPSC channel.
///
/// Messages can be sent with [`send`](Sender::send) (blocking) or
/// [`try_send`](Sender::try_send) (non-blocking).
pub struct Sender<T> {
    producer: ManuallyDrop<Producer<T>>,
    shared: Arc<Shared>,
    parker: Parker,
    receiver_unparker: Unparker,
    snooze_iters: usize,
}

impl<T> Sender<T> {
    /// Sends a message into the channel, blocking if necessary.
    ///
    /// If the channel is full, this method will block until space is available
    /// or the receiver disconnects.
    ///
    /// Returns `Err(SendError(value))` if the receiver has been dropped.
    pub fn send(&mut self, value: T) -> Result<(), SendError<T>> {
        if self.producer.is_disconnected() {
            return Err(SendError(value));
        }

        let mut val = value;

        // Fast path
        match self.producer.push(val) {
            Ok(()) => {
                self.notify_receiver();
                return Ok(());
            }
            Err(Full(v)) => val = v,
        }

        // Backoff phase
        let backoff = Backoff::new();
        for _ in 0..self.snooze_iters {
            backoff.snooze();

            if self.producer.is_disconnected() {
                return Err(SendError(val));
            }

            match self.producer.push(val) {
                Ok(()) => {
                    self.notify_receiver();
                    return Ok(());
                }
                Err(Full(v)) => val = v,
            }
        }

        // Park phase
        loop {
            self.shared.sender_parked.store(true, Ordering::SeqCst);

            if self.producer.is_disconnected() {
                self.shared.sender_parked.store(false, Ordering::Relaxed);
                return Err(SendError(val));
            }

            match self.producer.push(val) {
                Ok(()) => {
                    self.shared.sender_parked.store(false, Ordering::Relaxed);
                    self.notify_receiver();
                    return Ok(());
                }
                Err(Full(v)) => val = v,
            }

            self.parker.park();
            self.shared.sender_parked.store(false, Ordering::Relaxed);

            if self.producer.is_disconnected() {
                return Err(SendError(val));
            }

            match self.producer.push(val) {
                Ok(()) => {
                    self.notify_receiver();
                    return Ok(());
                }
                Err(Full(v)) => val = v,
            }
        }
    }

    /// Attempts to send a message without blocking.
    ///
    /// Returns immediately with:
    /// - `Ok(())` if the message was sent
    /// - `Err(TrySendError::Full(value))` if the channel is full
    /// - `Err(TrySendError::Disconnected(value))` if the receiver was dropped
    pub fn try_send(&mut self, value: T) -> Result<(), TrySendError<T>> {
        if self.producer.is_disconnected() {
            return Err(TrySendError::Disconnected(value));
        }

        match self.producer.push(value) {
            Ok(()) => {
                self.notify_receiver();
                Ok(())
            }
            Err(Full(v)) => Err(TrySendError::Full(v)),
        }
    }

    #[inline]
    fn notify_receiver(&self) {
        if self.shared.receiver_parked.load(Ordering::SeqCst) {
            self.receiver_unparker.unpark();
        }
    }

    /// Returns `true` if the receiver has been dropped.
    #[inline]
    pub fn is_disconnected(&self) -> bool {
        self.producer.is_disconnected()
    }

    /// Returns the capacity of the channel.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.producer.capacity()
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sender")
            .field("capacity", &self.capacity())
            .field("disconnected", &self.is_disconnected())
            .finish_non_exhaustive()
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.producer) };
        self.receiver_unparker.unpark();
    }
}

/// The receiving half of an SPSC channel.
///
/// Messages can be received with [`recv`](Receiver::recv) (blocking),
/// [`recv_timeout`](Receiver::recv_timeout) (blocking with timeout), or
/// [`try_recv`](Receiver::try_recv) (non-blocking).
pub struct Receiver<T> {
    consumer: ManuallyDrop<Consumer<T>>,
    shared: Arc<Shared>,
    parker: Parker,
    sender_unparker: Unparker,
    snooze_iters: usize,
}

impl<T> Receiver<T> {
    /// Receives a message from the channel, blocking if necessary.
    ///
    /// If the channel is empty, this method will block until a message arrives
    /// or the sender disconnects.
    ///
    /// Returns `Err(RecvError)` if the sender has been dropped and no messages
    /// remain in the channel.
    pub fn recv(&mut self) -> Result<T, RecvError> {
        // Fast path
        if let Some(v) = self.consumer.pop() {
            self.notify_sender();
            return Ok(v);
        }

        // Backoff phase
        let backoff = Backoff::new();
        for _ in 0..self.snooze_iters {
            backoff.snooze();

            if let Some(v) = self.consumer.pop() {
                self.notify_sender();
                return Ok(v);
            }

            if self.consumer.is_disconnected() {
                return self.consumer.pop().ok_or(RecvError);
            }
        }

        // Park phase
        loop {
            self.shared.receiver_parked.store(true, Ordering::SeqCst);

            if let Some(v) = self.consumer.pop() {
                self.shared.receiver_parked.store(false, Ordering::Relaxed);
                self.notify_sender();
                return Ok(v);
            }

            if self.consumer.is_disconnected() {
                self.shared.receiver_parked.store(false, Ordering::Relaxed);
                return Err(RecvError);
            }

            self.parker.park();
            self.shared.receiver_parked.store(false, Ordering::Relaxed);

            if let Some(v) = self.consumer.pop() {
                self.notify_sender();
                return Ok(v);
            }

            if self.consumer.is_disconnected() {
                return Err(RecvError);
            }
        }
    }

    /// Receives a message from the channel, blocking for at most `timeout`.
    ///
    /// Returns:
    /// - `Ok(value)` if a message was received
    /// - `Err(RecvTimeoutError::Timeout)` if the timeout elapsed
    /// - `Err(RecvTimeoutError::Disconnected)` if the sender was dropped
    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        let deadline = Instant::now() + timeout;

        // Fast path
        if let Some(v) = self.consumer.pop() {
            self.notify_sender();
            return Ok(v);
        }

        // Backoff phase
        let backoff = Backoff::new();
        for _ in 0..self.snooze_iters {
            if Instant::now() >= deadline {
                return Err(RecvTimeoutError::Timeout);
            }

            backoff.snooze();

            if let Some(v) = self.consumer.pop() {
                self.notify_sender();
                return Ok(v);
            }

            if self.consumer.is_disconnected() {
                return self.consumer.pop().ok_or(RecvTimeoutError::Disconnected);
            }
        }

        // Park phase with timeout
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(RecvTimeoutError::Timeout);
            }

            self.shared.receiver_parked.store(true, Ordering::SeqCst);

            if let Some(v) = self.consumer.pop() {
                self.shared.receiver_parked.store(false, Ordering::Relaxed);
                self.notify_sender();
                return Ok(v);
            }

            if self.consumer.is_disconnected() {
                self.shared.receiver_parked.store(false, Ordering::Relaxed);
                return Err(RecvTimeoutError::Disconnected);
            }

            let remaining = deadline - now;
            self.parker.park_timeout(remaining);
            self.shared.receiver_parked.store(false, Ordering::Relaxed);

            if let Some(v) = self.consumer.pop() {
                self.notify_sender();
                return Ok(v);
            }

            if self.consumer.is_disconnected() {
                return Err(RecvTimeoutError::Disconnected);
            }
        }
    }

    /// Attempts to receive a message without blocking.
    ///
    /// Returns immediately with:
    /// - `Ok(value)` if a message was available
    /// - `Err(TryRecvError::Empty)` if the channel is empty
    /// - `Err(TryRecvError::Disconnected)` if the sender was dropped and channel is empty
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        match self.consumer.pop() {
            Some(v) => {
                self.notify_sender();
                Ok(v)
            }
            None => {
                if self.consumer.is_disconnected() {
                    Err(TryRecvError::Disconnected)
                } else {
                    Err(TryRecvError::Empty)
                }
            }
        }
    }

    #[inline]
    fn notify_sender(&self) {
        if self.shared.sender_parked.load(Ordering::SeqCst) {
            self.sender_unparker.unpark();
        }
    }

    /// Returns `true` if the sender has been dropped.
    #[inline]
    pub fn is_disconnected(&self) -> bool {
        self.consumer.is_disconnected()
    }

    /// Returns the capacity of the channel.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.consumer.capacity()
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver")
            .field("capacity", &self.capacity())
            .field("disconnected", &self.is_disconnected())
            .finish_non_exhaustive()
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.consumer) };
        self.sender_unparker.unpark();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::thread;

    // ============================================================================
    // Basic Operations
    // ============================================================================

    #[test]
    fn basic_send_recv() {
        let (mut tx, mut rx) = channel::<u64>(4);

        tx.send(1).unwrap();
        tx.send(2).unwrap();
        tx.send(3).unwrap();

        assert_eq!(rx.recv().unwrap(), 1);
        assert_eq!(rx.recv().unwrap(), 2);
        assert_eq!(rx.recv().unwrap(), 3);
    }

    #[test]
    fn try_send_try_recv() {
        let (mut tx, mut rx) = channel::<u64>(2);

        assert!(tx.try_send(1).is_ok());
        assert!(tx.try_send(2).is_ok());
        assert!(matches!(tx.try_send(3), Err(TrySendError::Full(3))));

        assert_eq!(rx.try_recv().unwrap(), 1);
        assert_eq!(rx.try_recv().unwrap(), 2);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn send_fills_then_recv_drains() {
        let (mut tx, mut rx) = channel::<u64>(4);

        for i in 0..4 {
            tx.try_send(i).unwrap();
        }
        assert!(matches!(tx.try_send(99), Err(TrySendError::Full(99))));

        for i in 0..4 {
            assert_eq!(rx.recv().unwrap(), i);
        }
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    // ============================================================================
    // Timeout Operations
    // ============================================================================

    #[test]
    fn recv_timeout_success() {
        let (mut tx, mut rx) = channel::<u64>(4);

        tx.send(42).unwrap();

        let result = rx.recv_timeout(Duration::from_millis(100));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn recv_timeout_expires() {
        let (_tx, mut rx) = channel::<u64>(4);

        let start = Instant::now();
        let result = rx.recv_timeout(Duration::from_millis(50));

        assert!(matches!(result, Err(RecvTimeoutError::Timeout)));
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn recv_timeout_disconnected() {
        let (tx, mut rx) = channel::<u64>(4);

        drop(tx);

        let result = rx.recv_timeout(Duration::from_millis(100));
        assert!(matches!(result, Err(RecvTimeoutError::Disconnected)));
    }

    #[test]
    fn recv_timeout_data_arrives() {
        let (mut tx, mut rx) = channel::<u64>(4);

        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(25));
            tx.send(42).unwrap();
        });

        let result = rx.recv_timeout(Duration::from_millis(100));
        assert_eq!(result.unwrap(), 42);

        handle.join().unwrap();
    }

    #[test]
    fn recv_timeout_disconnect_while_waiting() {
        let (tx, mut rx) = channel::<u64>(4);

        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(25));
            drop(tx);
        });

        let result = rx.recv_timeout(Duration::from_millis(100));
        assert!(matches!(result, Err(RecvTimeoutError::Disconnected)));

        handle.join().unwrap();
    }

    // ============================================================================
    // Disconnection
    // ============================================================================

    #[test]
    fn recv_returns_error_when_sender_dropped() {
        let (tx, mut rx) = channel::<u64>(4);

        drop(tx);

        assert!(rx.recv().is_err());
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
    }

    #[test]
    fn recv_drains_before_error_when_sender_dropped() {
        let (mut tx, mut rx) = channel::<u64>(4);

        tx.send(1).unwrap();
        tx.send(2).unwrap();
        drop(tx);

        assert_eq!(rx.recv().unwrap(), 1);
        assert_eq!(rx.recv().unwrap(), 2);
        assert!(rx.recv().is_err());
    }

    #[test]
    fn send_returns_error_when_receiver_dropped() {
        let (mut tx, rx) = channel::<u64>(4);

        drop(rx);

        assert!(tx.send(1).is_err());
        assert!(matches!(tx.try_send(1), Err(TrySendError::Disconnected(1))));
    }

    #[test]
    fn is_disconnected_sender() {
        let (tx, rx) = channel::<u64>(4);

        assert!(!tx.is_disconnected());
        drop(rx);
        assert!(tx.is_disconnected());
    }

    #[test]
    fn is_disconnected_receiver() {
        let (tx, rx) = channel::<u64>(4);

        assert!(!rx.is_disconnected());
        drop(tx);
        assert!(rx.is_disconnected());
    }

    // ============================================================================
    // Cross-Thread Basic
    // ============================================================================

    #[test]
    fn cross_thread_single_message() {
        let (mut tx, mut rx) = channel::<u64>(4);

        let handle = thread::spawn(move || rx.recv().unwrap());

        tx.send(42).unwrap();

        assert_eq!(handle.join().unwrap(), 42);
    }

    #[test]
    fn cross_thread_multiple_messages() {
        let (mut tx, mut rx) = channel::<u64>(4);

        let handle = thread::spawn(move || {
            let mut sum = 0;
            for _ in 0..100 {
                sum += rx.recv().unwrap();
            }
            sum
        });

        for i in 0..100 {
            tx.send(i).unwrap();
        }

        let sum = handle.join().unwrap();
        assert_eq!(sum, 99 * 100 / 2);
    }

    // ============================================================================
    // FIFO Ordering
    // ============================================================================

    #[test]
    fn fifo_ordering_single_thread() {
        let (mut tx, mut rx) = channel::<u64>(8);

        for i in 0..8 {
            tx.try_send(i).unwrap();
        }

        for i in 0..8 {
            assert_eq!(rx.recv().unwrap(), i);
        }
    }

    #[test]
    fn fifo_ordering_cross_thread() {
        let (mut tx, mut rx) = channel::<u64>(64);

        let handle = thread::spawn(move || {
            let mut expected = 0u64;
            while expected < 10_000 {
                let val = rx.recv().unwrap();
                assert_eq!(val, expected, "FIFO order violated");
                expected += 1;
            }
        });

        for i in 0..10_000 {
            tx.send(i).unwrap();
        }

        handle.join().unwrap();
    }

    // ============================================================================
    // Blocking Behavior
    // ============================================================================

    #[test]
    fn recv_blocks_until_send() {
        let (mut tx, mut rx) = channel::<u64>(4);

        let start = Instant::now();

        let handle = thread::spawn(move || rx.recv().unwrap());

        thread::sleep(Duration::from_millis(50));
        tx.send(42).unwrap();

        let val = handle.join().unwrap();
        assert_eq!(val, 42);
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn send_blocks_until_recv() {
        let (mut tx, mut rx) = channel::<u64>(2);

        // Fill the buffer
        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();

        let start = Instant::now();

        let handle = thread::spawn(move || {
            tx.send(3).unwrap(); // Should block
            tx
        });

        thread::sleep(Duration::from_millis(50));
        rx.recv().unwrap(); // Free up space

        let _ = handle.join().unwrap();
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    // ============================================================================
    // Wake on Disconnect
    // ============================================================================

    #[test]
    fn recv_wakes_on_sender_drop() {
        let (tx, mut rx) = channel::<u64>(4);

        let handle = thread::spawn(move || {
            let result = rx.recv();
            assert!(result.is_err());
        });

        thread::sleep(Duration::from_millis(50));
        drop(tx);

        // Should complete, not hang
        handle.join().unwrap();
    }

    #[test]
    fn send_wakes_on_receiver_drop() {
        let (mut tx, rx) = channel::<u64>(1);

        tx.try_send(1).unwrap(); // Fill it

        let handle = thread::spawn(move || {
            let result = tx.send(2); // Should block then error
            assert!(result.is_err());
        });

        thread::sleep(Duration::from_millis(50));
        drop(rx);

        // Should complete, not hang
        handle.join().unwrap();
    }

    // ============================================================================
    // Capacity Edge Cases
    // ============================================================================

    #[test]
    fn capacity_one() {
        let (mut tx, mut rx) = channel::<u64>(1);

        for i in 0..100 {
            tx.send(i).unwrap();
            assert_eq!(rx.recv().unwrap(), i);
        }
    }

    #[test]
    fn capacity_one_cross_thread() {
        let (mut tx, mut rx) = channel::<u64>(1);

        let handle = thread::spawn(move || {
            for _ in 0..1000 {
                rx.recv().unwrap();
            }
        });

        for i in 0..1000 {
            tx.send(i).unwrap();
        }

        handle.join().unwrap();
    }

    // ============================================================================
    // Drop Behavior
    // ============================================================================

    #[test]
    fn values_dropped_on_channel_drop() {
        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        #[derive(Debug)]
        struct DropCounter;
        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        DROP_COUNT.store(0, Ordering::SeqCst);

        let (mut tx, rx) = channel::<DropCounter>(4);

        tx.try_send(DropCounter).unwrap();
        tx.try_send(DropCounter).unwrap();
        tx.try_send(DropCounter).unwrap();

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);

        drop(tx);
        drop(rx);

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn failed_send_returns_value() {
        let (mut tx, rx) = channel::<String>(1);

        tx.try_send("hello".to_string()).unwrap();

        let err = tx.try_send("world".to_string());
        match err {
            Err(TrySendError::Full(s)) => assert_eq!(s, "world"),
            _ => panic!("expected Full error"),
        }

        drop(rx);

        let err = tx.try_send("test".to_string());
        match err {
            Err(TrySendError::Disconnected(s)) => assert_eq!(s, "test"),
            _ => panic!("expected Disconnected error"),
        }
    }

    // ============================================================================
    // Special Types
    // ============================================================================

    #[test]
    fn zero_sized_type() {
        let (mut tx, mut rx) = channel::<()>(4);

        tx.send(()).unwrap();
        tx.send(()).unwrap();

        assert_eq!(rx.recv().unwrap(), ());
        assert_eq!(rx.recv().unwrap(), ());
    }

    #[test]
    fn large_message_type() {
        #[derive(Clone, PartialEq, Debug)]
        struct LargeMessage {
            data: [u8; 4096],
        }

        let (mut tx, mut rx) = channel::<LargeMessage>(4);

        let msg = LargeMessage { data: [42u8; 4096] };
        tx.send(msg.clone()).unwrap();

        let received = rx.recv().unwrap();
        assert_eq!(received.data[0], 42);
        assert_eq!(received.data[4095], 42);
    }

    // ============================================================================
    // Multiple Laps
    // ============================================================================

    #[test]
    fn many_laps_single_thread() {
        let (mut tx, mut rx) = channel::<u64>(4);

        // 1000 messages through 4-slot buffer = 250 laps
        for i in 0..1000 {
            tx.send(i).unwrap();
            assert_eq!(rx.recv().unwrap(), i);
        }
    }

    #[test]
    fn many_laps_cross_thread() {
        const COUNT: u64 = 100_000;

        let (mut tx, mut rx) = channel::<u64>(4); // Small buffer, many laps

        let producer = thread::spawn(move || {
            for i in 0..COUNT {
                tx.send(i).unwrap();
            }
        });

        let consumer = thread::spawn(move || {
            let mut expected = 0u64;
            while expected < COUNT {
                let val = rx.recv().unwrap();
                assert_eq!(val, expected);
                expected += 1;
            }
        });

        producer.join().unwrap();
        consumer.join().unwrap();
    }

    // ============================================================================
    // Stress Tests
    // ============================================================================

    #[test]
    fn stress_high_volume() {
        const COUNT: u64 = 100_000;

        let (mut tx, mut rx) = channel::<u64>(1024);

        let producer = thread::spawn(move || {
            for i in 0..COUNT {
                tx.send(i).unwrap();
            }
        });

        let consumer = thread::spawn(move || {
            let mut sum = 0u64;
            for _ in 0..COUNT {
                sum = sum.wrapping_add(rx.recv().unwrap());
            }
            sum
        });

        producer.join().unwrap();
        let sum = consumer.join().unwrap();
        assert_eq!(sum, COUNT * (COUNT - 1) / 2);
    }

    #[test]
    fn stress_small_buffer() {
        const COUNT: u64 = 10_000;

        let (mut tx, mut rx) = channel::<u64>(4);

        let producer = thread::spawn(move || {
            for i in 0..COUNT {
                tx.send(i).unwrap();
            }
        });

        let consumer = thread::spawn(move || {
            let mut received = 0u64;
            while received < COUNT {
                rx.recv().unwrap();
                received += 1;
            }
            received
        });

        producer.join().unwrap();
        let received = consumer.join().unwrap();
        assert_eq!(received, COUNT);
    }

    #[test]
    fn stress_capacity_one_high_volume() {
        const COUNT: u64 = 10_000;

        let (mut tx, mut rx) = channel::<u64>(1);

        let producer = thread::spawn(move || {
            for i in 0..COUNT {
                tx.send(i).unwrap();
            }
        });

        let consumer = thread::spawn(move || {
            let mut expected = 0u64;
            while expected < COUNT {
                let val = rx.recv().unwrap();
                assert_eq!(val, expected);
                expected += 1;
            }
        });

        producer.join().unwrap();
        consumer.join().unwrap();
    }

    // ============================================================================
    // Ping-Pong Tests (exercises park/unpark heavily)
    // ============================================================================

    #[test]
    fn ping_pong_basic() {
        let (mut tx1, mut rx1) = channel::<u64>(1);
        let (mut tx2, mut rx2) = channel::<u64>(1);

        let handle = thread::spawn(move || {
            for i in 0..1000 {
                let val = rx1.recv().unwrap();
                assert_eq!(val, i);
                tx2.send(i).unwrap();
            }
        });

        for i in 0..1000 {
            tx1.send(i).unwrap();
            let val = rx2.recv().unwrap();
            assert_eq!(val, i);
        }

        handle.join().unwrap();
    }

    #[test]
    fn ping_pong_high_iterations() {
        let (mut tx1, mut rx1) = channel::<u64>(1);
        let (mut tx2, mut rx2) = channel::<u64>(1);

        let handle = thread::spawn(move || {
            for i in 0..10_000 {
                let val = rx1.recv().unwrap();
                assert_eq!(val, i);
                tx2.send(i * 2).unwrap();
            }
        });

        for i in 0..10_000 {
            tx1.send(i).unwrap();
            let val = rx2.recv().unwrap();
            assert_eq!(val, i * 2);
        }

        handle.join().unwrap();
    }

    #[test]
    fn ping_pong_with_delays() {
        let (mut tx1, mut rx1) = channel::<u64>(1);
        let (mut tx2, mut rx2) = channel::<u64>(1);

        let handle = thread::spawn(move || {
            for i in 0..100 {
                let val = rx1.recv().unwrap();
                // Occasional delay to force other side to park
                if i % 20 == 0 {
                    thread::sleep(Duration::from_micros(100));
                }
                tx2.send(val).unwrap();
            }
        });

        for i in 0..100 {
            // Occasional delay to force other side to park
            if i % 17 == 0 {
                thread::sleep(Duration::from_micros(100));
            }
            tx1.send(i).unwrap();
            let val = rx2.recv().unwrap();
            assert_eq!(val, i);
        }

        handle.join().unwrap();
    }

    // ============================================================================
    // Deadlock Prevention Tests
    // ============================================================================

    #[test]
    fn no_deadlock_alternating() {
        let (mut tx, mut rx) = channel::<u64>(1);

        let handle = thread::spawn(move || {
            for i in 0..1000u64 {
                tx.send(i).unwrap();
            }
        });

        for _ in 0..1000 {
            rx.recv().unwrap();
        }

        handle.join().unwrap();
    }

    #[test]
    fn no_deadlock_burst_then_drain() {
        let (mut tx, mut rx) = channel::<u64>(8);

        for round in 0..100 {
            // Burst
            for i in 0..8 {
                tx.try_send(round * 8 + i).unwrap();
            }
            // Drain
            for i in 0..8 {
                assert_eq!(rx.recv().unwrap(), round * 8 + i);
            }
        }
    }

    #[test]
    fn no_deadlock_concurrent_full_empty_transitions() {
        let (mut tx, mut rx) = channel::<u64>(2);

        let producer = thread::spawn(move || {
            for i in 0..10_000u64 {
                tx.send(i).unwrap();
            }
        });

        let consumer = thread::spawn(move || {
            for _ in 0..10_000 {
                rx.recv().unwrap();
            }
        });

        producer.join().unwrap();
        consumer.join().unwrap();
    }

    #[test]
    fn no_deadlock_disconnect_while_blocked_recv() {
        let (tx, mut rx) = channel::<u64>(1);

        let handle = thread::spawn(move || {
            // Will block waiting for data
            let result = rx.recv();
            assert!(result.is_err()); // Should error, not deadlock
        });

        thread::sleep(Duration::from_millis(50));
        drop(tx); // Disconnect while receiver is blocked

        handle.join().unwrap();
    }

    #[test]
    fn no_deadlock_disconnect_while_blocked_send() {
        let (mut tx, rx) = channel::<u64>(1);
        tx.try_send(1).unwrap(); // Fill it

        let handle = thread::spawn(move || {
            // Will block waiting for space
            let result = tx.send(2);
            assert!(result.is_err()); // Should error, not deadlock
        });

        thread::sleep(Duration::from_millis(50));
        drop(rx); // Disconnect while sender is blocked

        handle.join().unwrap();
    }

    // ============================================================================
    // Park/Unpark Race Condition Tests
    // ============================================================================

    #[test]
    fn race_send_before_recv_parks() {
        // Send happens just before recv decides to park
        for _ in 0..100 {
            let (mut tx, mut rx) = channel::<u64>(1);

            let handle = thread::spawn(move || rx.recv().unwrap());

            // Tiny delay to let receiver potentially start parking
            thread::yield_now();
            tx.send(42).unwrap();

            assert_eq!(handle.join().unwrap(), 42);
        }
    }

    #[test]
    fn race_recv_before_send_parks() {
        // Recv happens just before send decides to park on full buffer
        for _ in 0..100 {
            let (mut tx, mut rx) = channel::<u64>(1);
            tx.try_send(1).unwrap(); // Fill it

            let handle = thread::spawn(move || {
                tx.send(2).unwrap();
            });

            // Tiny delay to let sender potentially start parking
            thread::yield_now();
            rx.recv().unwrap(); // Make space

            handle.join().unwrap();
        }
    }

    #[test]
    fn race_disconnect_during_park_transition() {
        // Disconnect happens during the brief window of parking
        for _ in 0..100 {
            let (tx, mut rx) = channel::<u64>(1);

            let handle = thread::spawn(move || {
                let _ = rx.recv(); // May succeed or fail, shouldn't deadlock
            });

            // Immediately disconnect
            drop(tx);

            handle.join().unwrap();
        }
    }

    // ============================================================================
    // Stress: Rapid Park/Unpark Cycles
    // ============================================================================

    #[test]
    fn stress_rapid_park_unpark_sender() {
        let (mut tx, mut rx) = channel::<u64>(1);

        let handle = thread::spawn(move || {
            for _ in 0..10_000 {
                rx.recv().unwrap();
            }
        });

        for i in 0..10_000 {
            tx.send(i).unwrap();
        }

        handle.join().unwrap();
    }

    #[test]
    fn stress_rapid_park_unpark_receiver() {
        let (mut tx, mut rx) = channel::<u64>(1);

        let handle = thread::spawn(move || {
            for i in 0..10_000 {
                tx.send(i).unwrap();
            }
        });

        for _ in 0..10_000 {
            rx.recv().unwrap();
        }

        handle.join().unwrap();
    }

    #[test]
    fn stress_park_unpark_both_sides() {
        // Both sender and receiver will park repeatedly
        let (mut tx, mut rx) = channel::<u64>(1);

        let sender = thread::spawn(move || {
            for i in 0..50_000 {
                tx.send(i).unwrap();
            }
        });

        let receiver = thread::spawn(move || {
            let mut count = 0;
            for _ in 0..50_000 {
                rx.recv().unwrap();
                count += 1;
            }
            count
        });

        sender.join().unwrap();
        assert_eq!(receiver.join().unwrap(), 50_000);
    }

    // ============================================================================
    // Timed Tests (ensure no indefinite blocking)
    // ============================================================================

    #[test]
    fn completes_in_reasonable_time() {
        use std::sync::mpsc;

        let (done_tx, done_rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let (mut tx, mut rx) = channel::<u64>(1);

            let h = thread::spawn(move || {
                for i in 0..1000 {
                    tx.send(i).unwrap();
                }
            });

            for _ in 0..1000 {
                rx.recv().unwrap();
            }

            h.join().unwrap();
            done_tx.send(()).unwrap();
        });

        // Should complete in well under a second
        let result = done_rx.recv_timeout(Duration::from_secs(5));
        assert!(result.is_ok(), "Test timed out - possible deadlock!");

        handle.join().unwrap();
    }

    #[test]
    fn does_not_hang_on_disconnect_during_recv() {
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = done.clone();

        let (tx, mut rx) = channel::<u64>(4);

        let handle = thread::spawn(move || {
            let _ = rx.recv(); // Will block, then return Err on disconnect
            done_clone.store(true, Ordering::SeqCst);
        });

        thread::sleep(Duration::from_millis(50));
        assert!(!done.load(Ordering::SeqCst)); // Still blocked

        drop(tx);

        handle.join().unwrap();
        assert!(done.load(Ordering::SeqCst)); // Completed
    }

    #[test]
    fn does_not_hang_on_disconnect_during_send() {
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = done.clone();

        let (mut tx, rx) = channel::<u64>(1);
        tx.try_send(1).unwrap(); // Fill it

        let handle = thread::spawn(move || {
            let _ = tx.send(2); // Will block, then return Err on disconnect
            done_clone.store(true, Ordering::SeqCst);
        });

        thread::sleep(Duration::from_millis(50));
        assert!(!done.load(Ordering::SeqCst)); // Still blocked

        drop(rx);

        handle.join().unwrap();
        assert!(done.load(Ordering::SeqCst)); // Completed
    }

    // ============================================================================
    // Rapid Connect/Disconnect
    // ============================================================================

    #[test]
    fn rapid_channel_creation() {
        for _ in 0..1000 {
            let (mut tx, mut rx) = channel::<u64>(4);
            tx.try_send(1).unwrap();
            assert_eq!(rx.recv().unwrap(), 1);
        }
    }

    #[test]
    fn rapid_disconnect() {
        for _ in 0..1000 {
            let (tx, rx) = channel::<u64>(4);
            drop(tx);
            drop(rx);
        }
    }
}
