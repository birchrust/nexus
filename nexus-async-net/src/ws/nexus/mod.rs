//! Async WebSocket — nexus-async-rt adapter for nexus-net.
//!
//! Same FrameReader, same zero-copy Message, same performance.
//! The only difference is `.await` on socket I/O.

mod stream;

pub use crate::maybe_tls::MaybeTls;
pub use stream::{WsStream, WsStreamBuilder};
