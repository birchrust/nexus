//! Async WebSocket adapter for tokio.
//!
//! Adds async methods to [`Client<S>`](super::Client) when `S`
//! implements tokio's [`AsyncRead`](tokio::io::AsyncRead) +
//! [`AsyncWrite`](tokio::io::AsyncWrite). Same zero-copy parsing,
//! same method names — the feature flag selects the impl.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::frame::Role;
use super::frame_reader::FrameReaderBuilder;
use super::frame_writer::FrameWriter;
use super::message::{CloseCode, Message};
use super::stream::{Client, ClientBuilder, Error, parse_ws_url};
use crate::buf::WriteBuf;
use crate::ws::HandshakeError;

// =============================================================================
// Async I/O helpers
// =============================================================================

async fn read_async<S: AsyncRead + Unpin>(stream: &mut S, buf: &mut [u8]) -> io::Result<usize> {
    stream.read(buf).await
}

async fn write_all_async<S: AsyncWrite + Unpin>(stream: &mut S, buf: &[u8]) -> io::Result<()> {
    stream.write_all(buf).await?;
    stream.flush().await
}

// =============================================================================
// Async impl on Client
// =============================================================================

impl<S: AsyncRead + AsyncWrite + Unpin> Client<S> {
    /// Connect with a pre-connected async stream using default configuration.
    ///
    /// Performs the HTTP upgrade handshake asynchronously.
    pub async fn connect_with(stream: S, url: &str) -> Result<Self, Error> {
        ClientBuilder::new().connect_with(stream, url).await
    }

    /// Accept an incoming WebSocket connection (server-side, async).
    pub async fn accept(stream: S) -> Result<Self, Error> {
        ClientBuilder::new().accept(stream).await
    }

    /// Receive the next message.
    ///
    /// Reads bytes from the stream asynchronously (through TLS if
    /// configured) and feeds them to the FrameReader.
    pub async fn recv(&mut self) -> Result<Option<Message<'_>>, Error> {
        loop {
            if self.reader.poll()? {
                return Ok(self.reader.next()?);
            }

            let spare = self.reader.spare();
            if spare.is_empty() {
                self.reader.compact();
                let spare = self.reader.spare();
                if spare.is_empty() {
                    return Ok(None); // buffer genuinely full
                }
            }

            let spare = self.reader.spare();
            let n = read_async(&mut self.stream, spare).await?;
            if n == 0 {
                return Ok(None); // EOF
            }
            self.reader.filled(n);
        }
    }

    /// Send a text message.
    pub async fn send_text(&mut self, text: &str) -> Result<(), Error> {
        self.writer
            .encode_text_into(text.as_bytes(), &mut self.write_buf);
        self.flush_write_buf().await
    }

    /// Send a binary message.
    pub async fn send_binary(&mut self, data: &[u8]) -> Result<(), Error> {
        self.writer.encode_binary_into(data, &mut self.write_buf);
        self.flush_write_buf().await
    }

    /// Send a ping.
    pub async fn send_ping(&mut self, data: &[u8]) -> Result<(), Error> {
        self.writer
            .encode_ping_into(data, &mut self.write_buf)
            .map_err(Error::Encode)?;
        self.flush_write_buf().await
    }

    /// Send a pong.
    pub async fn send_pong(&mut self, data: &[u8]) -> Result<(), Error> {
        self.writer
            .encode_pong_into(data, &mut self.write_buf)
            .map_err(Error::Encode)?;
        self.flush_write_buf().await
    }

    /// Initiate close handshake.
    pub async fn close(&mut self, code: CloseCode, reason: &str) -> Result<(), Error> {
        if code == CloseCode::NoStatus {
            let mut dst = [0u8; 14];
            let n = self.writer.encode_empty_close(&mut dst);
            self.write_raw(&dst[..n]).await
        } else {
            self.writer
                .encode_close_into(code.as_u16(), reason.as_bytes(), &mut self.write_buf)
                .map_err(Error::Encode)?;
            self.flush_write_buf().await
        }
    }

    // =========================================================================
    // Internal — write helpers
    // =========================================================================

    /// Flush the write buffer to the stream. Poisons on failure.
    async fn flush_write_buf(&mut self) -> Result<(), Error> {
        let data = self.write_buf.data();
        if let Err(e) = write_all_async(&mut self.stream, data).await {
            self.poisoned = true;
            return Err(Error::Io(e));
        }
        Ok(())
    }

    /// Write raw bytes to the stream. Poisons on failure.
    async fn write_raw(&mut self, data: &[u8]) -> Result<(), Error> {
        if let Err(e) = write_all_async(&mut self.stream, data).await {
            self.poisoned = true;
            return Err(Error::Io(e));
        }
        Ok(())
    }
}

