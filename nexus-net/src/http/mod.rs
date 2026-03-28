//! Sans-IO HTTP/1.x request/response parsing.
//!
//! Built on [`httparse`] for SIMD-accelerated header parsing.
//! Uses [`ReadBuf`](crate::buf::ReadBuf) for incremental byte buffering.
//!
//! - [`RequestReader`] — parse inbound HTTP requests
//! - [`ResponseReader`] — parse inbound HTTP responses
//! - [`write_request`] / [`write_response`] — construct outbound HTTP messages

mod error;
mod request;
mod response;

pub use error::HttpError;
pub use request::{Request, RequestReader};
pub use response::{Response, ResponseReader, write_request, write_response};
