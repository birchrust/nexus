//! Async REST client adapter for nexus-async-rt.
//!
//! Adds async `send()` to [`Client<S>`](super::Client) when `S`
//! implements [`AsyncRead`] + [`AsyncWrite`].

use std::io;
use std::pin::Pin;

use nexus_async_rt::{AsyncRead, AsyncWrite};

use super::connection::{Client, ClientBuilder};
use super::error::RestError;
use super::request::Request;
use super::response::RestResponse;
use crate::http::{HttpError, ResponseReader};

// =============================================================================
// Async I/O helpers
// =============================================================================

async fn read_async<S: AsyncRead + Unpin>(stream: &mut S, buf: &mut [u8]) -> io::Result<usize> {
    std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_read(cx, buf)).await
}

async fn write_all_async<S: AsyncWrite + Unpin>(
    stream: &mut S,
    mut buf: &[u8],
) -> io::Result<()> {
    while !buf.is_empty() {
        let n = std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_write(cx, buf)).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "write returned 0",
            ));
        }
        buf = &buf[n..];
    }
    std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_flush(cx)).await
}

// =============================================================================
// Async impl on Client
// =============================================================================

impl<S: AsyncRead + AsyncWrite + Unpin> Client<S> {
    /// Send a request and read the response.
    ///
    /// Same API as the blocking [`Client::send`] but with `.await` on I/O.
    ///
    /// `Response` borrows from `reader` — drop before next send.
    #[allow(clippy::needless_pass_by_value)]
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

        // Read response — poison on any error, diagnose timeouts.
        match async_read_response(&mut self.stream, &mut self.poisoned, reader).await {
            Ok(resp) => Ok(resp),
            Err(e) => {
                self.poisoned = true;
                Err(diagnose_error(e))
            }
        }
    }
}

// =============================================================================
// Async builder methods
// =============================================================================

impl ClientBuilder {
    /// Connect with a pre-connected async stream.
    pub fn connect_with<S: AsyncRead + AsyncWrite + Unpin>(self, stream: S) -> Client<S> {
        Client::new(stream)
    }
}

// =============================================================================
// Internal — async response reading
// =============================================================================

/// Cold path: diagnose send failure.
#[cold]
fn diagnose_error(err: RestError) -> RestError {
    if let RestError::Io(ref io_err) = err {
        if io_err.kind() == std::io::ErrorKind::TimedOut
            || io_err.kind() == std::io::ErrorKind::WouldBlock
        {
            return RestError::ConnectionStale;
        }
    }
    err
}

async fn async_read_response<'r, S: AsyncRead + Unpin>(
    stream: &mut S,
    poisoned: &mut bool,
    reader: &'r mut ResponseReader,
) -> Result<RestResponse<'r>, RestError> {
    reader.consume_response();

    let mut tmp = [0u8; 4096];
    loop {
        match reader.next() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(e) => {
                *poisoned = true;
                return Err(e.into());
            }
        }
        match read_async(stream, &mut tmp).await {
            Ok(0) => {
                *poisoned = true;
                return Err(RestError::ConnectionClosed(
                    "server closed before response headers",
                ));
            }
            Ok(n) => {
                if let Err(e) = reader.read(&tmp[..n]) {
                    *poisoned = true;
                    return Err(e.into());
                }
            }
            Err(e) => {
                *poisoned = true;
                return Err(RestError::Io(e));
            }
        }
    }

    let status = reader.status();

    // RFC 7230: 1xx, 204, 304 have no body.
    if matches!(status, 100..=199 | 204 | 304) {
        reader.set_body_consumed(0);
        return Ok(RestResponse::new(status, 0, reader));
    }

    if reader.is_chunked() {
        let body = async_read_chunked_body(stream, poisoned, reader).await?;
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
            *poisoned = true;
            return Err(RestError::Http(HttpError::Malformed(
                "no Content-Length and not chunked",
            )));
        }
    };

    let max_body = reader.max_body_size_limit();
    if max_body > 0 && content_length > max_body {
        *poisoned = true;
        return Err(RestError::BodyTooLarge {
            size: content_length,
            max: max_body,
        });
    }

    // Read remaining body bytes.
    while reader.body_remaining() < content_length {
        match read_async(stream, &mut tmp).await {
            Ok(0) => {
                *poisoned = true;
                return Err(RestError::ConnectionClosed(
                    "server closed during body read",
                ));
            }
            Ok(n) => {
                if let Err(e) = reader.read(&tmp[..n]) {
                    *poisoned = true;
                    return Err(e.into());
                }
            }
            Err(e) => {
                *poisoned = true;
                return Err(RestError::Io(e));
            }
        }
    }

    reader.set_body_consumed(content_length);
    Ok(RestResponse::new(status, content_length, reader))
}

