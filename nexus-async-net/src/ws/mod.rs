//! Async WebSocket — tokio adapter for nexus-net.
//!
//! Same FrameReader, same zero-copy Message, same performance.
//! The only difference is `.await` on socket I/O.

mod maybe_tls;
mod stream;

pub use maybe_tls::MaybeTls;
pub use stream::{WsStream, WsStreamBuilder};
