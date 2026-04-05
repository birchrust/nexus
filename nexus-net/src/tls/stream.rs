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
}

impl<S> TlsStream<S> {
    /// Wrap a transport stream with a TLS codec.
    ///
    /// The codec should already be constructed with the correct hostname.
    /// Call [`handshake`](Self::handshake) to complete the TLS handshake
    /// before reading or writing plaintext.
    pub fn new(stream: S, codec: TlsCodec) -> Self {
        Self { stream, codec }
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
// AsyncRead + AsyncWrite — nexus-async-rt path
// =============================================================================

#[cfg(feature = "nexus-rt")]
impl<S: nexus_async_rt::AsyncRead + nexus_async_rt::AsyncWrite + Unpin> TlsStream<S> {
    /// Drive the TLS handshake to completion asynchronously.
    ///
    /// Call once after construction, before any read/write.
    pub async fn handshake_async(&mut self) -> Result<(), super::TlsError> {
        let mut tmp = [0u8; 8192];
        while self.codec.is_handshaking() {
            if self.codec.wants_write() {
                let n = self.codec.write_tls_to(&mut tmp.as_mut_slice())?;
                write_all_async(&mut self.stream, &tmp[..n]).await?;
            }
            if self.codec.wants_read() {
                let n = read_async(&mut self.stream, &mut tmp).await?;
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
            write_all_async(&mut self.stream, &tmp[..n]).await?;
        }
        Ok(())
    }
}

#[cfg(feature = "nexus-rt")]
impl<S: nexus_async_rt::AsyncRead + nexus_async_rt::AsyncWrite + Unpin>
    nexus_async_rt::AsyncRead for TlsStream<S>
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let this = self.get_mut();

        // Try reading already-buffered plaintext first.
        let n = this.codec.read_plaintext(buf).map_err(tls_to_io)?;
        if n > 0 {
            return std::task::Poll::Ready(Ok(n));
        }

        // Need more ciphertext from the transport.
        let mut tmp = [0u8; 8192];
        match std::pin::Pin::new(&mut this.stream).poll_read(cx, &mut tmp) {
            std::task::Poll::Ready(Ok(0)) => std::task::Poll::Ready(Ok(0)),
            std::task::Poll::Ready(Ok(n)) => {
                this.codec.read_tls(&tmp[..n]).map_err(tls_to_io)?;
                this.codec.process_new_packets().map_err(tls_to_io)?;
                let pn = this.codec.read_plaintext(buf).map_err(tls_to_io)?;
                if pn > 0 {
                    std::task::Poll::Ready(Ok(pn))
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

#[cfg(feature = "nexus-rt")]
impl<S: nexus_async_rt::AsyncRead + nexus_async_rt::AsyncWrite + Unpin>
    nexus_async_rt::AsyncWrite for TlsStream<S>
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let this = self.get_mut();

        this.codec.encrypt(buf).map_err(tls_to_io)?;

        // Flush ciphertext. If the underlying stream can't accept it
        // all, we still report all plaintext as written (it's buffered
        // in rustls). The flush will drain the rest.
        let mut tmp = [0u8; 8192];
        while this.codec.wants_write() {
            let n = this.codec.write_tls_to(&mut tmp.as_mut_slice())?;
            match std::pin::Pin::new(&mut this.stream).poll_write(cx, &tmp[..n]) {
                std::task::Poll::Ready(Ok(_)) => {}
                std::task::Poll::Ready(Err(e)) => return std::task::Poll::Ready(Err(e)),
                std::task::Poll::Pending => {
                    // Underlying stream not ready. Plaintext is already
                    // encrypted in rustls buffer. Return Pending — flush
                    // will drain later.
                    return std::task::Poll::Pending;
                }
            }
        }
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();

        let mut tmp = [0u8; 8192];
        while this.codec.wants_write() {
            let n = this.codec.write_tls_to(&mut tmp.as_mut_slice())?;
            match std::pin::Pin::new(&mut this.stream).poll_write(cx, &tmp[..n]) {
                std::task::Poll::Ready(Ok(_)) => {}
                std::task::Poll::Ready(Err(e)) => return std::task::Poll::Ready(Err(e)),
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
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

// =============================================================================
// Helpers
// =============================================================================

fn tls_to_io(e: super::TlsError) -> io::Error {
    match e {
        super::TlsError::Io(io) => io,
        other => io::Error::other(other),
    }
}

#[cfg(feature = "nexus-rt")]
async fn read_async<S: nexus_async_rt::AsyncRead + Unpin>(
    stream: &mut S,
    buf: &mut [u8],
) -> io::Result<usize> {
    std::future::poll_fn(|cx| std::pin::Pin::new(&mut *stream).poll_read(cx, buf)).await
}

#[cfg(feature = "nexus-rt")]
async fn write_all_async<S: nexus_async_rt::AsyncWrite + Unpin>(
    stream: &mut S,
    mut buf: &[u8],
) -> io::Result<()> {
    while !buf.is_empty() {
        let n =
            std::future::poll_fn(|cx| std::pin::Pin::new(&mut *stream).poll_write(cx, buf))
                .await?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "write returned 0"));
        }
        buf = &buf[n..];
    }
    std::future::poll_fn(|cx| std::pin::Pin::new(&mut *stream).poll_flush(cx)).await
}