// =============================================================================
// Internal async handshake
// =============================================================================

/// Perform the async HTTP upgrade handshake on a stream that is already
/// plaintext-ready (TLS handled at the stream level).
async fn tokio_connect_impl<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    url: &str,
    reader_builder: FrameReaderBuilder,
    write_cap: usize,
) -> Result<Client<S>, Error> {
    let parsed = parse_ws_url(url)?;
    let host_header = parsed.host_header();

    let key = crate::ws::handshake::generate_key();
    let key_str =
        std::str::from_utf8(&key).expect("base64-encoded key is always valid ASCII/UTF-8");

    let headers: [(&str, &str); 5] = [
        ("Host", &host_header),
        ("Upgrade", "websocket"),
        ("Connection", "Upgrade"),
        ("Sec-WebSocket-Key", key_str),
        ("Sec-WebSocket-Version", "13"),
    ];
    let req_size = crate::http::request_size("GET", parsed.path, &headers);
    let mut req_buf = vec![0u8; req_size];
    let n = crate::http::write_request("GET", parsed.path, &headers, &mut req_buf)
        .map_err(|_| HandshakeError::MalformedHttp)?;

    write_all_async(&mut stream, &req_buf[..n]).await?;

    let mut resp_reader = crate::http::ResponseReader::new(4096);
    let mut tmp = [0u8; 4096];
    loop {
        let bytes_read = read_async(&mut stream, &mut tmp).await?;
        if bytes_read == 0 {
            return Err(HandshakeError::MalformedHttp.into());
        }
        resp_reader
            .read(&tmp[..bytes_read])
            .map_err(|_| HandshakeError::MalformedHttp)?;
        match resp_reader.next() {
            Ok(Some(resp)) => {
                if resp.status != 101 {
                    return Err(HandshakeError::UnexpectedStatus(resp.status).into());
                }
                let upgrade = resp
                    .header("Upgrade")
                    .ok_or(HandshakeError::MissingUpgrade)?;
                if !upgrade.eq_ignore_ascii_case("websocket") {
                    return Err(HandshakeError::MissingUpgrade.into());
                }
                let conn_hdr = resp
                    .header("Connection")
                    .ok_or(HandshakeError::MissingConnection)?;
                if !conn_hdr
                    .as_bytes()
                    .windows(7)
                    .any(|w| w.eq_ignore_ascii_case(b"upgrade"))
                {
                    return Err(HandshakeError::MissingConnection.into());
                }
                let accept = resp
                    .header("Sec-WebSocket-Accept")
                    .ok_or(HandshakeError::InvalidAcceptKey)?;
                if !crate::ws::handshake::validate_accept(key_str, accept) {
                    return Err(HandshakeError::InvalidAcceptKey.into());
                }

                let mut reader = reader_builder.role(Role::Client).build();
                let remainder = resp_reader.remainder();
                if !remainder.is_empty() {
                    reader
                        .read(remainder)
                        .map_err(|_| HandshakeError::MalformedHttp)?;
                }

                return Ok(Client {
                    stream,
                    reader,
                    writer: FrameWriter::new(Role::Client),
                    write_buf: WriteBuf::new(write_cap, 14),
                    poisoned: false,
                });
            }
            Ok(None) => {} // need more bytes
            Err(_) => return Err(HandshakeError::MalformedHttp.into()),
        }
    }
}

