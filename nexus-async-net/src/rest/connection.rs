//! Async HTTP/1.1 keep-alive connection — pure transport.

use nexus_net::http::{HttpError, ResponseReader};
use nexus_net::rest::{Request, RestError, RestResponse};
use nexus_net::tls::TlsConfig;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::maybe_tls::MaybeTls;

// =============================================================================
// Builder
// =============================================================================

/// Builder for [`AsyncHttpConnection`].
pub struct AsyncHttpConnectionBuilder {
    tls_config: Option<TlsConfig>,
    nodelay: bool,
}

impl AsyncHttpConnectionBuilder {
    /// Create a new builder with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tls_config: None,
            nodelay: false,
        }
    }

    /// Custom TLS configuration.
    #[must_use]
    pub fn tls(mut self, config: &TlsConfig) -> Self {
        self.tls_config = Some(config.clone());
        self
    }

    /// Set TCP_NODELAY.
    #[must_use]
    pub fn disable_nagle(mut self) -> Self {
        self.nodelay = true;
        self
    }

    /// Connect to an HTTP(S) endpoint. TLS auto-detected from scheme.
    pub async fn connect(self, url: &str) -> Result<AsyncHttpConnection<MaybeTls>, RestError> {
        let parsed = nexus_net::rest::parse_base_url(url)?;
        let addr = format!("{}:{}", parsed.host, parsed.port);

        let tcp = TcpStream::connect(&addr).await?;
        if self.nodelay {
            tcp.set_nodelay(true)?;
        }

        let stream = if parsed.tls {
            let tls_config = match &self.tls_config {
                Some(c) => c.clone(),
                None => TlsConfig::new().map_err(RestError::Tls)?,
            };

            let connector =
                tokio_rustls::TlsConnector::from(tls_config.client_config().clone());
            let server_name =
                tokio_rustls::rustls::pki_types::ServerName::try_from(parsed.host.to_owned())
                    .map_err(|_| {
                        RestError::InvalidUrl(format!("invalid hostname: {}", parsed.host))
                    })?;
            let tls_stream = connector.connect(server_name, tcp).await.map_err(|e| {
                RestError::Io(e)
            })?;
            MaybeTls::Tls(Box::new(tls_stream))
        } else {
            MaybeTls::Plain(tcp)
        };

        Ok(AsyncHttpConnection {
            stream,
            poisoned: false,
        })
    }

    /// Connect with a pre-connected async stream.
    pub fn connect_with<S: AsyncRead + AsyncWrite + Unpin>(
        self,
        stream: S,
    ) -> AsyncHttpConnection<S> {
        AsyncHttpConnection {
            stream,
            poisoned: false,
        }
    }
}

impl Default for AsyncHttpConnectionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// AsyncHttpConnection — pure async transport
// =============================================================================

/// Async HTTP/1.1 keep-alive connection — pure transport.
///
/// Sends request bytes and reads response bytes over an async stream.
/// All protocol logic lives in [`RequestWriter`](nexus_net::rest::RequestWriter)
/// and [`ResponseReader`].
///
/// # Usage
///
/// ```ignore
/// use nexus_net::rest::RequestWriter;
/// use nexus_net::http::ResponseReader;
/// use nexus_async_net::rest::AsyncHttpConnection;
///
/// let mut writer = RequestWriter::new("api.binance.com").unwrap();
/// let mut reader = ResponseReader::new(32 * 1024);
/// let mut conn = AsyncHttpConnection::connect("https://api.binance.com").await?;
///
/// let req = writer.get("/orders").query("symbol", "BTC").finish()?;
/// let resp = conn.send(req, &mut reader).await?;
/// ```
pub struct AsyncHttpConnection<S> {
    stream: S,
    poisoned: bool,
}

impl AsyncHttpConnection<MaybeTls> {
    /// Connect with default configuration. TLS auto-detected from scheme.
    pub async fn connect(url: &str) -> Result<Self, RestError> {
        AsyncHttpConnectionBuilder::new().connect(url).await
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncHttpConnection<S> {
    /// Wrap a pre-connected async stream.
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            poisoned: false,
        }
    }

    /// Create a builder.
    #[must_use]
    pub fn builder() -> AsyncHttpConnectionBuilder {
        AsyncHttpConnectionBuilder::new()
    }

