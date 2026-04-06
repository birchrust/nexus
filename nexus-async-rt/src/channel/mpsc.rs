//! Bounded cross-thread MPSC channel.
//!
//! `Sender`: `Clone + Send + Sync`. `Receiver`: `Send`.
//! Uses `nexus_queue::mpsc` for the data path (atomic, lock-free).
//! Cross-thread wake via the runtime's intrusive MPSC inbox + eventfd.
//!
//! Must be created inside [`Runtime::block_on`](crate::Runtime::block_on)
//! to capture the cross-thread wake context.
//!
//! ```ignore
//! use nexus_async_rt::channel::mpsc;
//!
//! // Inside block_on:
//! let (tx, rx) = mpsc::channel::<u64>(64);
//!
//! // tx can be sent to another thread
//! std::thread::spawn(move || {
//!     tx.try_send(42).unwrap();
//! });
//!
//! let val = rx.recv().await.unwrap();
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use super::{RecvError, SendError, TryRecvError, TrySendError};

// =============================================================================
// AtomicWaker — single-registerer, cross-thread safe
// =============================================================================

/// Atomic waker with inline storage — zero allocation.
///
/// Uses a 3-state atomic guard to protect the `UnsafeCell<Option<Waker>>`:
/// - `EMPTY` (0): no waker stored
/// - `STORED` (1): waker is present, safe to take
/// - `REGISTERING` (2): register in progress (single registerer lock)
///
/// Single-registerer guarantee: only one thread calls `register()`.
/// Multiple threads may call `wake()` concurrently.
struct AtomicWaker {
    state: std::sync::atomic::AtomicU8,
    waker: std::cell::UnsafeCell<Option<Waker>>,
}

const EMPTY: u8 = 0;
const STORED: u8 = 1;
const REGISTERING: u8 = 2;

// SAFETY: The atomic state machine prevents concurrent access to the
// UnsafeCell. Register is single-threaded; wake uses CAS to claim.
unsafe impl Send for AtomicWaker {}
unsafe impl Sync for AtomicWaker {}

impl AtomicWaker {
    fn new() -> Self {
        Self {
            state: std::sync::atomic::AtomicU8::new(EMPTY),
            waker: std::cell::UnsafeCell::new(None),
        }
    }

    /// Register a waker. Replaces any previous waker. Zero allocation.
    ///
    /// Single-registerer: only one thread calls this (the receiver or
    /// the last blocked sender). Concurrent register calls are UB.
    fn register(&self, waker: &Waker) {
        // Claim the registering lock. Single-registerer so no CAS loop needed.
        // If state is STORED, we're replacing. If EMPTY, we're first.
        // If REGISTERING, that's a bug (concurrent register).
        let prev = self.state.swap(REGISTERING, Ordering::Acquire);
        debug_assert_ne!(prev, REGISTERING, "concurrent register on AtomicWaker");

        // SAFETY: we hold the REGISTERING lock — no other thread can access
        // the UnsafeCell while state == REGISTERING. Wake checks state first.
        unsafe { *self.waker.get() = Some(waker.clone()) };

        self.state.store(STORED, Ordering::Release);
    }

    /// Take and wake the stored waker, if any. Thread-safe.
    ///
    /// Returns true if a waker was woken.
    fn wake(&self) -> bool {
        // Try to claim the waker: STORED → EMPTY.
        if self
            .state
            .compare_exchange(STORED, EMPTY, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            // SAFETY: we transitioned STORED→EMPTY, so we own the waker.
            // No other thread can access the UnsafeCell now.
            let waker = unsafe { (*self.waker.get()).take() };
            if let Some(w) = waker {
                w.wake();
                return true;
            }
        }
        false
    }

    /// Whether a waker is currently stored. Quick non-claiming check.
    fn has_waker(&self) -> bool {
        self.state.load(Ordering::Acquire) == STORED
    }
}

impl Drop for AtomicWaker {
    fn drop(&mut self) {
        // No atomics needed in drop — we have &mut self.
        *self.waker.get_mut() = None;
    }
}

// =============================================================================
// Shared state
// =============================================================================