async fn tokio_accept_impl<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    reader_builder: FrameReaderBuilder,
    write_cap: usize,
) -> Result<Client<S>, Error> {
    let mut req_reader = crate::http::RequestReader::new(4096);
    let mut tmp = [0u8; 4096];

    let ws_key;
    loop {
        let n = read_async(&mut stream, &mut tmp).await?;
        if n == 0 {
            return Err(HandshakeError::MalformedHttp.into());
        }
        req_reader
            .read(&tmp[..n])
            .map_err(|_| HandshakeError::MalformedHttp)?;
        match req_reader.next() {
            Ok(Some(req)) => {
                if req.method != "GET" {
                    return Err(HandshakeError::MalformedHttp.into());
                }
                let upgrade = req
                    .header("Upgrade")
                    .ok_or(HandshakeError::MissingUpgrade)?;
                if !upgrade.eq_ignore_ascii_case("websocket") {
                    return Err(HandshakeError::MissingUpgrade.into());
                }
                let conn_hdr = req
                    .header("Connection")
                    .ok_or(HandshakeError::MissingConnection)?;
                if !conn_hdr
                    .as_bytes()
                    .windows(7)
                    .any(|w| w.eq_ignore_ascii_case(b"upgrade"))
                {
                    return Err(HandshakeError::MissingConnection.into());
                }
                let version = req
                    .header("Sec-WebSocket-Version")
                    .ok_or(HandshakeError::UnsupportedVersion)?;
                if version != "13" {
                    return Err(HandshakeError::UnsupportedVersion.into());
                }
                let key = req
                    .header("Sec-WebSocket-Key")
                    .ok_or(HandshakeError::MissingKey)?;
                ws_key = key.to_owned();
                break;
            }
            Ok(None) => {}
            Err(_) => return Err(HandshakeError::MalformedHttp.into()),
        }
    }

    let accept = crate::ws::handshake::compute_accept_key(&ws_key);
    let accept_str = std::str::from_utf8(&accept).expect("base64 output is valid ASCII");

    let resp_headers = [
        ("Upgrade", "websocket"),
        ("Connection", "Upgrade"),
        ("Sec-WebSocket-Accept", accept_str),
    ];
    let resp_size = crate::http::response_size("Switching Protocols", &resp_headers);
    let mut resp_buf = vec![0u8; resp_size];
    let n = crate::http::write_response(101, "Switching Protocols", &resp_headers, &mut resp_buf)
        .map_err(|_| HandshakeError::MalformedHttp)?;
    write_all_async(&mut stream, &resp_buf[..n]).await?;

    let mut reader = reader_builder.role(Role::Server).build();
    let remainder = req_reader.remainder();
    if !remainder.is_empty() {
        reader
            .read(remainder)
            .map_err(|_| HandshakeError::MalformedHttp)?;
    }

    Ok(Client {
        stream,
        reader,
        writer: FrameWriter::new(Role::Server),
        write_buf: WriteBuf::new(write_cap, 14),
        poisoned: false,
    })
}

// =============================================================================
// Async builder methods
// =============================================================================

impl ClientBuilder {
    /// Connect with a pre-connected async stream (tokio).
    ///
    /// The stream must already handle TLS if connecting to `wss://`.
    /// For example, pass a `TlsStream<TcpStream>` or `MaybeTls<TcpStream>`.
    /// This method only performs the HTTP upgrade handshake.
    pub async fn connect_with<S: AsyncRead + AsyncWrite + Unpin>(
        self,
        stream: S,
        url: &str,
    ) -> Result<Client<S>, Error> {
        tokio_connect_impl(stream, url, self.reader_builder, self.write_buf_capacity).await
    }

