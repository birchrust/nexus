//! TLS stream wrapper — implements `Read + Write` over the sans-IO codec.
//!
//! Wraps a transport stream `S` and a [`TlsCodec`] into a single type
//! that transparently encrypts/decrypts. Protocol clients (`ws::Client`,
//! `rest::Client`) are generic over `S` — when `S = TlsStream<TcpStream>`,
//! TLS is handled transparently with zero branching in the protocol layer.
//!
//! ```text
//! Client<TlsStream<TcpStream>>   — encrypted
//! Client<TcpStream>              — plaintext
//! ```

use std::io::{self, Read, Write};

use super::codec::TlsCodec;

/// A stream that transparently encrypts and decrypts via [`TlsCodec`].
///
/// Implements `Read` and `Write` by routing through the TLS codec.
/// The inner stream `S` carries raw ciphertext; callers see plaintext.
pub struct TlsStream<S> {
    stream: S,
    codec: TlsCodec,
    /// Pending ciphertext from a partial write. When `poll_write` on the
    /// underlying stream accepts fewer bytes than we produced, the
    /// remainder is stored here and drained on the next poll.
    #[cfg_attr(not(feature = "tokio"), allow(dead_code))]
    pending_write: Vec<u8>,
}

impl<S> TlsStream<S> {
    /// Wrap a transport stream with a TLS codec.
    ///
    /// The codec should already be constructed with the correct hostname.
    /// Call [`handshake`](Self::handshake) to complete the TLS handshake
    /// before reading or writing plaintext.
    pub fn new(stream: S, codec: TlsCodec) -> Self {
        Self {
            stream,
            codec,
            pending_write: Vec::new(),
        }
    }

    /// Access the underlying transport stream.
    pub fn stream(&self) -> &S {
        &self.stream
    }

    /// Mutable access to the underlying transport stream.
    pub fn stream_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Access the TLS codec.
    pub fn codec(&self) -> &TlsCodec {
        &self.codec
    }

    /// Mutable access to the TLS codec.
    pub fn codec_mut(&mut self) -> &mut TlsCodec {
        &mut self.codec
    }

    /// Decompose into the inner stream and codec.
    pub fn into_parts(self) -> (S, TlsCodec) {
        (self.stream, self.codec)
    }
}

impl<S: Read + Write> TlsStream<S> {
    /// Drive the TLS handshake to completion (blocking).
    ///
    /// Call once after construction, before any read/write.
    pub fn handshake(&mut self) -> Result<(), super::TlsError> {
        while self.codec.is_handshaking() {
            if self.codec.wants_write() {
                self.codec.write_tls_to(&mut self.stream)?;
            }
            if self.codec.wants_read() {
                self.codec.read_tls_from(&mut self.stream)?;
                self.codec.process_new_packets()?;
            }
        }
        // Flush any remaining handshake data.
        if self.codec.wants_write() {
            self.codec.write_tls_to(&mut self.stream)?;
        }
        Ok(())
    }
}

// =============================================================================
// Read + Write — blocking path
// =============================================================================

impl<S: Read + Write> Read for TlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Try reading plaintext that's already buffered.
        let n = self.codec.read_plaintext(buf).map_err(tls_to_io)?;
        if n > 0 {
            return Ok(n);
        }

        // Need more ciphertext from the transport.
        // TLS may consume records without producing plaintext (session
        // tickets, key updates). Loop until we get plaintext or EOF.
        loop {
            let tls_n = self.codec.read_tls_from(&mut self.stream)?;
            if tls_n == 0 {
                return Ok(0); // EOF
            }
            self.codec.process_new_packets().map_err(tls_to_io)?;
            let n = self.codec.read_plaintext(buf).map_err(tls_to_io)?;
            if n > 0 {
                return Ok(n);
            }
        }
    }
}

impl<S: Read + Write> Write for TlsStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.codec.encrypt(buf).map_err(tls_to_io)?;
        while self.codec.wants_write() {
            self.codec.write_tls_to(&mut self.stream)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        while self.codec.wants_write() {
            self.codec.write_tls_to(&mut self.stream)?;
        }
        self.stream.flush()
    }
}

// =============================================================================
// AsyncRead + AsyncWrite — tokio path
// =============================================================================