async fn async_read_chunked_body<S: AsyncRead + Unpin>(
    stream: &mut S,
    poisoned: &mut bool,
    reader: &ResponseReader,
) -> Result<Vec<u8>, RestError> {
    use crate::http::ChunkedDecoder;

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
                    *poisoned = true;
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
        let n = match read_async(stream, &mut wire_buf).await {
            Ok(0) => {
                *poisoned = true;
                return Err(RestError::ConnectionClosed(
                    "server closed during chunked body",
                ));
            }
            Ok(n) => n,
            Err(e) => {
                *poisoned = true;
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
                    *poisoned = true;
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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_async_rt::{AsyncRead, AsyncWrite};
    use std::io::Cursor;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    struct MockStream {
        written: Vec<u8>,
        response: Cursor<Vec<u8>>,
    }

    impl MockStream {
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

    impl AsyncRead for MockStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<io::Result<usize>> {
            let n = std::io::Read::read(&mut self.response, buf)?;
            Poll::Ready(Ok(n))
        }
    }

    impl AsyncWrite for MockStream {
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

    fn block_on_mock<F: std::future::Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(val) => val,
            Poll::Pending => panic!("mock future returned Pending"),
        }
    }

    fn noop_waker() -> std::task::Waker {
        use std::task::{RawWaker, RawWakerVTable};
        const VTABLE: RawWakerVTable =
            RawWakerVTable::new(|p| RawWaker::new(p, &VTABLE), |_| {}, |_| {}, |_| {});
        unsafe { std::task::Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
    }

    #[test]
    fn get_request() {
        use super::super::request::RequestWriter;

        let mock = MockStream::new(&ok_response(r#"{"ok":true}"#));
        let mut writer = RequestWriter::new("api.example.com").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = Client::new(mock);

        let req = writer.get("/status").finish().unwrap();
        let resp = block_on_mock(conn.send(req, &mut reader)).unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.body_str().unwrap(), r#"{"ok":true}"#);

        let written = conn.stream().written_str();
        assert!(written.starts_with("GET /status HTTP/1.1\r\n"));
        assert!(written.contains("Host: api.example.com\r\n"));
    }

    #[test]
    fn post_with_body() {
        use super::super::request::RequestWriter;

        let mock = MockStream::new(&ok_response(r#"{"filled":true}"#));
        let mut writer = RequestWriter::new("api.example.com").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = Client::new(mock);

        let body = br#"{"symbol":"BTC","side":"buy"}"#;
        let req = writer.post("/order").body(body).finish().unwrap();
        let resp = block_on_mock(conn.send(req, &mut reader)).unwrap();
        assert_eq!(resp.status(), 200);

        let written = conn.stream().written_str();
        assert!(written.contains(&format!("Content-Length: {}\r\n", body.len())));
    }

    #[test]
    fn response_headers() {
        use super::super::request::RequestWriter;

        let resp_bytes = b"HTTP/1.1 200 OK\r\nX-Request-Id: abc\r\nContent-Length: 2\r\n\r\n{}";
        let mock = MockStream::new(resp_bytes);
        let mut writer = RequestWriter::new("host").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = Client::new(mock);

        let req = writer.get("/test").finish().unwrap();
        let resp = block_on_mock(conn.send(req, &mut reader)).unwrap();
        assert_eq!(resp.header("X-Request-Id"), Some("abc"));
    }

    #[test]
    fn connection_poisoned() {
        use super::super::request::RequestWriter;

        let resp_bytes = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial";
        let mock = MockStream::new(resp_bytes);
        let mut writer = RequestWriter::new("host").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = Client::new(mock);

        let req = writer.get("/test").finish().unwrap();
        let result = block_on_mock(conn.send(req, &mut reader));
        assert!(matches!(result, Err(RestError::ConnectionClosed(_))));

        let req = writer.get("/test2").finish().unwrap();
        let result = block_on_mock(conn.send(req, &mut reader));
        assert!(matches!(result, Err(RestError::ConnectionPoisoned)));
    }

    #[test]
    fn chunked_decoded() {
        use super::super::request::RequestWriter;

        let resp_bytes =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let mock = MockStream::new(resp_bytes);
        let mut writer = RequestWriter::new("host").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = Client::new(mock);

        let req = writer.get("/test").finish().unwrap();
        let resp = block_on_mock(conn.send(req, &mut reader)).unwrap();
        assert_eq!(resp.body_str().unwrap(), "hello");
    }

    #[test]
    fn status_204_no_body() {
        use super::super::request::RequestWriter;

        let resp_bytes = b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n";
        let mock = MockStream::new(resp_bytes);
        let mut writer = RequestWriter::new("host").unwrap();
        let mut reader = ResponseReader::new(4096);
        let mut conn = Client::new(mock);

        let req = writer.get("/test").finish().unwrap();
        let resp = block_on_mock(conn.send(req, &mut reader)).unwrap();
        assert_eq!(resp.status(), 204);
        assert_eq!(resp.body().len(), 0);
    }
}
