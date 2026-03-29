//! WebSocket stream over TLS — I/O wrapper with TLS + HTTP upgrade handshake.

use std::io::{Read, Write};

use crate::buf::WriteBuf;
use crate::tls::{TlsCodec, TlsConfig, TlsError};
use super::frame::Role;
use super::frame_reader::{FrameReader, FrameReaderBuilder};
use super::frame_writer::FrameWriter;
use super::handshake::{self, HandshakeError};
use super::message::{CloseCode, Message};
use super::stream::{WsError, WsStreamBuilder};

impl From<TlsError> for WsError {
    fn from(e: TlsError) -> Self {
        match e {
            TlsError::Io(io) => Self::Io(io),
            other => Self::Tls(other),
        }
    }
}

/// WebSocket stream over TLS.
///
/// Same API as [`WsStream`](super::WsStream) but with TLS encryption.
/// The TLS handshake and HTTP upgrade are performed during construction.
///
/// # Usage
///
/// ```no_run
/// use std::net::TcpStream;
/// use nexus_net::tls::TlsConfig;
/// use nexus_net::ws::{WsTlsStream, OwnedMessage};
///
/// let tcp = TcpStream::connect("exchange.com:443").unwrap();
/// let tls = TlsConfig::new().unwrap();
/// let mut ws = WsTlsStream::connect(tcp, &tls, "wss://exchange.com/ws/v1").unwrap();
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
pub struct WsTlsStream<S> {
    stream: S,
    tls: TlsCodec,
    reader: FrameReader,
    writer: FrameWriter,
    write_buf: WriteBuf,
}

impl WsStreamBuilder {
    /// Connect as WebSocket client over TLS with configured buffers.
    pub fn connect_tls<S: Read + Write>(
        self,
        stream: S,
        tls_config: &TlsConfig,
        url: &str,
    ) -> Result<WsTlsStream<S>, WsError> {
        let (host, path) = parse_ws_url(url);
        WsTlsStream::connect_impl(
            stream, tls_config, host, path,
            self.reader_builder, self.write_buf_capacity, self.write_buf_headroom,
        )
    }
}

impl<S: Read + Write> WsTlsStream<S> {
    /// Connect as WebSocket client over TLS with default configuration.
    ///
    /// Performs TLS handshake, then HTTP upgrade.
    pub fn connect(
        stream: S,
        tls_config: &TlsConfig,
        url: &str,
    ) -> Result<Self, WsError> {
        let (host, path) = parse_ws_url(url);
        Self::connect_impl(stream, tls_config, host, path, FrameReader::builder(), 65_536, 14)
    }

    /// Read the next message. Reads from the socket as needed.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Message<'_>>, WsError> {
        loop {
            if self.reader.poll()? {
                return Ok(self.reader.next()?);
            }

            // Read TLS records from socket, decrypt into FrameReader
            let n = self.tls.read_tls_from(&mut self.stream)?;
            if n == 0 {
                return Ok(None); // EOF
            }
            self.tls.process_into(&mut self.reader)?;
        }
    }

    /// Send a text message.
    pub fn send_text(&mut self, text: &str) -> Result<(), WsError> {
        self.writer.encode_text_into(text.as_bytes(), &mut self.write_buf);
        self.tls.encrypt(self.write_buf.data())?;
        self.tls.write_tls_to(&mut self.stream)?;
        Ok(())
    }

    /// Send a binary message.
    pub fn send_binary(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.writer.encode_binary_into(data, &mut self.write_buf);
        self.tls.encrypt(self.write_buf.data())?;
        self.tls.write_tls_to(&mut self.stream)?;
        Ok(())
    }

    /// Send a ping.
    pub fn send_ping(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.writer.encode_ping_into(data, &mut self.write_buf);
        self.tls.encrypt(self.write_buf.data())?;
        self.tls.write_tls_to(&mut self.stream)?;
        Ok(())
    }

    /// Send a pong.
    pub fn send_pong(&mut self, data: &[u8]) -> Result<(), WsError> {
        self.writer.encode_pong_into(data, &mut self.write_buf);
        self.tls.encrypt(self.write_buf.data())?;
        self.tls.write_tls_to(&mut self.stream)?;
        Ok(())
    }

    /// Initiate close handshake.
    pub fn close(&mut self, code: CloseCode, reason: &str) -> Result<(), WsError> {
        self.writer.encode_close_into(code.as_u16(), reason.as_bytes(), &mut self.write_buf);
        self.tls.encrypt(self.write_buf.data())?;
        self.tls.write_tls_to(&mut self.stream)?;
        Ok(())
    }

    /// Access the underlying stream.
    pub fn stream(&self) -> &S { &self.stream }
    /// Mutable access to the underlying stream.
    pub fn stream_mut(&mut self) -> &mut S { &mut self.stream }
    /// Access the TLS codec.
    pub fn tls(&self) -> &TlsCodec { &self.tls }
    /// Access the FrameReader.
    pub fn reader(&self) -> &FrameReader { &self.reader }
    /// Access the FrameWriter.
    pub fn writer(&self) -> &FrameWriter { &self.writer }

    // =========================================================================
    // Internal
    // =========================================================================

    fn connect_impl(
        mut stream: S,
        tls_config: &TlsConfig,
        host: &str,
        path: &str,
        reader_builder: FrameReaderBuilder,
        write_cap: usize,
        write_headroom: usize,
    ) -> Result<Self, WsError> {
        // Phase 1: TLS handshake
        let mut tls = TlsCodec::new(tls_config, host)?;

        while tls.is_handshaking() {
            if tls.wants_write() {
                tls.write_tls_to(&mut stream)?;
            }
            if tls.wants_read() {
                tls.read_tls_from(&mut stream)?;
                tls.process_new_packets()?;
            }
        }
        // Flush any remaining handshake data
        if tls.wants_write() {
            tls.write_tls_to(&mut stream)?;
        }

        // Phase 2: HTTP upgrade (over the TLS tunnel)
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

        // Write HTTP request through TLS
        tls.encrypt(&req_buf[..n])?;
        tls.write_tls_to(&mut stream)?;

        // Read HTTP response through TLS
        let mut resp_reader = crate::http::ResponseReader::new(4096);
        let mut tmp = [0u8; 4096];
        loop {
            tls.read_tls_from(&mut stream)?;
            tls.process_new_packets()?;

            let n = match tls.read_plaintext(&mut tmp) {
                Ok(0) => return Err(HandshakeError::MalformedHttp.into()),
                Ok(n) => n,
                Err(e) => return Err(WsError::Tls(e)),
            };

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
                        tls,
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
}

fn parse_ws_url(url: &str) -> (&str, &str) {
    let stripped = url.strip_prefix("wss://")
        .or_else(|| url.strip_prefix("ws://"))
        .unwrap_or(url);
    stripped.find('/').map_or((stripped, "/"), |i| (&stripped[..i], &stripped[i..]))
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}
