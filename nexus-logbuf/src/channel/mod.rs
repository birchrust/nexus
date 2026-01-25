//! Channel API with parking for receivers.
//!
//! Wraps the low-level [`queue`](crate::queue) primitives with ergonomic
//! blocking semantics for receivers.
//!
//! # Philosophy
//!
//! **Senders are never slowed down.** They use immediate `try_send()` and never
//! make syscalls. If the buffer is full, they return an error immediately.
//! Users who want retry-with-backoff can implement their own loop.
//!
//! **Receivers can block.** They use `park_timeout` to wait for messages
//! without burning CPU. The timeout ensures they periodically check for
//! disconnection and don't block forever.
//!
//! # API Pattern
//!
//! Both channel variants provide:
//! - `try_send()` / `try_recv()` — immediate, non-blocking
//! - `notify()` — wake a parked receiver (call after sending)
//! - `wait()` / `wait_timeout()` — block waiting for data (receivers only)
//!
//! The blocking receive pattern:
//! ```ignore
//! loop {
//!     if let Some(record) = rx.try_recv() {
//!         // process record
//!         break;
//!     }
//!     if rx.wait() {
//!         break; // disconnected
//!     }
//! }
//! ```
//!
//! # Variants
//!
//! - [`spsc`]: Single-producer, single-consumer channel.
//! - [`mpsc`]: Multi-producer, single-consumer channel.

pub mod mpsc;
pub mod spsc;
