use std::io::{self, Read, Write};

use rustls::ClientConnection;
use rustls::pki_types::ServerName;

use super::{TlsConfig, TlsError};
use crate::ws::FrameReader;

/// Sans-IO TLS codec. Decrypts inbound bytes, encrypts outbound bytes.
///
/// Wraps a rustls `ClientConnection` with an API shaped for nexus-net:
/// feed raw TLS bytes in, get plaintext into a [`FrameReader`]; encrypt
/// plaintext from a [`FrameWriter`](crate::ws::FrameWriter) and flush to a socket.
///
/// # Usage
///
/// ```ignore
/// let config = TlsConfig::new()?;
/// let mut tls = TlsCodec::new(&config, "exchange.com")?;
///
/// // Handshake
/// while tls.is_handshaking() {
///     tls.write_tls_to(&mut socket)?;
///     tls.read_tls_from(&mut socket)?;
///     tls.process_new_packets()?;
/// }
///
/// // Steady state
/// tls.read_tls_from(&mut socket)?;
/// tls.process_into(&mut reader)?;
/// // ... reader.next() ...
/// ```
pub struct TlsCodec {
    inner: ClientConnection,
}

impl TlsCodec {
    /// Create a new TLS codec for the given hostname.
    ///
    /// The hostname is used for SNI (Server Name Indication) and
    /// certificate verification.
    pub fn new(config: &TlsConfig, hostname: &str) -> Result<Self, TlsError> {
        let server_name = ServerName::try_from(hostname.to_owned())
            .map_err(|_| TlsError::InvalidHostname(hostname.to_owned()))?;

        let conn = ClientConnection::new(config.inner.clone(), server_name)?;

        Ok(Self { inner: conn })
    }

    // =========================================================================
    // Inbound (socket → TLS → FrameReader)
    // =========================================================================

    /// Feed raw TLS bytes from a byte slice (sans-IO path).
    ///
    /// Returns the number of bytes consumed.
    pub fn read_tls(&mut self, src: &[u8]) -> Result<usize, TlsError> {
        let mut cursor = io::Cursor::new(src);
        Ok(self.inner.read_tls(&mut cursor)?)
    }

    /// Read raw TLS bytes from a socket.
    ///
    /// Returns the number of bytes read, or 0 on EOF.
    pub fn read_tls_from<R: Read>(&mut self, src: &mut R) -> io::Result<usize> {
        self.inner.read_tls(src)
    }

    /// Process buffered TLS records (decrypt).
    ///
    /// Call after [`read_tls`](Self::read_tls) or
    /// [`read_tls_from`](Self::read_tls_from) to decrypt any
    /// complete TLS records. This does not produce plaintext
    /// directly — call [`process_into`](Self::process_into) or
    /// [`read_plaintext`](Self::read_plaintext) afterwards.
    pub fn process_new_packets(&mut self) -> Result<(), TlsError> {
        self.inner.process_new_packets()?;
        Ok(())
    }

    /// Decrypt buffered TLS records and feed plaintext into a FrameReader.
    ///
    /// Combines [`process_new_packets`](Self::process_new_packets) and
    /// a read into the FrameReader in one call. Returns the number of
    /// plaintext bytes fed.
    pub fn process_into(&mut self, reader: &mut FrameReader) -> Result<usize, TlsError> {
        self.inner.process_new_packets()?;

        // Use BufRead::fill_buf to avoid ChunkVecBuffer::read overhead.
        // fill_buf returns a reference to buffered plaintext — one fewer
        // copy than Read::read which copies into an intermediate buffer.
        let mut rd = self.inner.reader();
        let chunk = match std::io::BufRead::fill_buf(&mut rd) {
            Ok(chunk) => chunk,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(0),
            Err(e) => return Err(TlsError::Io(e)),
        };
        if chunk.is_empty() {
            return Ok(0);
        }
        let n = chunk.len();
        if let Err(e) = reader.read(chunk) {
            return Err(TlsError::Io(io::Error::other(format!(
                "FrameReader buffer full: {e}"
            ))));
        }
        std::io::BufRead::consume(&mut rd, n);
        Ok(n)
    }

    /// Read decrypted plaintext into a buffer (sans-IO path).
    ///
    /// For users who want to feed bytes into FrameReader manually
    /// or use a different parser.
    pub fn read_plaintext(&mut self, dst: &mut [u8]) -> Result<usize, TlsError> {
        match self.inner.reader().read(dst) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(TlsError::Io(e)),
        }
    }

    // =========================================================================
    // Outbound (FrameWriter → TLS → socket)
    // =========================================================================

    /// Encrypt plaintext for sending.
    ///
    /// The encrypted bytes are buffered internally. Call
    /// [`write_tls_to`](Self::write_tls_to) to flush them to a socket.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<(), TlsError> {
        self.inner.writer().write_all(plaintext)?;
        Ok(())
    }

    /// Flush encrypted bytes to a socket.
    ///
    /// Returns the number of bytes written. Call in a loop or when
    /// [`wants_write`](Self::wants_write) returns true.
    pub fn write_tls_to<W: Write>(&mut self, dst: &mut W) -> io::Result<usize> {
        self.inner.write_tls(dst)
    }

    // =========================================================================
    // State
    // =========================================================================

    /// Whether the TLS handshake is still in progress.
    pub fn is_handshaking(&self) -> bool {
        self.inner.is_handshaking()
    }

    /// Whether the codec has buffered TLS data to read.
    pub fn wants_read(&self) -> bool {
        self.inner.wants_read()
    }

    /// Whether the codec has encrypted data to write.
    pub fn wants_write(&self) -> bool {
        self.inner.wants_write()
    }
}

impl std::fmt::Debug for TlsCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsCodec")
            .field("handshaking", &self.inner.is_handshaking())
            .finish()
    }
}