#[cfg(feature = "tokio")]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> TlsStream<S> {
    /// Drive the TLS handshake to completion asynchronously (tokio).
    ///
    /// Call once after construction, before any read/write.
    pub async fn handshake_async(&mut self) -> Result<(), super::TlsError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut tmp = [0u8; 8192];
        while self.codec.is_handshaking() {
            if self.codec.wants_write() {
                let n = self.codec.write_tls_to(&mut tmp.as_mut_slice())?;
                self.stream.write_all(&tmp[..n]).await?;
                self.stream.flush().await?;
            }
            if self.codec.wants_read() {
                let n = self.stream.read(&mut tmp).await?;
                if n == 0 {
                    return Err(super::TlsError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed during TLS handshake",
                    )));
                }
                self.codec.read_tls(&tmp[..n])?;
                self.codec.process_new_packets()?;
            }
        }
        if self.codec.wants_write() {
            let n = self.codec.write_tls_to(&mut tmp.as_mut_slice())?;
            self.stream.write_all(&tmp[..n]).await?;
            self.stream.flush().await?;
        }
        Ok(())
    }
}

#[cfg(feature = "tokio")]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> tokio::io::AsyncRead
    for TlsStream<S>
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();

        // Try reading already-buffered plaintext first.
        let slice = buf.initialize_unfilled();
        let n = this.codec.read_plaintext(slice).map_err(tls_to_io)?;
        if n > 0 {
            buf.advance(n);
            return std::task::Poll::Ready(Ok(()));
        }

        // Need more ciphertext from the transport.
        let mut tmp = [0u8; 8192];
        let mut tmp_buf = tokio::io::ReadBuf::new(&mut tmp);
        match std::pin::Pin::new(&mut this.stream).poll_read(cx, &mut tmp_buf) {
            std::task::Poll::Ready(Ok(())) => {
                let filled = tmp_buf.filled().len();
                if filled == 0 {
                    return std::task::Poll::Ready(Ok(())); // EOF
                }
                this.codec
                    .read_tls(&tmp[..filled])
                    .map_err(tls_to_io)?;
                this.codec.process_new_packets().map_err(tls_to_io)?;
                let slice = buf.initialize_unfilled();
                let pn = this.codec.read_plaintext(slice).map_err(tls_to_io)?;
                if pn > 0 {
                    buf.advance(pn);
                    std::task::Poll::Ready(Ok(()))
                } else {
                    // TLS consumed a non-application record. Need to poll
                    // again — wake ourselves so the executor retries.
                    cx.waker().wake_by_ref();
                    std::task::Poll::Pending
                }
            }
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(e)),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

#[cfg(feature = "tokio")]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite
    for TlsStream<S>
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let this = self.get_mut();

        // Drain any pending ciphertext from a previous partial write.
        if let Err(e) = tokio_drain_pending(this, cx) {
            return std::task::Poll::Ready(Err(e));
        }
        if !this.pending_write.is_empty() {
            // Still have pending data — can't accept more plaintext yet.
            return std::task::Poll::Pending;
        }

        this.codec.encrypt(buf).map_err(tls_to_io)?;

        // Drain ciphertext from rustls into pending_write, then flush
        // as much as we can to the underlying stream.
        let mut tmp = [0u8; 8192];
        while this.codec.wants_write() {
            let n = this.codec.write_tls_to(&mut tmp.as_mut_slice())?;
            this.pending_write.extend_from_slice(&tmp[..n]);
        }

        if let Err(e) = tokio_drain_pending(this, cx) {
            return std::task::Poll::Ready(Err(e));
        }

        // Report all plaintext as written — ciphertext is either flushed
        // or buffered in pending_write for the next poll.
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();

        // Drain any remaining rustls ciphertext.
        let mut tmp = [0u8; 8192];
        while this.codec.wants_write() {
            let n = this.codec.write_tls_to(&mut tmp.as_mut_slice())?;
            this.pending_write.extend_from_slice(&tmp[..n]);
        }

        // Flush pending ciphertext to the stream.
        if let Err(e) = tokio_drain_pending(this, cx) {
            return std::task::Poll::Ready(Err(e));
        }
        if !this.pending_write.is_empty() {
            return std::task::Poll::Pending;
        }

        std::pin::Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

/// Drain pending ciphertext to the underlying tokio stream. Handles partial
/// writes by advancing the buffer.
#[cfg(feature = "tokio")]
fn tokio_drain_pending<S: tokio::io::AsyncWrite + Unpin>(
    this: &mut TlsStream<S>,
    cx: &mut std::task::Context<'_>,
) -> io::Result<()> {
    while !this.pending_write.is_empty() {
        match std::pin::Pin::new(&mut this.stream).poll_write(cx, &this.pending_write) {
            std::task::Poll::Ready(Ok(0)) => {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "write returned 0"));
            }
            std::task::Poll::Ready(Ok(n)) => {
                this.pending_write.drain(..n);
            }
            std::task::Poll::Ready(Err(e)) => return Err(e),
            std::task::Poll::Pending => return Ok(()), // Will be retried next poll.
        }
    }
    Ok(())
}

// =============================================================================
// Helpers
// =============================================================================

fn tls_to_io(e: super::TlsError) -> io::Error {
    match e {
        super::TlsError::Io(io) => io,
        other => io::Error::other(other),
    }
}

