//! Sans-IO HTTP/1.1 REST primitives + blocking transport.
//!
//! **Protocol layer (sans-IO):**
//! - [`RequestWriter`] — typestate request encoder, produces [`Request`]
//! - [`ResponseReader`](crate::http::ResponseReader) — response parser
//!
//! **Transport layer (blocking I/O):**
//! - [`HttpConnection`] — sends request bytes, reads response bytes
//!
//! # Usage
//!
//! ```ignore
//! use nexus_net::rest::{HttpConnection, RequestWriter};
//! use nexus_net::http::ResponseReader;
//!
//! // Protocol (sans-IO)
//! let mut writer = RequestWriter::new("api.exchange.com");
//! writer.default_header("Authorization", "Bearer token123")?;
//! let mut reader = ResponseReader::new(32 * 1024);
//!
//! // Transport
//! let mut conn = HttpConnection::connect("http://api.exchange.com")?;
//!
//! // Build + send
//! let req = writer.get("/api/v1/orders")
//!     .query("symbol", "BTC-USD")
//!     .send()?;
//! let resp = conn.send(req, &mut reader, 32 * 1024)?;
//!
//! println!("status: {}", resp.status());
//! println!("body: {}", resp.body_str()?);
//! ```

mod connection;
mod error;
mod request;
mod response;

pub use connection::{HttpConnection, HttpConnectionBuilder, ParsedUrl, parse_base_url};
pub use error::RestError;
pub use request::{Headers, Method, Query, Ready, Request, RequestBuilder, RequestWriter};
pub use response::RestResponse;
