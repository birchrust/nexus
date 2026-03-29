//! REST response — borrows from the ResponseReader's buffer.

use crate::http::ResponseReader;

/// HTTP response. Borrows from the connection's ResponseReader.
///
/// Must be dropped before the next request on the same connection
/// (same pattern as WebSocket `Message<'_>`).
pub struct RestResponse<'a> {
    pub(crate) status: u16,
    pub(crate) body_len: usize,
    pub(crate) resp_reader: &'a ResponseReader,
}

impl RestResponse<'_> {
    /// HTTP status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Look up a response header by name (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.resp_reader.header(name)
    }

    /// Response body as bytes.
    pub fn body(&self) -> &[u8] {
        let remainder = self.resp_reader.remainder();
        &remainder[..self.body_len.min(remainder.len())]
    }

    /// Response body as a UTF-8 string.
    pub fn body_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.body())
    }

    /// Response body length (from Content-Length header).
    pub fn body_len(&self) -> usize {
        self.body_len
    }

    /// Number of response headers.
    pub fn header_count(&self) -> usize {
        self.resp_reader.header_count()
    }
}

impl std::fmt::Debug for RestResponse<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestResponse")
            .field("status", &self.status)
            .field("body_len", &self.body_len)
            .finish()
    }
}
