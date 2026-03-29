//! WebSocket stream — I/O wrapper with HTTP upgrade handshake.

use std::io::{Read, Write};

use crate::buf::WriteBuf;
use super::error::ProtocolError;
use super::frame::Role;
use super::frame_reader::{FrameReader, FrameReaderBuilder};
use super::frame_writer::FrameWriter;
use super::handshake::{self, HandshakeError};
use super::message::{CloseCode, Message};

/// Parse a WebSocket URL into (host, path).
///
/// Accepts: `ws://host/path`, `wss://host/path`, or `host/path`.
/// If no path is present, defaults to `"/"`.
fn parse_ws_url(url: &str) -> (&str, &str) {
    let stripped = url.strip_prefix("wss://")
        .or_else(|| url.strip_prefix("ws://"))
        .unwrap_or(url);
    stripped.find('/').map_or((stripped, "/"), |i| (&stripped[..i], &stripped[i..]))
}

/// Unified error type for WsStream operations.
#[derive(Debug)]
pub enum WsError {
    /// I/O error from the underlying stream.
    Io(std::io::Error),
    /// WebSocket protocol error.
    Protocol(ProtocolError),
    /// HTTP handshake failed.
    Handshake(HandshakeError),
    /// TLS error.
    #[cfg(feature = "tls")]
    Tls(crate::tls::TlsError),
}

impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Protocol(e) => write!(f, "protocol error: {e}"),
            Self::Handshake(e) => write!(f, "handshake error: {e}"),
            #[cfg(feature = "tls")]
            Self::Tls(e) => write!(f, "TLS error: {e}"),
        }
    }
}

impl std::error::Error for WsError {}

impl From<std::io::Error> for WsError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}
impl From<ProtocolError> for WsError {
    fn from(e: ProtocolError) -> Self { Self::Protocol(e) }
}
impl From<HandshakeError> for WsError {
    fn from(e: HandshakeError) -> Self { Self::Handshake(e) }
}

/// Builder for [`WsStream`].
pub struct WsStreamBuilder {
    pub(crate) reader_builder: FrameReaderBuilder,
    pub(crate) write_buf_capacity: usize,
    pub(crate) write_buf_headroom: usize,
}

impl WsStreamBuilder {
    /// ReadBuf capacity. Default: 1MB.
    #[must_use]
    pub fn buffer_capacity(mut self, n: usize) -> Self {
        self.reader_builder = self.reader_builder.buffer_capacity(n);
        self
    }

    /// Maximum single frame payload. Default: 16MB.
    #[must_use]
    pub fn max_frame_size(mut self, n: u64) -> Self {
        self.reader_builder = self.reader_builder.max_frame_size(n);
        self
    }

    /// Maximum assembled message size. Default: 16MB.
    #[must_use]
    pub fn max_message_size(mut self, n: usize) -> Self {
        self.reader_builder = self.reader_builder.max_message_size(n);
        self
    }

    /// Write buffer capacity. Default: 4KB.
    #[must_use]
    pub fn write_buffer_capacity(mut self, n: usize) -> Self {
        self.write_buf_capacity = n;
        self
    }

    /// Connect as WebSocket client with configured buffers.
    pub fn connect<S: Read + Write>(
        self,
        stream: S,
        url: &str,
    ) -> Result<WsStream<S>, WsError> {
        let (host, path) = parse_ws_url(url);
        WsStream::connect_impl(stream, host, path, self.reader_builder, self.write_buf_capacity, self.write_buf_headroom)
    }

    /// Accept as WebSocket server with configured buffers.
    pub fn accept<S: Read + Write>(self, stream: S) -> Result<WsStream<S>, WsError> {
        WsStream::accept_impl(stream, self.reader_builder, self.write_buf_capacity, self.write_buf_headroom)
    }
}

/// WebSocket stream — owns a socket, reader, writer, and buffers.
///
/// Generic over `S: Read + Write` so it works with plain TCP, rustls,
/// or any other byte stream.
///
/// # Usage
///
/// ```no_run
/// use std::net::TcpStream;
/// use nexus_net::ws::{WsStream, OwnedMessage};
///
/// let tcp = TcpStream::connect("echo.websocket.org:80").unwrap();
/// let mut ws = WsStream::connect(tcp, "ws://echo.websocket.org/").unwrap();
///
/// ws.send_text("Hello!").unwrap();
///
/// while let Some(msg) = ws.next().unwrap() {
///     let owned = msg.into_owned();
///     match &owned {
///         OwnedMessage::Text(s) => println!("received: {s}"),
///         OwnedMessage::Ping(p) => ws.send_pong(p).unwrap(),
///         OwnedMessage::Close(_) => break,
///         _ => {}
///     }
/// }
/// ```
pub struct WsStream<S> {
    stream: S,
    reader: FrameReader,
    writer: FrameWriter,
    write_buf: WriteBuf,
}

