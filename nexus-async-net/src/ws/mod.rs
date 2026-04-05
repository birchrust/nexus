//! Async WebSocket — adapts nexus-net for async runtimes.

#[cfg(feature = "tokio-rt")]
mod tokio;

#[cfg(feature = "tokio-rt")]
pub use self::tokio::*;
