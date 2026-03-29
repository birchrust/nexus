//! Non-blocking WebSocket connection handshake.

use std::io::{self, Read, Write};

use crate::buf::WriteBuf;
use super::frame::Role;
use super::frame_reader::{FrameReader, FrameReaderBuilder};
use super::frame_writer::FrameWriter;
use super::handshake::{self, HandshakeError};
use super::stream::{WsError, WsStream, WsStreamBuilder, parse_ws_url};

#[cfg(feature = "tls")]
use crate::tls::TlsCodec;

/// A WebSocket connection in the handshake phase.
///
/// Drive the handshake by calling [`poll()`](Self::poll) when the socket
/// is ready. Returns [`WsStream<S>`] when complete.
///
/// Check [`wants_read()`](Self::wants_read) / [`wants_write()`](Self::wants_write)
/// to determine which readiness event to wait for in your event loop.
///
/// # Usage
///
/// ```ignore
/// use nexus_net::ws::{WsConnecting, WsStreamBuilder};
///
/// let tcp = TcpStream::connect("exchange.com:443")?;
/// tcp.set_nonblocking(true)?;
/// let mut connecting = WsStreamBuilder::new()
///     .begin_connect(tcp, "wss://exchange.com/ws")?;
///
/// // In your event loop:
/// loop {
///     // ... poll for socket readiness ...
///     if let Some(ws) = connecting.poll()? {
///         // Handshake complete — ws.recv() is now available
///         break;
///     }
/// }
/// ```
pub struct WsConnecting<S> {
    // ManuallyDrop: ownership transferred to WsStream in finish().
    // Drop impl handles cleanup if finish() is never called (error path).
    stream: std::mem::ManuallyDrop<S>,
    state: ConnectState,
    #[cfg(feature = "tls")]
    tls: Option<TlsCodec>,
    reader_builder: FrameReaderBuilder,
    write_buf_capacity: usize,
    write_buf_headroom: usize,
    // Handshake data
    ws_key: [u8; 24],
    req_buf: Vec<u8>,
    req_offset: usize,
    resp_reader: crate::http::ResponseReader,
    host: String,
    path: String,
    finished: bool, // true after finish() called — suppress Drop
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectState {
    /// TLS handshake: need to write.
    #[cfg(feature = "tls")]
    TlsWrite,
    /// TLS handshake: need to read.
    #[cfg(feature = "tls")]
    TlsRead,
    /// Sending HTTP upgrade request.
    HttpSend,
    /// Reading HTTP upgrade response.
    HttpRecv,
    /// Handshake complete, ready to transition.
    Done,
}

impl WsStreamBuilder {
    /// Start a non-blocking connection handshake.
    ///
    /// Returns a [`WsConnecting`] that must be driven to completion
    /// via [`poll()`](WsConnecting::poll) before messages can be sent/received.
    ///
    /// The caller is responsible for setting the socket to non-blocking
    /// mode before calling this.
    pub fn begin_connect<S: Read + Write>(
        self,
        stream: S,
        url: &str,
    ) -> Result<WsConnecting<S>, WsError> {
        let parsed = parse_ws_url(url)?;

        #[cfg(feature = "tls")]
        let tls = if parsed.tls {
            let config = match self.tls_config {
                Some(c) => c,
                None => crate::tls::TlsConfig::new().map_err(WsError::Tls)?,
            };
            Some(TlsCodec::new(&config, parsed.host)?)
        } else {
            None
        };

        #[cfg(not(feature = "tls"))]
        if parsed.tls {
            return Err(WsError::TlsNotEnabled);
        }

        let ws_key = handshake::generate_key();

        #[cfg(feature = "tls")]
        let initial_state = if tls.is_some() {
            ConnectState::TlsWrite
        } else {
            ConnectState::HttpSend
        };

        #[cfg(not(feature = "tls"))]
        let initial_state = ConnectState::HttpSend;

        let mut connecting = WsConnecting {
            stream: std::mem::ManuallyDrop::new(stream),
            state: initial_state,
            #[cfg(feature = "tls")]
            tls,
            reader_builder: self.reader_builder,
            write_buf_capacity: self.write_buf_capacity,
            write_buf_headroom: self.write_buf_headroom,
            ws_key,
            req_buf: Vec::new(),
            req_offset: 0,
            resp_reader: crate::http::ResponseReader::new(4096),
            host: parsed.host.to_owned(),
            path: parsed.path.to_owned(),
            finished: false,
        };

        // Build the HTTP upgrade request for ws:// (no TLS step)
        if matches!(initial_state, ConnectState::HttpSend) {
            let path = connecting.path.clone();
            connecting.prepare_http_request(&path);
        }

        Ok(connecting)
    }
}

impl<S: Read + Write> WsConnecting<S> {
    /// Drive the handshake forward. Non-blocking.
    ///
    /// Returns `Ok(None)` while in progress, `Ok(Some(ws))` when the
    /// connection is ready and [`recv()`](WsStream::recv) can be called.
    ///
    /// Call when the socket is readable or writable (check
    /// [`wants_read()`](Self::wants_read) / [`wants_write()`](Self::wants_write)).
    ///
    /// On `WouldBlock`, returns `Ok(None)` — call again when the socket
    /// is ready.
    pub fn poll(&mut self) -> Result<Option<WsStream<S>>, WsError> {
        loop {
            match self.state {
                #[cfg(feature = "tls")]
                ConnectState::TlsWrite => {
                    let tls = self.tls.as_mut().unwrap();
                    match tls.write_tls_to(&mut *self.stream) {
                        Ok(_) => {}
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                        Err(e) => return Err(e.into()),
                    }
                    if tls.is_handshaking() {
                        self.state = ConnectState::TlsRead;
                    } else {
                        self.state = ConnectState::HttpSend;
                        let path = self.path.clone();
                        self.prepare_http_request(&path);
                    }
                }
                #[cfg(feature = "tls")]
                ConnectState::TlsRead => {
                    let tls = self.tls.as_mut().unwrap();
                    match tls.read_tls_from(&mut *self.stream) {
                        Ok(0) => return Err(WsError::Handshake(HandshakeError::MalformedHttp)),
                        Ok(_) => {}
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                        Err(e) => return Err(e.into()),
                    }
                    tls.process_new_packets()?;
                    if tls.wants_write() {
                        self.state = ConnectState::TlsWrite;
                    } else if !tls.is_handshaking() {
                        self.state = ConnectState::HttpSend;
                        let path = self.path.clone();
                        self.prepare_http_request(&path);
                    }
                }
                ConnectState::HttpSend => {
                    if self.req_offset >= self.req_buf.len() {
                        self.state = ConnectState::HttpRecv;
                        return Ok(None);
                    }

                    #[cfg(feature = "tls")]
                    if let Some(tls) = &mut self.tls {
                        // TLS path: encrypt ALL remaining plaintext at once,
                        // then flush ciphertext. encrypt() consumes everything;
                        // write_tls_to may partially flush.
                        if self.req_offset < self.req_buf.len() {
                            let data = &self.req_buf[self.req_offset..];
                            tls.encrypt(data)?;
                            self.req_offset = self.req_buf.len(); // all plaintext consumed
                        }
                        // Flush whatever ciphertext we can
                        match tls.write_tls_to(&mut *self.stream) {
                            Ok(_) => {}
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                            Err(e) => return Err(e.into()),
                        }
                        // If TLS still has buffered ciphertext, come back later
                        if tls.wants_write() {
                            return Ok(None);
                        }
                        self.state = ConnectState::HttpRecv;
                        return Ok(None);
                    }

                    // Plain WS path: write plaintext directly
                    {
                        let data = &self.req_buf[self.req_offset..];
                        let n = match (*self.stream).write(data) {
                            Ok(n) => n,
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                            Err(e) => return Err(e.into()),
                        };
                        if n == 0 {
                            return Err(WsError::Io(io::Error::new(
                                io::ErrorKind::WriteZero,
                                "write returned 0 during handshake",
                            )));
                        }
                        self.req_offset += n;
                        if self.req_offset >= self.req_buf.len() {
                            self.state = ConnectState::HttpRecv;
                        }
                    }
                    return Ok(None);
                }
                ConnectState::HttpRecv => {
                    let mut tmp = [0u8; 4096];
                    let n = self.read_bytes(&mut tmp)?;
                    if n == 0 { return Ok(None); }

                    self.resp_reader.read(&tmp[..n])
                        .map_err(|_| HandshakeError::MalformedHttp)?;

                    // Check if we have a complete response.
                    // validate_upgrade borrows self immutably, so we
                    // can't call it while resp_reader is mutably borrowed.
                    // next() consumes the response, so we validate inline.
                    match self.resp_reader.next() {
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
                            if !conn.as_bytes().windows(7)
                                .any(|w| w.eq_ignore_ascii_case(b"upgrade"))
                            {
                                return Err(HandshakeError::MissingConnection.into());
                            }
                            let key_str = std::str::from_utf8(&self.ws_key).unwrap();
                            let accept = resp.header("Sec-WebSocket-Accept")
                                .ok_or(HandshakeError::InvalidAcceptKey)?;
                            if !handshake::validate_accept(key_str, accept) {
                                return Err(HandshakeError::InvalidAcceptKey.into());
                            }
                            self.state = ConnectState::Done;
                            // Fall through to Done
                        }
                        Ok(None) => return Ok(None),
                        Err(_) => return Err(HandshakeError::MalformedHttp.into()),
                    }
                }
                ConnectState::Done => {
                    return Ok(Some(self.finish()?));
                }
            }
        }
    }

