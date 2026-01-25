//! High-performance lock-free queues for latency-critical applications.
//!
//! `nexus-queue` provides a bounded SPSC (single-producer, single-consumer) queue
//! optimized for trading systems and other low-latency workloads.
//!
//! # Quick Start
//!
//! ```
//! use nexus_queue::spsc;
//!
//! let (mut tx, mut rx) = spsc::ring_buffer::<u64>(1024);
//!
//! tx.push(42).unwrap();
//! assert_eq!(rx.pop(), Some(42));
//! ```
//!
//! # Design
//!
//! The SPSC implementation uses cached head/tail indices with separate cache lines
//! to avoid false sharing. Producer and consumer each maintain a local copy of
//! the other's index, only refreshing from the atomic when their cache indicates
//! the queue is full (producer) or empty (consumer).
//!
//! This design performs well on multi-socket NUMA systems where cache line
//! ownership is important for latency.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs, missing_debug_implementations)]

use core::fmt;

pub mod spsc;

/// Error returned when pushing to a full queue.
///
/// Contains the value that could not be pushed, returning ownership to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full<T>(pub T);

impl<T> Full<T> {
    /// Returns the value that could not be pushed.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Display for Full<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "queue is full")
    }
}

impl<T: fmt::Debug> std::error::Error for Full<T> {}
