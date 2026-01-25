//! Low-level ring buffer primitives.
//!
//! These are the raw building blocks - no blocking, no backpressure handling.
//! For most use cases, prefer the [`channel`](crate::channel) module which
//! provides ergonomic blocking APIs.
//!
//! - [`spsc`]: Single-producer, single-consumer. Lowest latency.
//! - [`mpsc`]: Multi-producer, single-consumer. CAS on tail for claiming.

pub mod mpsc;
pub mod spsc;