    /// Whether the handshake needs to write to the socket.
    pub fn wants_write(&self) -> bool {
        matches!(self.state,
            ConnectState::HttpSend
            | if_tls!(ConnectState::TlsWrite)
        )
    }

    /// Whether the handshake needs to read from the socket.
    pub fn wants_read(&self) -> bool {
        matches!(self.state,
            ConnectState::HttpRecv
            | if_tls!(ConnectState::TlsRead)
        )
    }

    /// Access the underlying stream (for mio registration).
    pub fn stream(&self) -> &S { &self.stream }

    /// Mutable access to the underlying stream.
    pub fn stream_mut(&mut self) -> &mut S { &mut self.stream }

    // =========================================================================
    // Internal
    // =========================================================================

    fn prepare_http_request(&mut self, path: &str) {
        let key_str = std::str::from_utf8(&self.ws_key).unwrap();
        let headers = [
            ("Host", self.host.as_str()),
            ("Upgrade", "websocket"),
            ("Connection", "Upgrade"),
            ("Sec-WebSocket-Key", key_str),
            ("Sec-WebSocket-Version", "13"),
        ];
        let size = crate::http::request_size("GET", path, &headers);
        let mut buf = vec![0u8; size];
        // unwrap is safe: buffer is exactly the right size
        let n = crate::http::write_request("GET", path, &headers, &mut buf).unwrap();
        self.req_buf = buf[..n].to_vec();
        self.req_offset = 0;
    }

