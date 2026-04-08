//! Async WebSocket — adapts nexus-net for async runtimes.
//!
//! Both the `tokio-rt` and `nexus` backends expose `WsStream` and
//! `WsStreamBuilder` with the same `recv()`/`send_*()` API.
//!
//! The tokio backend additionally implements `futures_core::Stream` and
//! `futures_sink::Sink` for ecosystem compatibility. The nexus backend
//! omits these intentionally — direct `recv()`/`send_*()` methods are
//! preferred for latency-sensitive single-threaded code.

#[cfg(feature = "nexus")]
mod nexus;
#[cfg(feature = "tokio-rt")]
mod tokio;

#[cfg(feature = "nexus")]
pub use self::nexus::*;
#[cfg(feature = "tokio-rt")]
pub use self::tokio::*;
