//! Async HTTP/1.1 keep-alive connection -- nexus-async-rt backend.

use std::io;
use std::net::ToSocketAddrs;
use std::pin::Pin;

use nexus_async_rt::{AsyncRead, AsyncWrite, TcpStream};
use nexus_net::http::{HttpError, ResponseReader};
use nexus_net::rest::{Request, RestError, RestResponse};
#[cfg(feature = "tls")]
use nexus_net::tls::TlsConfig;

use crate::maybe_tls::MaybeTls;

// =============================================================================
// Async I/O helpers (poll_fn wrappers)
// =============================================================================

async fn read_async<S: AsyncRead + Unpin>(s: &mut S, buf: &mut [u8]) -> io::Result<usize> {
    std::future::poll_fn(|cx| Pin::new(&mut *s).poll_read(cx, buf)).await
}

async fn write_all_async<S: AsyncWrite + Unpin>(s: &mut S, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        let n = std::future::poll_fn(|cx| Pin::new(&mut *s).poll_write(cx, buf)).await?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "write returned 0"));
        }
        buf = &buf[n..];
    }
    Ok(())
}

async fn flush_async<S: AsyncWrite + Unpin>(s: &mut S) -> io::Result<()> {
    std::future::poll_fn(|cx| Pin::new(&mut *s).poll_flush(cx)).await
}

// =============================================================================
// TLS handshake (sans-IO codec driven over async transport)
// =============================================================================

#[cfg(feature = "tls")]
#[allow(clippy::future_not_send)]
async fn handshake_tls(
    stream: &mut TcpStream,
    codec: &mut nexus_net::tls::TlsCodec,
) -> Result<(), nexus_net::tls::TlsError> {
    let mut tmp = [0u8; 8192];

    while codec.is_handshaking() {
        if codec.wants_write() {
            let mut buf = Vec::new();
            codec.write_tls_to(&mut buf)?;
            write_all_async(stream, &buf)
                .await
                .map_err(nexus_net::tls::TlsError::Io)?;
            flush_async(stream)
                .await
                .map_err(nexus_net::tls::TlsError::Io)?;
        }
        if codec.wants_read() {
            let n = read_async(stream, &mut tmp)
                .await
                .map_err(nexus_net::tls::TlsError::Io)?;
            if n == 0 {
                return Err(nexus_net::tls::TlsError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "closed during TLS handshake",
                )));
            }
            codec.read_tls(&tmp[..n])?;
            codec.process_new_packets()?;
        }
    }

    // Flush any remaining handshake data.
    if codec.wants_write() {
        let mut buf = Vec::new();
        codec.write_tls_to(&mut buf)?;
        write_all_async(stream, &buf)
            .await
            .map_err(nexus_net::tls::TlsError::Io)?;
        flush_async(stream)
            .await
            .map_err(nexus_net::tls::TlsError::Io)?;
    }

    Ok(())
}

// =============================================================================
// Builder
// =============================================================================

/// Builder for [`AsyncHttpConnection`].
pub struct AsyncHttpConnectionBuilder {
    #[cfg(feature = "tls")]
    tls_config: Option<TlsConfig>,
    nodelay: bool,
    connect_timeout: Option<std::time::Duration>,
    #[cfg(feature = "socket-opts")]
    tcp_keepalive: Option<std::time::Duration>,
    #[cfg(feature = "socket-opts")]
    recv_buf_size: Option<usize>,
    #[cfg(feature = "socket-opts")]
    send_buf_size: Option<usize>,
}