impl WsStreamBuilder {
    /// Create a new builder with defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            reader_builder: FrameReader::builder(),
            write_buf_capacity: 65_536,
            write_buf_headroom: 14,
        }
    }
}

impl Default for WsStreamBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Read + Write> WsStream<S> {
    /// Create a builder for configuring buffer sizes.
    #[must_use]
    pub fn builder() -> WsStreamBuilder {
        WsStreamBuilder::new()
    }

    /// Create from pre-existing parts. For testing or custom handshakes.
    pub fn from_parts(stream: S, reader: FrameReader, writer: FrameWriter) -> Self {
        Self {
            stream,
            reader,
            writer,
            write_buf: WriteBuf::new(65_536, 14),
        }
    }

    /// Connect as WebSocket client with default configuration.
    ///
    /// `url` is a WebSocket URL: `ws://host/path` or `wss://host/path`.
    /// The scheme prefix is optional — `host/path` works too.
    ///
    /// ```no_run
    /// # use std::net::TcpStream;
    /// # use nexus_net::ws::WsStream;
    /// let tcp = TcpStream::connect("exchange.com:443").unwrap();
    /// let ws = WsStream::connect(tcp, "wss://exchange.com/ws/v1").unwrap();
    /// ```
    pub fn connect(stream: S, url: &str) -> Result<Self, WsError> {
        let (host, path) = parse_ws_url(url);
        Self::connect_impl(stream, host, path, FrameReader::builder(), 65_536, 14)
    }

    /// Connect with a pre-configured FrameReader.
    pub fn connect_with_reader(
        stream: S,
        url: &str,
        reader_builder: FrameReaderBuilder,
    ) -> Result<Self, WsError> {
        let (host, path) = parse_ws_url(url);
        Self::connect_impl(stream, host, path, reader_builder, 65_536, 14)
    }

    /// Accept as WebSocket server with default configuration.
    pub fn accept(stream: S) -> Result<Self, WsError> {
        Self::accept_impl(stream, FrameReader::builder(), 65_536, 14)
    }

    /// Accept with a pre-configured FrameReader.
    pub fn accept_with_reader(
        stream: S,
        reader_builder: FrameReaderBuilder,
    ) -> Result<Self, WsError> {
        Self::accept_impl(stream, reader_builder, 65_536, 14)
    }

