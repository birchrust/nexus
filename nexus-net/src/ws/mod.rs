//! WebSocket framing — sans-IO encode/decode per RFC 6455.
//!
//! Three layers:
//! - **Message API**: [`Message`], [`OwnedMessage`], [`CloseCode`]
//! - **Wire parser**: [`FrameReader`] (ReadBuf → Message)
//! - **I/O wrapper**: [`WsStream`] (socket + handshake + reader/writer)
//!
//! Use `FrameReader`/`FrameWriter` directly for sans-IO integration.
//! Use `WsStream` for the convenience path with built-in HTTP upgrade.

mod error;
pub(crate) mod frame;
mod frame_reader;
mod frame_writer;
pub(crate) mod handshake;
pub(crate) mod mask;
mod message;
mod stream;
#[cfg(feature = "tls")]
mod tls_stream;

// User-facing types
pub use error::ProtocolError;
pub use frame::Role;
pub use frame_reader::{FrameReader, FrameReaderBuilder, ReadError};
pub use frame_writer::{FrameHeader, FrameWriter};
pub use handshake::HandshakeError;
pub use mask::apply_mask;
pub use message::{CloseCode, CloseFrame, Message, OwnedCloseFrame, OwnedMessage};
pub use stream::{WsError, WsStream, WsStreamBuilder};
#[cfg(feature = "tls")]
pub use tls_stream::WsTlsStream;