    /// Accept an incoming async WebSocket connection (server-side, tokio).
    pub async fn accept<S: AsyncRead + AsyncWrite + Unpin>(
        self,
        stream: S,
    ) -> Result<Client<S>, Error> {
        tokio_accept_impl(stream, self.reader_builder, self.write_buf_capacity).await
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws::FrameReader;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// Mock async stream for unit tests. No real I/O — always Ready.
    struct MockStream {
        read_data: Vec<u8>,
        read_pos: usize,
    }

    impl MockStream {
        fn new(data: Vec<u8>) -> Self {
            Self {
                read_data: data,
                read_pos: 0,
            }
        }
    }

    impl AsyncRead for MockStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let remaining = &self.read_data[self.read_pos..];
            if remaining.is_empty() {
                return Poll::Ready(Ok(()));
            }
            let n = buf.remaining().min(remaining.len());
            buf.put_slice(&remaining[..n]);
            self.read_pos += n;
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for MockStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn make_frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::new();
        let byte0 = if fin { 0x80 } else { 0x00 } | opcode;
        frame.push(byte0);
        if payload.len() <= 125 {
            frame.push(payload.len() as u8);
        } else if payload.len() <= 65535 {
            frame.push(126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        } else {
            frame.push(127);
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
        frame.extend_from_slice(payload);
        frame
    }

    fn ws_from_bytes(data: Vec<u8>) -> Client<MockStream> {
        let mock = MockStream::new(data);
        let reader = FrameReader::builder().role(Role::Client).build();
        let writer = FrameWriter::new(Role::Client);
        Client::from_parts(mock, reader, writer)
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
        // SAFETY: no-op vtable, null data is never dereferenced.
        unsafe { std::task::Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
    }

    #[test]
    fn recv_text() {
        let frame = make_frame(true, 0x1, b"Hello");
        let mut ws = ws_from_bytes(frame);
        match block_on_mock(ws.recv()).unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn recv_binary() {
        let frame = make_frame(true, 0x2, &[0x42; 100]);
        let mut ws = ws_from_bytes(frame);
        match block_on_mock(ws.recv()).unwrap().unwrap() {
            Message::Binary(b) => assert_eq!(b.len(), 100),
            other => panic!("expected Binary, got {other:?}"),
        }
    }

    #[test]
    fn recv_ping() {
        let frame = make_frame(true, 0x9, b"ping");
        let mut ws = ws_from_bytes(frame);
        match block_on_mock(ws.recv()).unwrap().unwrap() {
            Message::Ping(p) => assert_eq!(p, b"ping"),
            other => panic!("expected Ping, got {other:?}"),
        }
    }

    #[test]
    fn recv_fragmented_text() {
        let mut data = make_frame(false, 0x1, b"Hel");
        data.extend_from_slice(&make_frame(true, 0x0, b"lo"));
        let mut ws = ws_from_bytes(data);
        match block_on_mock(ws.recv()).unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn recv_fragment_with_control() {
        let mut data = make_frame(false, 0x1, b"Hel");
        data.extend_from_slice(&make_frame(true, 0x9, b"ping"));
        data.extend_from_slice(&make_frame(true, 0x0, b"lo"));
        let mut ws = ws_from_bytes(data);
        match block_on_mock(ws.recv()).unwrap().unwrap() {
            Message::Ping(p) => assert_eq!(p, b"ping"),
            other => panic!("expected Ping, got {other:?}"),
        }
        match block_on_mock(ws.recv()).unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn recv_close() {
        let mut payload = vec![];
        payload.extend_from_slice(&1000u16.to_be_bytes());
        payload.extend_from_slice(b"bye");
        let frame = make_frame(true, 0x8, &payload);
        let mut ws = ws_from_bytes(frame);
        match block_on_mock(ws.recv()).unwrap().unwrap() {
            Message::Close(cf) => {
                assert_eq!(cf.code, CloseCode::Normal);
                assert_eq!(cf.reason, "bye");
            }
            other => panic!("expected Close, got {other:?}"),
        }
    }

    #[test]
    fn eof_returns_none() {
        let mut ws = ws_from_bytes(Vec::new());
        assert!(block_on_mock(ws.recv()).unwrap().is_none());
    }

    #[test]
    fn fifo_three_messages() {
        let mut data = make_frame(true, 0x1, b"first");
        data.extend_from_slice(&make_frame(true, 0x1, b"second"));
        data.extend_from_slice(&make_frame(true, 0x1, b"third"));
        let mut ws = ws_from_bytes(data);

        match block_on_mock(ws.recv()).unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "first"),
            other => panic!("expected first, got {other:?}"),
        }
        match block_on_mock(ws.recv()).unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "second"),
            other => panic!("expected second, got {other:?}"),
        }
        match block_on_mock(ws.recv()).unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "third"),
            other => panic!("expected third, got {other:?}"),
        }
    }

    #[test]
    fn send_on_broken_stream() {
        struct BrokenWrite;

        impl AsyncRead for BrokenWrite {
            fn poll_read(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                _buf: &mut tokio::io::ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        impl AsyncWrite for BrokenWrite {
            fn poll_write(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                _buf: &[u8],
            ) -> Poll<io::Result<usize>> {
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "connection lost",
                )))
            }

            fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }

            fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        let reader = FrameReader::builder().role(Role::Client).build();
        let writer = FrameWriter::new(Role::Client);
        let mut ws = Client::from_parts(BrokenWrite, reader, writer);

        let result = block_on_mock(ws.send_text("hello"));
        assert!(result.is_err(), "send on broken stream should fail");
    }
}
