//! Sans-IO HTTP/1.x protocol primitives.
//!
//! Built on [`httparse`] for SIMD-accelerated header parsing.
//! Uses [`ReadBuf`](crate::buf::ReadBuf) for incremental byte buffering.
//!
//! - [`ResponseReader`] — parse inbound HTTP responses (used by REST client)
//! - [`ChunkedDecoder`] — chunked transfer encoding decoder
//! - [`write_request`] / [`write_response`] — construct outbound HTTP messages
//!
//! The HTTP client API is in [`rest`](crate::rest).
//! `RequestReader` is internal (used for WebSocket upgrade handshake).

mod chunked;
mod error;
mod request;
mod response;

pub use chunked::ChunkedDecoder;
pub use error::HttpError;
// RequestReader parses inbound HTTP requests (used for WS upgrade handshake).
// The public HTTP client API is in `rest::`.
pub use request::RequestReader;
pub use response::{
    Response, ResponseReader, request_size, response_size, write_request, write_response,
};
