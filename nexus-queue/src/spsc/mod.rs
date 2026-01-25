//! Single-producer single-consumer bounded queue.
//!
//! A lock-free ring buffer optimized for exactly one producer thread and one
//! consumer thread. Uses cached indices to minimize atomic operations on the
//! hot path.
//!
//! # Example
//!
//! ```
//! use nexus_queue::spsc;
//!
//! let (mut tx, mut rx) = spsc::ring_buffer::<u64>(1024);
//!
//! tx.push(42).unwrap();
//! assert_eq!(rx.pop(), Some(42));
//! ```

mod index;

pub use index::{Consumer, Producer, ring_buffer};