    /// Send a request and read the response.
    ///
    /// Same API as [`HttpConnection::send`](nexus_net::rest::HttpConnection::send)
    /// but with `.await` on I/O.
    #[allow(clippy::needless_pass_by_value)] // Move by design — request is consumed after send.
    pub async fn send<'r>(
        &mut self,
        req: Request<'_>,
        reader: &'r mut ResponseReader,
    ) -> Result<RestResponse<'r>, RestError> {
        if self.poisoned {
            return Err(RestError::ConnectionPoisoned);
        }

        // Send request bytes
        if let Err(e) = self.stream.write_all(req.data()).await {
            self.poisoned = true;
            return Err(RestError::Io(e));
        }
        if let Err(e) = self.stream.flush().await {
            self.poisoned = true;
            return Err(RestError::Io(e));
        }

        // Read response — poison on any error (matches sync handle_send_error)
        match self.read_response(reader).await {
            Ok(resp) => Ok(resp),
            Err(e) => {
                self.poisoned = true;
                Err(e)
            }
        }
    }

    /// Whether the connection is poisoned.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Access the underlying stream.
    pub fn stream(&self) -> &S {
        &self.stream
    }

    /// Mutable access to the underlying stream.
    pub fn stream_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    // =========================================================================
    // Internal — async response reading
    // =========================================================================

    async fn read_response<'r>(
        &mut self,
        reader: &'r mut ResponseReader,
    ) -> Result<RestResponse<'r>, RestError> {
        reader.consume_response();

        // Read until headers are complete.
        let mut tmp = [0u8; 4096];
        loop {
            match reader.next() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(e) => {
                    self.poisoned = true;
                    return Err(e.into());
                }
            }
            match self.stream.read(&mut tmp).await {
                Ok(0) => {
                    self.poisoned = true;
                    return Err(RestError::ConnectionClosed);
                }
                Ok(n) => {
                    if let Err(e) = reader.read(&tmp[..n]) {
                        self.poisoned = true;
                        return Err(e.into());
                    }
                }
                Err(e) => {
                    self.poisoned = true;
                    return Err(RestError::Io(e));
                }
            }
        }

        // Validate using cached values from try_parse.
        let status = reader.status();

        if matches!(status, 100..=199 | 204 | 304) {
            reader.set_body_consumed(0);
            return Ok(RestResponse::new(status, 0, reader));
        }

        if reader.is_chunked() {
            let body = self.read_chunked_body(reader).await?;
            reader.set_body_consumed(reader.body_remaining());
            return Ok(RestResponse::new_chunked(status, body, reader));
        }

        let content_length = match reader.content_length() {
            Some(Ok(n)) => n,
            Some(Err(())) => return Err(RestError::Http(HttpError::Malformed)),
            None => {
                // No Content-Length and not chunked — can't determine body
                // boundaries for keep-alive. Error instead of silent empty body.
                self.poisoned = true;
                return Err(RestError::Http(HttpError::Malformed));
            }
        };

        let max_body = reader.max_body_size_limit();
        if max_body > 0 && content_length > max_body {
            self.poisoned = true;
            return Err(RestError::BodyTooLarge {
                size: content_length,
                max: max_body,
            });
        }

        // Read remaining body bytes (Content-Length delimited).
        while reader.body_remaining() < content_length {
            match self.stream.read(&mut tmp).await {
                Ok(0) => {
                    self.poisoned = true;
                    return Err(RestError::ConnectionClosed);
                }
                Ok(n) => {
                    if let Err(e) = reader.read(&tmp[..n]) {
                        self.poisoned = true;
                        return Err(e.into());
                    }
                }
                Err(e) => {
                    self.poisoned = true;
                    return Err(RestError::Io(e));
                }
            }
        }

        reader.set_body_consumed(content_length);
        Ok(RestResponse::new(status, content_length, reader))
    }

    async fn read_chunked_body(
        &mut self,
        reader: &ResponseReader,
    ) -> Result<Vec<u8>, RestError> {
        use nexus_net::http::ChunkedDecoder;

        let max_body = reader.max_body_size_limit();
        let mut decoder = ChunkedDecoder::new();
        let mut body = Vec::with_capacity(4096);
        let mut wire_buf = [0u8; 4096];
        let mut decode_buf = [0u8; 4096];

        // Decode any chunk data that arrived with the headers.
        let remainder = reader.remainder();
        if !remainder.is_empty() {
            let mut pos = 0;
            while pos < remainder.len() && !decoder.is_done() {
                let (consumed, produced) = decoder
                    .decode(&remainder[pos..], &mut decode_buf)
                    .map_err(|_| RestError::Http(HttpError::Malformed))?;
                pos += consumed;
                if produced > 0 {
                    body.extend_from_slice(&decode_buf[..produced]);
                    if max_body > 0 && body.len() > max_body {
                        self.poisoned = true;
                        return Err(RestError::BodyTooLarge {
                            size: body.len(),
                            max: max_body,
                        });
                    }
                }
                if consumed == 0 && produced == 0 {
                    break;
                }
            }
        }

        while !decoder.is_done() {
            let n = match self.stream.read(&mut wire_buf).await {
                Ok(0) => {
                    self.poisoned = true;
                    return Err(RestError::ConnectionClosed);
                }
                Ok(n) => n,
                Err(e) => {
                    self.poisoned = true;
                    return Err(RestError::Io(e));
                }
            };

            let mut pos = 0;
            while pos < n && !decoder.is_done() {
                let (consumed, produced) = decoder
                    .decode(&wire_buf[pos..n], &mut decode_buf)
                    .map_err(|_| RestError::Http(HttpError::Malformed))?;
                pos += consumed;
                if produced > 0 {
                    body.extend_from_slice(&decode_buf[..produced]);
                    if max_body > 0 && body.len() > max_body {
                        self.poisoned = true;
                        return Err(RestError::BodyTooLarge {
                            size: body.len(),
                            max: max_body,
                        });
                    }
                }
                if consumed == 0 && produced == 0 {
                    break;
                }
            }
        }

        Ok(body)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::ReadBuf;

    struct MockAsyncStream {
        written: Vec<u8>,
        response: Cursor<Vec<u8>>,
    }

    impl MockAsyncStream {
        fn new(response: &[u8]) -> Self {
            Self {
                written: Vec::new(),
                response: Cursor::new(response.to_vec()),
            }
        }

        fn written_str(&self) -> &str {
            std::str::from_utf8(&self.written).unwrap()
        }
    }

    impl AsyncRead for MockAsyncStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let n = std::io::Read::read(&mut self.response, buf.initialize_unfilled())?;
            buf.advance(n);
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for MockAsyncStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.written.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn ok_response(body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn async_get_request() {
        use nexus_net::rest::RequestWriter;

        let mock = MockAsyncStream::new(&ok_response(r#"{"ok":true}"#));
        let mut writer = RequestWriter::new("api.example.com").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = AsyncHttpConnection::new(mock);

        let req = writer.get("/status").finish().unwrap();
        let resp = conn.send(req, &mut reader).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.body_str().unwrap(), r#"{"ok":true}"#);

        let written = conn.stream().written_str();
        assert!(written.starts_with("GET /status HTTP/1.1\r\n"));
        assert!(written.contains("Host: api.example.com\r\n"));
    }

    #[tokio::test]
    async fn async_post_with_body() {
        use nexus_net::rest::RequestWriter;

        let mock = MockAsyncStream::new(&ok_response(r#"{"filled":true}"#));
        let mut writer = RequestWriter::new("api.example.com").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = AsyncHttpConnection::new(mock);

        let body = br#"{"symbol":"BTC","side":"buy"}"#;
        let req = writer.post("/order").body(body).finish().unwrap();
        let resp = conn.send(req, &mut reader).await.unwrap();
        assert_eq!(resp.status(), 200);

        let written = conn.stream().written_str();
        assert!(written.contains(&format!("Content-Length: {}\r\n", body.len())));
        assert!(written.ends_with(std::str::from_utf8(body).unwrap()));
    }

    #[tokio::test]
    async fn async_response_headers() {
        use nexus_net::rest::RequestWriter;

        let resp_bytes = b"HTTP/1.1 200 OK\r\nX-Request-Id: abc\r\nContent-Length: 2\r\n\r\n{}";
        let mock = MockAsyncStream::new(resp_bytes);
        let mut writer = RequestWriter::new("host").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = AsyncHttpConnection::new(mock);

        let req = writer.get("/test").finish().unwrap();
        let resp = conn.send(req, &mut reader).await.unwrap();
        assert_eq!(resp.header("X-Request-Id"), Some("abc"));
    }

    #[tokio::test]
    async fn async_connection_poisoned() {
        use nexus_net::rest::RequestWriter;

        // Response with Content-Length: 100 but only partial body → EOF
        let resp_bytes = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial";
        let mock = MockAsyncStream::new(resp_bytes);
        let mut writer = RequestWriter::new("host").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = AsyncHttpConnection::new(mock);

        let req = writer.get("/test").finish().unwrap();
        let result = conn.send(req, &mut reader).await;
        assert!(matches!(result, Err(RestError::ConnectionClosed)));

        let req = writer.get("/test2").finish().unwrap();
        let result = conn.send(req, &mut reader).await;
        assert!(matches!(result, Err(RestError::ConnectionPoisoned)));
    }

    #[tokio::test]
    async fn async_chunked_decoded() {
        use nexus_net::rest::RequestWriter;

        let resp_bytes = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let mock = MockAsyncStream::new(resp_bytes);
        let mut writer = RequestWriter::new("host").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = AsyncHttpConnection::new(mock);

        let req = writer.get("/test").finish().unwrap();
        let resp = conn.send(req, &mut reader).await.unwrap();
        assert_eq!(resp.body_str().unwrap(), "hello");
    }
}