struct Inner<T> {
    /// Data queue (lock-free MPSC from nexus-queue).
    producer: nexus_queue::mpsc::Producer<T>,
    consumer: nexus_queue::mpsc::Consumer<T>,

    /// Waker for the receiver (set by recv, fired by send).
    /// Stores a CROSS-THREAD waker so senders on any thread can wake it.
    rx_waker: AtomicWaker,

    /// Waker for the last blocked sender (set by send, fired by recv).
    tx_waker: AtomicWaker,

    /// Cross-thread wake context for building cross-thread wakers.
    cross_ctx: std::sync::Arc<crate::cross_wake::CrossWakeContext>,

    /// Number of live Sender handles. Atomic for cross-thread clone/drop.
    sender_count: AtomicU64,
    /// Whether the receiver has been dropped.
    rx_closed: std::sync::atomic::AtomicBool,
}

// SAFETY: Producer and Consumer are both Send when T: Send.
// AtomicWaker is Send+Sync (guarded by atomic state machine).
// All atomic fields are Send+Sync. Arc provides shared ownership.
unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Send> Sync for Inner<T> {}

// =============================================================================
// channel()
// =============================================================================

/// Create a bounded cross-thread MPSC channel.
///
/// `capacity` is rounded up to the next power of two.
///
/// # Panics
///
/// - Panics if called outside [`Runtime::block_on`](crate::Runtime::block_on).
/// - Panics if `capacity` is 0.
pub fn channel<T: Send>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    crate::context::assert_in_runtime(
        "mpsc::channel() called outside Runtime::block_on",
    );

    assert!(capacity > 0, "channel capacity must be > 0");

    let cross_ctx = crate::cross_wake::cross_wake_context()
        .expect("mpsc::channel() requires runtime context for cross-thread wake");

    let (producer, consumer) = nexus_queue::mpsc::bounded(capacity);

    let inner = Arc::new(Inner {
        producer,
        consumer,
        rx_waker: AtomicWaker::new(),
        tx_waker: AtomicWaker::new(),
        cross_ctx,
        sender_count: AtomicU64::new(1),
        rx_closed: std::sync::atomic::AtomicBool::new(false),
    });

    let tx = Sender {
        inner: inner.clone(),
    };
    let rx = Receiver { inner };
    (tx, rx)
}

// =============================================================================
// Sender
// =============================================================================

/// Sending half of a bounded cross-thread MPSC channel.
///
/// `Clone + Send + Sync`. Can be used from any thread.
pub struct Sender<T> {
    inner: Arc<Inner<T>>,
}

impl<T: Send> Sender<T> {
    /// Send a value, waiting if the buffer is full.
    ///
    /// Returns `Err(SendError(value))` if the receiver was dropped.
    pub fn send(&self, value: T) -> SendFut<'_, T> {
        SendFut {
            sender: self,
            value: Some(value),
        }
    }

    /// Try to send a value without waiting.
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        if self.inner.rx_closed.load(Ordering::Acquire) {
            return Err(TrySendError::Closed(value));
        }

        match self.inner.producer.push(value) {
            Ok(()) => {
                // Wake receiver if waiting.
                self.inner.rx_waker.wake();
                Ok(())
            }
            Err(nexus_queue::Full(value)) => Err(TrySendError::Full(value)),
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.inner.sender_count.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if self.inner.sender_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            // Last sender dropped — wake receiver so it sees closed.
            self.inner.rx_waker.wake();
        }
    }
}

// SAFETY: Inner uses atomic queue + atomic wakers. All cross-thread safe.
unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

// =============================================================================
// SendFut
// =============================================================================

/// Future returned by [`Sender::send`].
pub struct SendFut<'a, T> {
    sender: &'a Sender<T>,
    value: Option<T>,
}

impl<T: Send> Future for SendFut<'_, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        let inner = &this.sender.inner;

        // Check if receiver dropped.
        if inner.rx_closed.load(Ordering::Acquire) {
            let value = this.value.take().expect("polled after completion");
            return Poll::Ready(Err(SendError(value)));
        }

        // Try to push.
        let value = this.value.take().expect("polled after completion");
        match inner.producer.push(value) {
            Ok(()) => {
                inner.rx_waker.wake();
                Poll::Ready(Ok(()))
            }
            Err(nexus_queue::Full(value)) => {
                // Buffer full — park.
                this.value = Some(value);
                inner.tx_waker.register(cx.waker());
                Poll::Pending
            }
        }
    }
}

