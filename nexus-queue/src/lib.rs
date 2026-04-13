//! High-performance lock-free queues for latency-critical applications.
//!
//! `nexus-queue` provides bounded queues optimized for trading systems and other
//! low-latency workloads:
//!
//! - [`spsc`]: Single-producer single-consumer queue with cached indices
//! - [`mpsc`]: Multi-producer single-consumer queue with Vyukov-style turn counters
//! - [`spmc`]: Single-producer multi-consumer queue for fan-out workloads
//!
//! # Quick Start
//!
//! ```
//! // SPSC - one producer, one consumer
//! use nexus_queue::spsc;
//!
//! let (tx, rx) = spsc::ring_buffer::<u64>(1024);
//! tx.push(42).unwrap();
//! assert_eq!(rx.pop(), Some(42));
//! ```
//!
//! ```
//! // MPSC - multiple producers, one consumer
//! use nexus_queue::mpsc;
//!
//! let (tx, rx) = mpsc::ring_buffer::<u64>(1024);
//! let tx2 = tx.clone();  // Clone for second producer
//!
//! tx.push(1).unwrap();
//! tx2.push(2).unwrap();
//!
//! assert!(rx.pop().is_some());
//! assert!(rx.pop().is_some());
//! ```
//!
//! ```
//! // SPMC - one producer, multiple consumers
//! use nexus_queue::spmc;
//!
//! let (tx, rx) = spmc::ring_buffer::<u64>(1024);
//! let rx2 = rx.clone();  // Clone for second consumer
//!
//! tx.push(1).unwrap();
//! tx.push(2).unwrap();
//!
//! // Each value consumed by exactly one consumer
//! assert!(rx.pop().is_some());
//! assert!(rx2.pop().is_some());
//! ```
//!
//! # Design
//!
//! ## SPSC
//!
//! Uses cached head/tail indices with separate cache lines to avoid false sharing.
//! Producer and consumer each maintain a local copy of the other's index, only
//! refreshing from the atomic when their cache indicates the queue is full
//! (producer) or empty (consumer).
//!
//! ## MPSC
//!
//! Uses CAS-based slot claiming with Vyukov-style turn counters. Producers compete
//! via CAS on the tail index, then wait for their slot's turn counter before
//! writing. This provides backpressure (try_push fails when full) without blocking.
//!
//! ## SPMC
//!
//! Mirror of MPSC with roles swapped. The single producer writes directly (no CAS),
//! while consumers compete via CAS on the head index. Eliminates producer-side
//! contention for fan-out workloads like 1 IO thread → N parser threads.
//!
//! All designs perform well on multi-socket NUMA systems where cache line
//! ownership is important for latency.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs, missing_debug_implementations)]

use core::fmt;

pub mod mpsc;
pub mod spmc;
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
