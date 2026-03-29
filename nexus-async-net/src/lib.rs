//! nexus-async-net — async WebSocket adapter for nexus-net.
//!
//! Thin tokio wrapper over nexus-net's synchronous WebSocket primitives.
//! Same zero-copy FrameReader, same Message type, same performance.
//!
//! # Usage
//!
//! ```ignore
//! use nexus_async_net::ws::WsStream;
//!
//! let mut ws = WsStream::connect("wss://exchange.com/ws").await?;
//! ws.send_text("Hello!").await?;
//!
//! while let Some(msg) = ws.recv().await? {
//!     // msg is nexus_net::ws::Message<'_>
//! }
//! ```

pub mod ws;

// Re-export nexus-net types for convenience
pub use nexus_net;
