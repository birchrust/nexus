//! Async channels for task communication.
//!
//! # Channel Types
//!
//! - [`local`] — bounded MPSC for single-threaded use. `!Send`, `!Sync`.
//!   No atomics, no `Arc`, zero synchronization overhead.
//! - [`mpsc`] — bounded MPSC for cross-thread use. `Sender: Clone + Send + Sync`,
//!   `Receiver: Send`. Lock-free atomic queue (nexus-queue).
//! - [`spsc`] — bounded SPSC for cross-thread use. `Sender: Send`,
//!   `Receiver: Send`. Single producer, single consumer. Fastest cross-thread channel.
//!
//! All must be created inside [`Runtime::block_on`](crate::Runtime::block_on).
//!
//! # Example
//!
//! ```ignore
//! use nexus_async_rt::channel::local;
//!
//! // Inside block_on:
//! let (tx, rx) = local::channel::<u64>(64);
//!
//! spawn_boxed(async move {
//!     tx.send(42).await.unwrap();
//! });
//!
//! let value = rx.recv().await.unwrap();
//! assert_eq!(value, 42);
//! ```

pub mod local;
pub mod mpsc;
pub mod spsc;

use std::fmt;

// =============================================================================
// Error types
// =============================================================================

/// The receiver was dropped — channel is closed.
///
/// Contains the value that could not be sent.
pub struct SendError<T>(pub T);

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SendError(..)")
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("channel closed")
    }
}

impl<T: fmt::Debug> std::error::Error for SendError<T> {}

/// Error returned by [`Sender::try_send`](mpsc::Sender::try_send).
pub enum TrySendError<T> {
    /// The channel buffer is full.
    Full(T),
    /// The receiver was dropped — channel is closed.
    Closed(T),
}

impl<T> TrySendError<T> {
    /// Consume the error, returning the value that could not be sent.
    pub fn into_inner(self) -> T {
        match self {
            Self::Full(v) | Self::Closed(v) => v,
        }
    }

    /// Whether this is a `Full` error.
    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }

    /// Whether this is a `Closed` error.
    pub fn is_closed(&self) -> bool {
        matches!(self, Self::Closed(_))
    }
}

impl<T> fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => f.write_str("Full(..)"),
            Self::Closed(_) => f.write_str("Closed(..)"),
        }
    }
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => f.write_str("channel full"),
            Self::Closed(_) => f.write_str("channel closed"),
        }
    }
}

impl<T: fmt::Debug> std::error::Error for TrySendError<T> {}

/// All senders were dropped — no more values will arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvError;

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("channel closed")
    }
}

impl std::error::Error for RecvError {}

/// Error returned by [`Receiver::try_recv`](mpsc::Receiver::try_recv).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    /// The channel buffer is empty.
    Empty,
    /// All senders were dropped — channel is closed.
    Closed,
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("channel empty"),
            Self::Closed => f.write_str("channel closed"),
        }
    }
}

impl std::error::Error for TryRecvError {}