    /// Read the next message. Reads from the socket as needed.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Message<'_>>, WsError> {
        // Use poll() to advance the parser without borrowing the return.
        // Once poll() returns true, call next() once to get Message<'_>.
        loop {
            if self.reader.poll()? {
                // Message ready — next() will return it immediately
                return Ok(self.reader.next()?);
            }

            // Need more bytes from socket
            let spare = self.reader.spare();
            if spare.is_empty() {
                return Ok(None); // buffer full
            }
            let n = self.stream.read(spare)?;
            if n == 0 {
                return Ok(None); // EOF
            }
            self.reader.filled(n);
        }
    }

    /// Send a text message. Zero allocation — uses internal WriteBuf.
    pub fn send_text(&mut self, text: &str) -> Result<(), WsError> {
        self.writer.encode_text_into(text.as_bytes(), &mut self.write_buf);
        self.stream.write_all(self.write_buf.data())?;
        Ok(())
    }

    /// Send a binary message.
    pub fn send_binary(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.writer.encode_binary_into(data, &mut self.write_buf);
        self.stream.write_all(self.write_buf.data())?;
        Ok(())
    }

    /// Send a ping.
    pub fn send_ping(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.writer.encode_ping_into(data, &mut self.write_buf);
        self.stream.write_all(self.write_buf.data())?;
        Ok(())
    }

    /// Send a pong.
    pub fn send_pong(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.writer.encode_pong_into(data, &mut self.write_buf);
        self.stream.write_all(self.write_buf.data())?;
        Ok(())
    }

    /// Initiate close handshake.
    pub fn close(&mut self, code: CloseCode, reason: &str) -> Result<(), WsError> {
        self.writer.encode_close_into(code.as_u16(), reason.as_bytes(), &mut self.write_buf);
        self.stream.write_all(self.write_buf.data())?;
        Ok(())
    }

    /// Access the underlying stream.
    pub fn stream(&self) -> &S { &self.stream }
    /// Mutable access to the underlying stream.
    pub fn stream_mut(&mut self) -> &mut S { &mut self.stream }
    /// Access the FrameReader.
    pub fn reader(&self) -> &FrameReader { &self.reader }
    /// Access the FrameWriter.
    pub fn writer(&self) -> &FrameWriter { &self.writer }

    // =========================================================================
    // Internal
    // =========================================================================

    fn connect_impl(
        mut stream: S,
        host: &str,
        path: &str,
        reader_builder: FrameReaderBuilder,
        write_cap: usize,
        write_headroom: usize,
    ) -> Result<Self, WsError> {
        let key = handshake::generate_key();
        let key_str = std::str::from_utf8(&key).unwrap();

        let mut req_buf = [0u8; 512];
        let n = crate::http::write_request("GET", path, &[
            ("Host", host),
            ("Upgrade", "websocket"),
            ("Connection", "Upgrade"),
            ("Sec-WebSocket-Key", key_str),
            ("Sec-WebSocket-Version", "13"),
        ], &mut req_buf);
        stream.write_all(&req_buf[..n])?;

        let mut resp_reader = crate::http::ResponseReader::new(4096);
        let mut tmp = [0u8; 4096];
        loop {
            let n = stream.read(&mut tmp)?;
            if n == 0 {
                return Err(HandshakeError::MalformedHttp.into());
            }
            resp_reader.read(&tmp[..n]).map_err(|_| HandshakeError::MalformedHttp)?;
            match resp_reader.next() {
                Ok(Some(resp)) => {
                    if resp.status != 101 {
                        return Err(HandshakeError::UnexpectedStatus(resp.status).into());
                    }
                    let upgrade = resp.header("Upgrade")
                        .ok_or(HandshakeError::MissingUpgrade)?;
                    if !upgrade.eq_ignore_ascii_case("websocket") {
                        return Err(HandshakeError::MissingUpgrade.into());
                    }
                    let conn = resp.header("Connection")
                        .ok_or(HandshakeError::MissingConnection)?;
                    if !contains_ignore_case(conn, "upgrade") {
                        return Err(HandshakeError::MissingConnection.into());
                    }
                    let accept = resp.header("Sec-WebSocket-Accept")
                        .ok_or(HandshakeError::InvalidAcceptKey)?;
                    if !handshake::validate_accept(key_str, accept) {
                        return Err(HandshakeError::InvalidAcceptKey.into());
                    }

                    let mut reader = reader_builder.role(Role::Client).build();
                    let remainder = resp_reader.remainder();
                    if !remainder.is_empty() {
                        reader.read(remainder).map_err(|_| HandshakeError::MalformedHttp)?;
                    }

                    return Ok(Self {
                        stream,
                        reader,
                        writer: FrameWriter::new(Role::Client),
                        write_buf: WriteBuf::new(write_cap, write_headroom),
                    });
                }
                Ok(None) => {} // need more bytes
                Err(_) => return Err(HandshakeError::MalformedHttp.into()),
            }
        }
    }

    fn accept_impl(
        mut stream: S,
        reader_builder: FrameReaderBuilder,
        write_cap: usize,
        write_headroom: usize,
    ) -> Result<Self, WsError> {
        let mut req_reader = crate::http::RequestReader::new(4096);
        let mut tmp = [0u8; 4096];

        let ws_key;
        loop {
            let n = stream.read(&mut tmp)?;
            if n == 0 {
                return Err(HandshakeError::MalformedHttp.into());
            }
            req_reader.read(&tmp[..n]).map_err(|_| HandshakeError::MalformedHttp)?;
            match req_reader.next() {
                Ok(Some(req)) => {
                    if req.method != "GET" {
                        return Err(HandshakeError::MalformedHttp.into());
                    }
                    let upgrade = req.header("Upgrade")
                        .ok_or(HandshakeError::MissingUpgrade)?;
                    if !upgrade.eq_ignore_ascii_case("websocket") {
                        return Err(HandshakeError::MissingUpgrade.into());
                    }
                    let conn = req.header("Connection")
                        .ok_or(HandshakeError::MissingConnection)?;
                    if !contains_ignore_case(conn, "upgrade") {
                        return Err(HandshakeError::MissingConnection.into());
                    }
                    let version = req.header("Sec-WebSocket-Version")
                        .ok_or(HandshakeError::UnsupportedVersion)?;
                    if version != "13" {
                        return Err(HandshakeError::UnsupportedVersion.into());
                    }
                    let key = req.header("Sec-WebSocket-Key")
                        .ok_or(HandshakeError::MissingKey)?;
                    ws_key = key.to_owned();
                    break;
                }
                Ok(None) => {} // need more bytes
                Err(_) => return Err(HandshakeError::MalformedHttp.into()),
            }
        }

        let accept = handshake::compute_accept_key(&ws_key);
        let accept_str = std::str::from_utf8(&accept).unwrap();

        let mut resp_buf = [0u8; 256];
        let n = crate::http::write_response(101, "Switching Protocols", &[
            ("Upgrade", "websocket"),
            ("Connection", "Upgrade"),
            ("Sec-WebSocket-Accept", accept_str),
        ], &mut resp_buf);
        stream.write_all(&resp_buf[..n])?;

        let mut reader = reader_builder.role(Role::Server).build();
        let remainder = req_reader.remainder();
        if !remainder.is_empty() {
            reader.read(remainder).map_err(|_| HandshakeError::MalformedHttp)?;
        }

        Ok(Self {
            stream,
            reader,
            writer: FrameWriter::new(Role::Server),
            write_buf: WriteBuf::new(write_cap, write_headroom),
        })
    }
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::mask::apply_mask;

    // Helper: build unmasked WS frame
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

    /// Mock stream: delivers one byte at a time (for byte-at-a-time tests).
    struct ByteAtATimeStream {
        data: Vec<u8>,
        pos: usize,
    }

    impl ByteAtATimeStream {
        fn new(data: Vec<u8>) -> Self { Self { data, pos: 0 } }
    }

    impl Read for ByteAtATimeStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            buf[0] = self.data[self.pos];
            self.pos += 1;
            Ok(1)
        }
    }

    impl Write for ByteAtATimeStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> { Ok(buf.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }

    fn ws_from_bytes(data: Vec<u8>) -> WsStream<ByteAtATimeStream> {
        let mock = ByteAtATimeStream::new(data);
        let reader = FrameReader::builder().role(Role::Client).build();
        let writer = FrameWriter::new(Role::Client);
        WsStream::from_parts(mock, reader, writer)
    }

    // === Byte-at-a-time delivery (Autobahn 2.6, 5.5, 5.8) ===

    #[test]
    fn byte_at_a_time_ping() {
        let frame = make_frame(true, 0x9, &[0x42; 125]);
        let mut ws = ws_from_bytes(frame);
        match ws.next().unwrap().unwrap() {
            Message::Ping(p) => assert_eq!(p.len(), 125),
            other => panic!("expected Ping, got {other:?}"),
        }
    }

    #[test]
    fn byte_at_a_time_text() {
        let frame = make_frame(true, 0x1, b"Hello");
        let mut ws = ws_from_bytes(frame);
        match ws.next().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn byte_at_a_time_fragmented_text() {
        let mut data = make_frame(false, 0x1, b"Hel");
        data.extend_from_slice(&make_frame(true, 0x0, b"lo"));
        let mut ws = ws_from_bytes(data);
        match ws.next().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn byte_at_a_time_fragment_with_ping() {
        let mut data = make_frame(false, 0x1, b"Hel");
        data.extend_from_slice(&make_frame(true, 0x9, b"ping"));
        data.extend_from_slice(&make_frame(true, 0x0, b"lo"));
        let mut ws = ws_from_bytes(data);
        // Ping first
        match ws.next().unwrap().unwrap() {
            Message::Ping(p) => assert_eq!(p, b"ping"),
            other => panic!("expected Ping, got {other:?}"),
        }
        // Then assembled text
        match ws.next().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn byte_at_a_time_close() {
        let mut payload = vec![];
        payload.extend_from_slice(&1000u16.to_be_bytes());
        payload.extend_from_slice(b"bye");
        let frame = make_frame(true, 0x8, &payload);
        let mut ws = ws_from_bytes(frame);
        match ws.next().unwrap().unwrap() {
            Message::Close(cf) => {
                assert_eq!(cf.code, CloseCode::Normal);
                assert_eq!(cf.reason, "bye");
            }
            other => panic!("expected Close, got {other:?}"),
        }
    }

    #[test]
    fn eof_returns_none() {
        let ws_data = Vec::new(); // empty — immediate EOF
        let mut ws = ws_from_bytes(ws_data);
        assert!(ws.next().unwrap().is_none());
    }
}