// SendFut borrows Sender which is Send+Sync, but the future itself
// holds Option<T>. It's Send if T is Send.
unsafe impl<T: Send> Send for SendFut<'_, T> {}

// =============================================================================
// Receiver
// =============================================================================

/// Receiving half of a bounded cross-thread MPSC channel.
///
/// `Send` but not `Clone` — single consumer. Can be moved to another thread.
pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
}

impl<T: Send> Receiver<T> {
    /// Receive a value, waiting if the buffer is empty.
    ///
    /// Returns `Err(RecvError)` when all senders have been dropped and
    /// the buffer is empty.
    pub fn recv(&self) -> RecvFut<'_, T> {
        RecvFut { receiver: self }
    }

    /// Try to receive a value without waiting.
    #[allow(clippy::option_if_let_else)] // match is clearer here
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.inner.consumer.pop() {
            Some(value) => {
                // Only bump epoch + wake sender if one is actually blocked.
                if self.inner.tx_waker.has_waker() {
                    self.inner.tx_waker.wake();
                }
                Ok(value)
            }
            None => {
                if self.inner.sender_count.load(Ordering::Acquire) == 0 {
                    Err(TryRecvError::Closed)
                } else {
                    Err(TryRecvError::Empty)
                }
            }
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.inner.rx_closed.store(true, Ordering::Release);
        // Wake all blocked senders so they see closed.
        self.inner.tx_waker.wake();
    }
}

// Consumer is Send, Arc is Send+Sync → Receiver is Send.
unsafe impl<T: Send> Send for Receiver<T> {}

// =============================================================================
// RecvFut
// =============================================================================

/// Future returned by [`Receiver::recv`].
pub struct RecvFut<'a, T> {
    receiver: &'a Receiver<T>,
}

impl<T: Send> Future for RecvFut<'_, T> {
    type Output = Result<T, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = &self.receiver.inner;

        // Try to pop.
        if let Some(value) = inner.consumer.pop() {
            if inner.tx_waker.has_waker() {
                inner.tx_waker.wake();
            }
            return Poll::Ready(Ok(value));
        }

        // Empty + all senders dropped → closed.
        if inner.sender_count.load(Ordering::Acquire) == 0 {
            return Poll::Ready(Err(RecvError));
        }

        // Empty + senders alive → park with a cross-thread-safe waker.
        // If the waker is our local runtime waker, build a cross-thread
        // waker from the task pointer (targeted, efficient). Otherwise
        // (e.g., root future waker), clone it directly — it's already
        // Send+Sync via Arc<RootWake>.
        #[allow(clippy::option_if_let_else)]
        let wake = match crate::waker::task_ptr_from_local_waker(cx.waker()) {
            Some(task_ptr) => crate::cross_wake::cross_thread_waker(task_ptr, &inner.cross_ctx),
            None => cx.waker().clone(),
        };
        inner.rx_waker.register(&wake);

        // Re-check after registering to avoid lost wake.
        if let Some(value) = inner.consumer.pop() {
            if inner.tx_waker.has_waker() {
                inner.tx_waker.wake();
            }
            return Poll::Ready(Ok(value));
        }

        if inner.sender_count.load(Ordering::Acquire) == 0 {
            return Poll::Ready(Err(RecvError));
        }

        Poll::Pending
    }
}