impl AsyncHttpConnectionBuilder {
    /// Create a new builder with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "tls")]
            tls_config: None,
            nodelay: false,
            connect_timeout: None,
            #[cfg(feature = "socket-opts")]
            tcp_keepalive: None,
            #[cfg(feature = "socket-opts")]
            recv_buf_size: None,
            #[cfg(feature = "socket-opts")]
            send_buf_size: None,
        }
    }

    /// Custom TLS configuration.
    #[cfg(feature = "tls")]
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

    /// TCP connect timeout.
    #[must_use]
    pub fn connect_timeout(mut self, d: std::time::Duration) -> Self {
        self.connect_timeout = Some(d);
        self
    }

    /// Set TCP keepalive idle time.
    #[cfg(feature = "socket-opts")]
    #[must_use]
    pub fn tcp_keepalive(mut self, idle: std::time::Duration) -> Self {
        self.tcp_keepalive = Some(idle);
        self
    }

    /// Set `SO_RCVBUF` (socket receive buffer size).
    #[cfg(feature = "socket-opts")]
    #[must_use]
    pub fn recv_buffer_size(mut self, n: usize) -> Self {
        self.recv_buf_size = Some(n);
        self
    }

    /// Set `SO_SNDBUF` (socket send buffer size).
    #[cfg(feature = "socket-opts")]
    #[must_use]
    pub fn send_buffer_size(mut self, n: usize) -> Self {
        self.send_buf_size = Some(n);
        self
    }

    /// Connect to an HTTP(S) endpoint. TLS auto-detected from scheme.
    ///
    /// DNS resolution uses blocking `ToSocketAddrs` (cold path).
    /// TCP connect uses `nexus_async_rt::TcpStream::connect` (mio, non-blocking).
    #[allow(clippy::future_not_send)]
    pub async fn connect(self, url: &str) -> Result<AsyncHttpConnection<MaybeTls>, RestError> {
        let parsed = nexus_net::rest::parse_base_url(url)?;
        let addr_str = format!("{}:{}", parsed.host, parsed.port);
        let addr = addr_str
            .to_socket_addrs()
            .map_err(RestError::Io)?
            .next()
            .ok_or_else(|| RestError::InvalidUrl(format!("DNS resolution failed: {addr_str}")))?;

        let connect_fn = async {
            let tcp = TcpStream::connect(addr, nexus_async_rt::io())?;
            Ok::<TcpStream, RestError>(tcp)
        };

        #[allow(unused_mut)] // mut needed when tls feature is enabled
        let mut tcp = match self.connect_timeout {
            Some(dur) => nexus_async_rt::timeout(dur, connect_fn)
                .await
                .map_err(|_| {
                    RestError::Io(io::Error::new(io::ErrorKind::TimedOut, "connect timeout"))
                })??,
            None => connect_fn.await?,
        };

        if self.nodelay {
            tcp.set_nodelay(true)?;
        }
        #[cfg(feature = "socket-opts")]
        self.apply_socket_opts(&tcp)?;

        let stream = if parsed.tls {
            #[cfg(feature = "tls")]
            {
                let tls_config = match &self.tls_config {
                    Some(c) => c.clone(),
                    None => TlsConfig::new().map_err(RestError::Tls)?,
                };

                let mut codec = nexus_net::tls::TlsCodec::new(&tls_config, parsed.host)?;

                handshake_tls(&mut tcp, &mut codec).await?;

                let tls_inner = crate::maybe_tls::TlsInner::new(tcp, codec);
                MaybeTls::Tls(Box::new(tls_inner))
            }
            #[cfg(not(feature = "tls"))]
            {
                return Err(RestError::TlsNotEnabled);
            }
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

#[cfg(feature = "socket-opts")]
impl AsyncHttpConnectionBuilder {
    fn apply_socket_opts(&self, tcp: &TcpStream) -> Result<(), RestError> {
        use std::os::fd::AsFd;
        let fd = tcp.as_fd();
        let sock = socket2::SockRef::from(&fd);
        if let Some(idle) = self.tcp_keepalive {
            let keepalive = socket2::TcpKeepalive::new().with_time(idle);
            sock.set_tcp_keepalive(&keepalive).map_err(RestError::Io)?;
        }
        if let Some(size) = self.recv_buf_size {
            sock.set_recv_buffer_size(size).map_err(RestError::Io)?;
        }
        if let Some(size) = self.send_buf_size {
            sock.set_send_buffer_size(size).map_err(RestError::Io)?;
        }
        Ok(())
    }
}

impl Default for AsyncHttpConnectionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// AsyncHttpConnection -- pure async transport
// =============================================================================

/// Async HTTP/1.1 keep-alive connection -- pure transport.
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
/// use nexus_async_net::rest::{AsyncHttpConnection, AsyncHttpConnectionBuilder};
/// use nexus_net::tls::TlsConfig;
///
/// let mut writer = RequestWriter::new("api.binance.com").unwrap();
/// let mut reader = ResponseReader::new(32 * 1024);
/// let tls = TlsConfig::new()?;
/// let mut conn = AsyncHttpConnectionBuilder::new()
///     .tls(&tls)
///     .connect("https://api.binance.com")
///     .await?;
///
/// let req = writer.get("/orders").query("symbol", "BTC").finish()?;
/// let resp = conn.send(req, &mut reader).await?;
/// ```
pub struct AsyncHttpConnection<S> {
    stream: S,
    poisoned: bool,
}

// MaybeTls connections are created exclusively through `AsyncHttpConnectionBuilder`.

#[allow(clippy::future_not_send)]
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
    /// Same API as [`Client::send`](nexus_net::rest::Client::send)
    /// but with `.await` on I/O.
    #[allow(clippy::needless_pass_by_value)] // Move by design -- request is consumed after send.
    pub async fn send<'r>(
        &mut self,
        req: Request<'_>,
        reader: &'r mut ResponseReader,
    ) -> Result<RestResponse<'r>, RestError> {
        if self.poisoned {
            return Err(RestError::ConnectionPoisoned);
        }

        // Send request bytes
        if let Err(e) = write_all_async(&mut self.stream, req.as_bytes()).await {
            self.poisoned = true;
            return Err(RestError::Io(e));
        }
        if let Err(e) = flush_async(&mut self.stream).await {
            self.poisoned = true;
            return Err(RestError::Io(e));
        }

        // Read response -- poison on any error, diagnose timeouts.
        match self.read_response(reader).await {
            Ok(resp) => Ok(resp),
            Err(e) => {
                self.poisoned = true;
                Err(self.diagnose_error(e))
            }
        }
    }

    /// Whether the connection is poisoned.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Cold path: diagnose send failure. Matches sync `handle_send_error`.
    #[cold]
    #[allow(clippy::unused_self)]
    fn diagnose_error(&self, err: RestError) -> RestError {
        if let RestError::Io(ref io_err) = err {
            if io_err.kind() == io::ErrorKind::TimedOut
                || io_err.kind() == io::ErrorKind::WouldBlock
            {
                return RestError::ConnectionStale;
            }
        }
        err
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
    // Internal -- async response reading
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
            match read_async(&mut self.stream, &mut tmp).await {
                Ok(0) => {
                    self.poisoned = true;
                    return Err(RestError::ConnectionClosed(
                        "server closed before response headers",
                    ));
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
            Some(Err(())) => {
                return Err(RestError::Http(HttpError::Malformed(
                    "invalid Content-Length header",
                )));
            }
            None => {
                self.poisoned = true;
                return Err(RestError::Http(HttpError::Malformed(
                    "no Content-Length and not chunked",
                )));
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
            match read_async(&mut self.stream, &mut tmp).await {
                Ok(0) => {
                    self.poisoned = true;
                    return Err(RestError::ConnectionClosed(
                        "server closed during body read",
                    ));
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

    async fn read_chunked_body(&mut self, reader: &ResponseReader) -> Result<Vec<u8>, RestError> {
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
                    .map_err(RestError::Http)?;
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
            let n = match read_async(&mut self.stream, &mut wire_buf).await {
                Ok(0) => {
                    self.poisoned = true;
                    return Err(RestError::ConnectionClosed(
                        "server closed during chunked body",
                    ));
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
                    .map_err(RestError::Http)?;
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
            buf: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            let n = std::io::Read::read(&mut self.response, buf)?;
            Poll::Ready(Ok(n))
        }
    }

    impl AsyncWrite for MockAsyncStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.written.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
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

    // Mock stream tests work without a runtime -- poll_fn on a mock that
    // always returns Ready completes immediately in any executor.

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        // Minimal single-poll executor for futures that resolve immediately.
        use std::task::{RawWaker, RawWakerVTable, Waker};

        fn noop(_: *const ()) {}
        fn noop_clone(p: *const ()) -> RawWaker {
            RawWaker::new(p, &VTABLE)
        }
        const VTABLE: RawWakerVTable = RawWakerVTable::new(noop_clone, noop, noop, noop);

        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut f = std::pin::pin!(f);

        // Poll up to 1000 times (mock streams always return Ready).
        for _ in 0..1000 {
            match f.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => continue,
            }
        }
        panic!("mock future did not resolve within 1000 polls");
    }

    #[test]
    fn async_get_request() {
        use nexus_net::rest::RequestWriter;

        let mock = MockAsyncStream::new(&ok_response(r#"{"ok":true}"#));
        let mut writer = RequestWriter::new("api.example.com").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = AsyncHttpConnection::new(mock);

        block_on(async {
            let req = writer.get("/status").finish().unwrap();
            let resp = conn.send(req, &mut reader).await.unwrap();
            assert_eq!(resp.status(), 200);
            assert_eq!(resp.body_str().unwrap(), r#"{"ok":true}"#);

            let written = conn.stream().written_str();
            assert!(written.starts_with("GET /status HTTP/1.1\r\n"));
            assert!(written.contains("Host: api.example.com\r\n"));
        });
    }

    #[test]
    fn async_post_with_body() {
        use nexus_net::rest::RequestWriter;

        let mock = MockAsyncStream::new(&ok_response(r#"{"filled":true}"#));
        let mut writer = RequestWriter::new("api.example.com").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = AsyncHttpConnection::new(mock);

        block_on(async {
            let body = br#"{"symbol":"BTC","side":"buy"}"#;
            let req = writer.post("/order").body(body).finish().unwrap();
            let resp = conn.send(req, &mut reader).await.unwrap();
            assert_eq!(resp.status(), 200);

            let written = conn.stream().written_str();
            assert!(written.contains(&format!("Content-Length: {}\r\n", body.len())));
            assert!(written.ends_with(std::str::from_utf8(body).unwrap()));
        });
    }

    #[test]
    fn async_response_headers() {
        use nexus_net::rest::RequestWriter;

        let resp_bytes = b"HTTP/1.1 200 OK\r\nX-Request-Id: abc\r\nContent-Length: 2\r\n\r\n{}";
        let mock = MockAsyncStream::new(resp_bytes);
        let mut writer = RequestWriter::new("host").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = AsyncHttpConnection::new(mock);

        block_on(async {
            let req = writer.get("/test").finish().unwrap();
            let resp = conn.send(req, &mut reader).await.unwrap();
            assert_eq!(resp.header("X-Request-Id"), Some("abc"));
        });
    }

    #[test]
    fn async_connection_poisoned() {
        use nexus_net::rest::RequestWriter;

        // Response with Content-Length: 100 but only partial body -> EOF
        let resp_bytes = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial";
        let mock = MockAsyncStream::new(resp_bytes);
        let mut writer = RequestWriter::new("host").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = AsyncHttpConnection::new(mock);

        block_on(async {
            let req = writer.get("/test").finish().unwrap();
            let result = conn.send(req, &mut reader).await;
            assert!(matches!(result, Err(RestError::ConnectionClosed(_))));

            let req = writer.get("/test2").finish().unwrap();
            let result = conn.send(req, &mut reader).await;
            assert!(matches!(result, Err(RestError::ConnectionPoisoned)));
        });
    }

    #[test]
    fn async_chunked_decoded() {
        use nexus_net::rest::RequestWriter;

        let resp_bytes =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let mock = MockAsyncStream::new(resp_bytes);
        let mut writer = RequestWriter::new("host").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = AsyncHttpConnection::new(mock);

        block_on(async {
            let req = writer.get("/test").finish().unwrap();
            let resp = conn.send(req, &mut reader).await.unwrap();
            assert_eq!(resp.body_str().unwrap(), "hello");
        });
    }
}
