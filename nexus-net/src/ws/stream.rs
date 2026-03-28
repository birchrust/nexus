//! WebSocket stream — I/O wrapper with HTTP upgrade handshake.

use std::io::{Read, Write};

use super::error::ProtocolError;
use super::frame::Role;
use super::frame_reader::FrameReader;
use super::frame_writer::FrameWriter;
use super::handshake::{self, HandshakeError};
use super::message::{CloseCode, Message};

/// Unified error type for WsStream operations.
#[derive(Debug)]
pub enum WsError {
    /// I/O error from the underlying stream.
    Io(std::io::Error),
    /// WebSocket protocol error.
    Protocol(ProtocolError),
    /// HTTP handshake failed.
    Handshake(HandshakeError),
}

impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Protocol(e) => write!(f, "protocol error: {e}"),
            Self::Handshake(e) => write!(f, "handshake error: {e}"),
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

/// WebSocket stream — owns a socket, reader, and writer.
///
/// Generic over `S: Read + Write` so it works with plain TCP, rustls,
/// or any other byte stream.
///
/// # Usage
///
/// ```no_run
/// use std::net::TcpStream;
/// use nexus_net::ws::{WsStream, Message};
///
/// let tcp = TcpStream::connect("echo.websocket.org:80").unwrap();
/// let mut ws = WsStream::connect(tcp, "echo.websocket.org", "/").unwrap();
///
/// ws.send_text("Hello!").unwrap();
///
/// while let Some(msg) = ws.next().unwrap() {
///     let owned = msg.into_owned();
///     match &owned {
///         nexus_net::ws::OwnedMessage::Text(s) => println!("received: {s}"),
///         nexus_net::ws::OwnedMessage::Ping(p) => ws.send_pong(p).unwrap(),
///         nexus_net::ws::OwnedMessage::Close(_) => break,
///         _ => {}
///     }
/// }
/// ```
pub struct WsStream<S> {
    stream: S,
    reader: FrameReader,
    writer: FrameWriter,
}

impl<S: Read + Write> WsStream<S> {
    /// Connect as WebSocket client.
    ///
    /// Performs the HTTP/1.1 upgrade handshake, then returns a ready stream.
    pub fn connect(mut stream: S, host: &str, path: &str) -> Result<Self, WsError> {
        // Generate key
        let key = handshake::generate_key();
        let key_str = std::str::from_utf8(&key).unwrap();

        // Write upgrade request
        let mut req_buf = [0u8; 512];
        let n = crate::http::write_request("GET", path, &[
            ("Host", host),
            ("Upgrade", "websocket"),
            ("Connection", "Upgrade"),
            ("Sec-WebSocket-Key", key_str),
            ("Sec-WebSocket-Version", "13"),
        ], &mut req_buf);
        stream.write_all(&req_buf[..n])?;

        // Read response
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
                    // Validate 101
                    if resp.status != 101 {
                        return Err(HandshakeError::UnexpectedStatus(resp.status).into());
                    }
                    // Validate headers
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

                    // Feed any remainder to the frame reader
                    let mut reader = FrameReader::builder().role(Role::Client).build();
                    let remainder = resp_reader.remainder();
                    if !remainder.is_empty() {
                        reader.read(remainder).map_err(|_| HandshakeError::MalformedHttp)?;
                    }

                    return Ok(Self {
                        stream,
                        reader,
                        writer: FrameWriter::new(Role::Client),
                    });
                }
                Ok(None) => {} // need more bytes
                Err(_) => return Err(HandshakeError::MalformedHttp.into()),
            }
        }
    }

    /// Accept as WebSocket server.
    ///
    /// Reads the upgrade request, validates it, sends the 101 response.
    pub fn accept(mut stream: S) -> Result<Self, WsError> {
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
                    // Validate upgrade request
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
                Ok(None) => {}
                Err(_) => return Err(HandshakeError::MalformedHttp.into()),
            }
        }

        // Compute accept key
        let accept = handshake::compute_accept_key(&ws_key);
        let accept_str = std::str::from_utf8(&accept).unwrap();

        // Write 101 response
        let mut resp_buf = [0u8; 256];
        let n = crate::http::write_response(101, "Switching Protocols", &[
            ("Upgrade", "websocket"),
            ("Connection", "Upgrade"),
            ("Sec-WebSocket-Accept", accept_str),
        ], &mut resp_buf);
        stream.write_all(&resp_buf[..n])?;

        // Feed any remainder to frame reader
        let mut reader = FrameReader::builder().role(Role::Server).build();
        let remainder = req_reader.remainder();
        if !remainder.is_empty() {
            reader.read(remainder).map_err(|_| HandshakeError::MalformedHttp)?;
        }

        Ok(Self {
            stream,
            reader,
            writer: FrameWriter::new(Role::Server),
        })
    }

    /// Read the next message. Reads from the socket as needed.
    ///
    /// Blocks until a complete message is available or the connection closes.
    /// Call in a loop to process messages.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Message<'_>>, WsError> {
        // Ensure there's data to parse — read from socket if needed.
        // We do this BEFORE calling next() to avoid the borrow overlap.
        while !self.has_buffered_frame() {
            let spare = self.reader.spare();
            if spare.is_empty() {
                return Ok(None);
            }
            let n = self.stream.read(spare)?;
            if n == 0 {
                return Ok(None);
            }
            self.reader.filled(n);
        }

        // Now parse — the Message borrows from self.reader, no more
        // socket reads needed.
        Ok(self.reader.next()?)
    }

    /// Quick check if there's enough data for at least a partial parse.
    /// This doesn't consume anything — just checks if data is available.
    fn has_buffered_frame(&self) -> bool {
        self.reader.buffered() >= 2 // minimum WS frame is 2 bytes
    }

    /// Send a text message.
    pub fn send_text(&mut self, text: &str) -> Result<(), WsError> {
        let mut dst = vec![0u8; self.writer.max_encoded_len(text.len())];
        let n = self.writer.encode_text(text.as_bytes(), &mut dst);
        self.stream.write_all(&dst[..n])?;
        Ok(())
    }

    /// Send a binary message.
    pub fn send_binary(&mut self, data: &[u8]) -> Result<(), WsError> {
        let mut dst = vec![0u8; self.writer.max_encoded_len(data.len())];
        let n = self.writer.encode_binary(data, &mut dst);
        self.stream.write_all(&dst[..n])?;
        Ok(())
    }

    /// Send a ping.
    pub fn send_ping(&mut self, data: &[u8]) -> Result<(), WsError> {
        let mut dst = vec![0u8; self.writer.max_encoded_len(data.len())];
        let n = self.writer.encode_ping(data, &mut dst);
        self.stream.write_all(&dst[..n])?;
        Ok(())
    }

    /// Send a pong.
    pub fn send_pong(&mut self, data: &[u8]) -> Result<(), WsError> {
        let mut dst = vec![0u8; self.writer.max_encoded_len(data.len())];
        let n = self.writer.encode_pong(data, &mut dst);
        self.stream.write_all(&dst[..n])?;
        Ok(())
    }

    /// Initiate close handshake.
    pub fn close(&mut self, code: CloseCode, reason: &str) -> Result<(), WsError> {
        let mut dst = vec![0u8; self.writer.max_encoded_len(2 + reason.len())];
        let n = self.writer.encode_close(code.as_u16(), reason.as_bytes(), &mut dst);
        self.stream.write_all(&dst[..n])?;
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
}

/// Case-insensitive substring check without allocation.
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}