unsafe impl<T: Send> Send for RecvFut<'_, T> {}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Create a channel without runtime context (for unit tests).
    /// Creates a real mio::Poll + Waker for the cross-thread wake path.
    fn test_channel<T: Send>(capacity: usize) -> (Sender<T>, Receiver<T>) {
        let poll = mio::Poll::new().unwrap();
        let mio_waker = std::sync::Arc::new(
            mio::Waker::new(poll.registry(), mio::Token(usize::MAX)).unwrap(),
        );
        let cross_ctx = Arc::new(crate::cross_wake::CrossWakeContext {
            queue: crate::cross_wake::CrossWakeQueue::new(),
            mio_waker,
            parked: std::sync::atomic::AtomicBool::new(false),
        });

        let (producer, consumer) = nexus_queue::mpsc::bounded(capacity);
        let inner = Arc::new(Inner {
            producer,
            consumer,
            rx_waker: AtomicWaker::new(),
            tx_waker: AtomicWaker::new(),
            cross_ctx,
            sender_count: AtomicU64::new(1),
            rx_closed: std::sync::atomic::AtomicBool::new(false),
        });
        (
            Sender { inner: inner.clone() },
            Receiver { inner },
        )
    }

    #[test]
    fn send_recv_single() {
        let (tx, rx) = test_channel::<u32>(4);
        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        tx.try_send(3).unwrap();

        assert_eq!(rx.try_recv().unwrap(), 1);
        assert_eq!(rx.try_recv().unwrap(), 2);
        assert_eq!(rx.try_recv().unwrap(), 3);
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn fifo_ordering() {
        let (tx, rx) = test_channel(8);
        for i in 0..8u32 {
            tx.try_send(i).unwrap();
        }
        for i in 0..8u32 {
            assert_eq!(rx.try_recv().unwrap(), i);
        }
    }

    #[test]
    fn try_send_full() {
        let (tx, rx) = test_channel(2);
        tx.try_send(1u32).unwrap();
        tx.try_send(2).unwrap();

        let err = tx.try_send(3).unwrap_err();
        assert!(err.is_full());
        assert_eq!(err.into_inner(), 3);

        assert_eq!(rx.try_recv().unwrap(), 1);
        tx.try_send(3).unwrap();
    }

    #[test]
    fn try_recv_empty() {
        let (tx, rx) = test_channel::<u32>(4);
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));

        tx.try_send(1).unwrap();
        assert_eq!(rx.try_recv().unwrap(), 1);
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn sender_drop_signals_closed() {
        let (tx, rx) = test_channel::<u32>(4);
        tx.try_send(42).unwrap();
        drop(tx);

        assert_eq!(rx.try_recv().unwrap(), 42);
        assert_eq!(rx.try_recv(), Err(TryRecvError::Closed));
    }

    #[test]
    fn receiver_drop_signals_closed() {
        let (tx, rx) = test_channel::<u32>(4);
        drop(rx);

        let err = tx.try_send(1).unwrap_err();
        assert!(err.is_closed());
    }

    #[test]
    fn multiple_senders() {
        let (tx1, rx) = test_channel(8);
        let tx2 = tx1.clone();

        tx1.try_send(1u32).unwrap();
        tx2.try_send(2).unwrap();
        tx1.try_send(3).unwrap();

        assert_eq!(rx.try_recv().unwrap(), 1);
        assert_eq!(rx.try_recv().unwrap(), 2);
        assert_eq!(rx.try_recv().unwrap(), 3);
    }

    #[test]
    fn sender_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Sender<u64>>();
    }

    #[test]
    fn receiver_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Receiver<u64>>();
    }

    #[test]
    fn cross_thread_try_send() {
        // Buffer large enough for all values (no backpressure).
        let (tx, rx) = test_channel::<u64>(128);

        let handle = std::thread::spawn(move || {
            for i in 0..100 {
                tx.try_send(i).unwrap();
            }
        });

        handle.join().unwrap();

        for i in 0..100u64 {
            assert_eq!(rx.try_recv().unwrap(), i);
        }
    }

    #[test]
    fn cross_thread_multiple_producers() {
        // 4 producers × 100 values = 400. Buffer must fit all.
        let (tx, rx) = test_channel::<u64>(512);

        let handles: Vec<_> = (0..4u64)
            .map(|id| {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    for i in 0..100 {
                        tx.try_send(id * 1000 + i).unwrap();
                    }
                })
            })
            .collect();

        drop(tx); // drop original

        for h in handles {
            h.join().unwrap();
        }

        let mut received = Vec::new();
        while let Ok(v) = rx.try_recv() {
            received.push(v);
        }
        assert_eq!(received.len(), 400);
    }

    #[test]
    fn stress_sequential() {
        let (tx, rx) = test_channel(64);
        for i in 0..100_000u64 {
            tx.try_send(i).unwrap();
            assert_eq!(rx.try_recv().unwrap(), i);
        }
    }
}
