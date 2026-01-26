//! High-performance bounded channels for low-latency systems.
//!
//! This crate provides blocking channels optimized for trading systems and other
//! latency-critical workloads:
//!
//! - [`spsc`]: Single-producer single-consumer channel
//! - [`mpsc`]: Multi-producer single-consumer channel
//!
//! # Design
//!
//! Both channel types use a three-phase backoff strategy that minimizes syscall
//! overhead:
//!
//! 1. **Fast path**: Try the operation immediately
//! 2. **Backoff**: Spin with exponential backoff using `crossbeam::Backoff`
//! 3. **Park**: Sleep until woken by the other end
//!
//! The key optimization is *conditional parking*: we only issue expensive unpark
//! syscalls when the other end has actually gone to sleep. This dramatically
//! reduces tail latency compared to channels that unpark unconditionally.
//!
//! # Quick Start
//!
//! ```
//! use nexus_channel::spsc;
//!
//! let (mut tx, mut rx) = spsc::channel::<u64>(1024);
//!
//! tx.send(42).unwrap();
//! assert_eq!(rx.recv().unwrap(), 42);
//! ```
//!
//! ```
//! use nexus_channel::mpsc;
//! use std::thread;
//!
//! let (tx, mut rx) = mpsc::channel::<u64>(1024);
//!
//! let tx2 = tx.clone();
//! thread::spawn(move || tx.send(1).unwrap());
//! thread::spawn(move || tx2.send(2).unwrap());
//!
//! rx.recv().unwrap();
//! rx.recv().unwrap();
//! ```
//!
//! # Timeout Support
//!
//! Both channel types support receiving with a timeout:
//!
//! ```
//! use nexus_channel::{spsc, RecvTimeoutError};
//! use std::time::Duration;
//!
//! let (tx, mut rx) = spsc::channel::<u64>(4);
//!
//! match rx.recv_timeout(Duration::from_millis(100)) {
//!     Ok(value) => println!("got {}", value),
//!     Err(RecvTimeoutError::Timeout) => println!("timed out"),
//!     Err(RecvTimeoutError::Disconnected) => println!("sender dropped"),
//! }
//! ```
//!
//! # Performance
//!
//! Benchmarked against `crossbeam-channel` on Intel Core Ultra 7 @ 2.7GHz,
//! pinned to physical cores with turbo disabled:
//!
//! | Metric | nexus-channel (SPSC) | crossbeam-channel | Improvement |
//! |--------|----------------------|-------------------|-------------|
//! | p50 latency | 665 cycles | 1344 cycles | **2.0x** |
//! | p999 latency | 2501 cycles | 37023 cycles | **14.8x** |
//! | Throughput | 64 M msgs/sec | 34 M msgs/sec | **1.9x** |
//!
//! The large p999 improvement comes from avoiding unnecessary syscalls.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs, missing_debug_implementations)]

use core::fmt;

pub mod mpsc;
pub mod spsc;

// ============================================================================
// Error Types
// ============================================================================

/// Error returned when sending fails due to disconnection.
///
/// Contains the message that could not be sent, allowing recovery of the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendError<T>(pub T);

impl<T> SendError<T> {
    /// Returns the message that could not be sent.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "channel disconnected")
    }
}

impl<T: fmt::Debug> std::error::Error for SendError<T> {}

/// Error returned when receiving fails due to disconnection.
///
/// This error occurs when all senders have been dropped and no messages
/// remain in the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvError;

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "channel disconnected")
    }
}

impl std::error::Error for RecvError {}

/// Error returned by `try_send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrySendError<T> {
    /// The channel is full but still connected.
    Full(T),

    /// The receiver has been dropped.
    Disconnected(T),
}

impl<T> TrySendError<T> {
    /// Returns the message that could not be sent.
    pub fn into_inner(self) -> T {
        match self {
            TrySendError::Full(v) | TrySendError::Disconnected(v) => v,
        }
    }

    /// Returns `true` if this error is the `Full` variant.
    pub fn is_full(&self) -> bool {
        matches!(self, TrySendError::Full(_))
    }

    /// Returns `true` if this error is the `Disconnected` variant.
    pub fn is_disconnected(&self) -> bool {
        matches!(self, TrySendError::Disconnected(_))
    }
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrySendError::Full(_) => write!(f, "channel full"),
            TrySendError::Disconnected(_) => write!(f, "channel disconnected"),
        }
    }
}

impl<T: fmt::Debug> std::error::Error for TrySendError<T> {}

/// Error returned by `try_recv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    /// The channel is empty but still connected.
    Empty,

    /// All senders have been dropped and no messages remain.
    Disconnected,
}

impl TryRecvError {
    /// Returns `true` if this error is the `Empty` variant.
    pub fn is_empty(&self) -> bool {
        matches!(self, TryRecvError::Empty)
    }

    /// Returns `true` if this error is the `Disconnected` variant.
    pub fn is_disconnected(&self) -> bool {
        matches!(self, TryRecvError::Disconnected)
    }
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryRecvError::Empty => write!(f, "channel empty"),
            TryRecvError::Disconnected => write!(f, "channel disconnected"),
        }
    }
}

impl std::error::Error for TryRecvError {}

/// Error returned by `recv_timeout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvTimeoutError {
    /// The timeout elapsed before a message arrived.
    Timeout,

    /// All senders have been dropped and no messages remain.
    Disconnected,
}

impl RecvTimeoutError {
    /// Returns `true` if this error is the `Timeout` variant.
    pub fn is_timeout(&self) -> bool {
        matches!(self, RecvTimeoutError::Timeout)
    }

    /// Returns `true` if this error is the `Disconnected` variant.
    pub fn is_disconnected(&self) -> bool {
        matches!(self, RecvTimeoutError::Disconnected)
    }
}

impl fmt::Display for RecvTimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecvTimeoutError::Timeout => write!(f, "timed out"),
            RecvTimeoutError::Disconnected => write!(f, "channel disconnected"),
        }
    }
}

impl std::error::Error for RecvTimeoutError {}