    fn finish(&mut self) -> Result<WsStream<S>, WsError> {
        self.finished = true;

        let reader_builder = std::mem::replace(
            &mut self.reader_builder,
            FrameReader::builder(),
        );
        let mut reader = reader_builder.role(Role::Client).build();
        let remainder = self.resp_reader.remainder();
        if !remainder.is_empty() {
            reader.read(remainder)
                .map_err(|_| WsError::Handshake(HandshakeError::MalformedHttp))?;
        }

        // SAFETY: stream is ManuallyDrop. We take ownership here.
        // The `finished` flag prevents Drop from dropping it again.
        // finish() is only called once (state == Done).
        let stream = unsafe { std::mem::ManuallyDrop::take(&mut self.stream) };

        #[cfg(feature = "tls")]
        let tls = self.tls.take();

        Ok(WsStream::from_parts_internal(
            stream,
            #[cfg(feature = "tls")]
            tls,
            reader,
            FrameWriter::new(Role::Client),
            WriteBuf::new(self.write_buf_capacity, self.write_buf_headroom),
        ))
    }

    /// Read bytes through TLS or direct.
    fn read_bytes(&mut self, dst: &mut [u8]) -> Result<usize, WsError> {
        #[cfg(feature = "tls")]
        if let Some(tls) = &mut self.tls {
            return match tls.read_tls_from(&mut *self.stream) {
                Ok(0) => Ok(0),
                Ok(_) => {
                    tls.process_new_packets()?;
                    tls.read_plaintext(dst).map_err(WsError::Tls)
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
                Err(e) => Err(e.into()),
            };
        }
        match (*self.stream).read(dst) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e.into()),
        }
    }
}

impl<S> Drop for WsConnecting<S> {
    fn drop(&mut self) {
        if !self.finished {
            // finish() was never called — drop the stream manually.
            // SAFETY: stream hasn't been taken via ManuallyDrop::take.
            unsafe { std::mem::ManuallyDrop::drop(&mut self.stream); }
        }
        // tls is Option — dropped normally by the compiler.
    }
}

// Macro to conditionally include TLS variants in matches!()
#[cfg(feature = "tls")]
macro_rules! if_tls {
    ($pat:pat) => { $pat };
}
#[cfg(not(feature = "tls"))]
macro_rules! if_tls {
    ($pat:pat) => { ConnectState::Done }; // never matches Done twice, but unused
}
use if_tls;
