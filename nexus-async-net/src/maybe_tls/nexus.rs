// Consumed by rt_stream (Phase 2b) — no callers yet.
#![allow(dead_code)]
//! MaybeTls — plain TCP or TLS, unified async I/O (nexus-async-rt backend).
//!
//! Unlike the tokio variant which delegates TLS to `tokio-rustls`, this
//! drives nexus-net's sans-IO [`TlsCodec`] at the poll level. The codec
//! handles encrypt/decrypt; we shuttle bytes between it and the TCP stream.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use nexus_async_rt::{AsyncRead, AsyncWrite, TcpStream};

/// Async stream that may or may not be TLS-wrapped.
///
/// Created by connection builders based on the URL scheme.
pub enum MaybeTls {
    /// Plain TCP (ws://, http://).
    Plain(TcpStream),
    /// TLS over TCP (wss://, https://).
    #[cfg(feature = "tls")]
    Tls(Box<TlsInner>),
}

/// TLS state: a TCP stream plus the sans-IO codec and a write staging buffer.
///
/// Opaque to users — fields are `pub(crate)`. Exposed only because
/// [`MaybeTls::Tls`] holds a `Box<TlsInner>`.
#[cfg(feature = "tls")]
pub struct TlsInner {
    pub(crate) stream: TcpStream,
    pub(crate) codec: nexus_net::tls::TlsCodec,
    /// Ciphertext waiting to be flushed to the transport.
    pending_write: Vec<u8>,
}

#[cfg(feature = "tls")]
impl TlsInner {
    pub(crate) fn new(stream: TcpStream, codec: nexus_net::tls::TlsCodec) -> Self {
        Self {
            stream,
            codec,
            pending_write: Vec::with_capacity(16_384),
        }
    }
}

impl MaybeTls {
    /// Whether this connection is TLS-wrapped.
    pub fn is_tls(&self) -> bool {
        match self {
            Self::Plain(_) => false,
            #[cfg(feature = "tls")]
            Self::Tls(_) => true,
        }
    }
}

// =============================================================================
// AsyncRead
// =============================================================================

impl AsyncRead for MaybeTls {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(inner) => {
                // Try already-buffered plaintext first.
                let n = inner.codec.read_plaintext(buf).map_err(tls_to_io)?;
                if n > 0 {
                    return Poll::Ready(Ok(n));
                }

                // Need more ciphertext from the transport.
                let mut tmp = [0u8; 8192];
                match Pin::new(&mut inner.stream).poll_read(cx, &mut tmp) {
                    Poll::Ready(Ok(0)) => Poll::Ready(Ok(0)), // EOF
                    Poll::Ready(Ok(n)) => {
                        inner.codec.read_tls(&tmp[..n]).map_err(tls_to_io)?;
                        inner.codec.process_new_packets().map_err(tls_to_io)?;
                        let pn = inner.codec.read_plaintext(buf).map_err(tls_to_io)?;
                        if pn > 0 {
                            Poll::Ready(Ok(pn))
                        } else {
                            // Non-application TLS record consumed (handshake, alert, etc.).
                            // Wake self to retry.
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        }
                    }
                    Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

// =============================================================================
// AsyncWrite
// =============================================================================

impl AsyncWrite for MaybeTls {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(inner) => {
                // Drain any pending ciphertext before encrypting more.
                drain_pending(inner, cx)?;
                if !inner.pending_write.is_empty() {
                    // Couldn't drain — backpressure.
                    return Poll::Pending;
                }

                // Encrypt plaintext through the codec.
                inner.codec.encrypt(buf).map_err(tls_to_io)?;

                // Collect resulting ciphertext into pending_write.
                inner
                    .codec
                    .write_tls_to(&mut inner.pending_write)
                    .map_err(io::Error::other)?;

                // Best-effort drain of what we just encrypted.
                drain_pending(inner, cx)?;

                Poll::Ready(Ok(buf.len()))
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(inner) => {
                // Drain any codec ciphertext not yet staged.
                if inner.codec.wants_write() {
                    inner
                        .codec
                        .write_tls_to(&mut inner.pending_write)
                        .map_err(io::Error::other)?;
                }

                // Drain pending_write to the transport.
                drain_pending(inner, cx)?;
                if !inner.pending_write.is_empty() {
                    return Poll::Pending;
                }

                // Flush the underlying stream.
                Pin::new(&mut inner.stream).poll_flush(cx)
            }
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(inner) => Pin::new(&mut inner.stream).poll_shutdown(cx),
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Drain the `pending_write` buffer to the transport, writing as much as the
/// socket will accept without blocking.
#[cfg(feature = "tls")]
fn drain_pending(inner: &mut TlsInner, cx: &mut Context<'_>) -> io::Result<()> {
    while !inner.pending_write.is_empty() {
        match Pin::new(&mut inner.stream).poll_write(cx, &inner.pending_write) {
            Poll::Ready(Ok(0)) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "transport write returned 0",
                ));
            }
            Poll::Ready(Ok(n)) => {
                inner.pending_write.drain(..n);
            }
            Poll::Ready(Err(e)) => return Err(e),
            Poll::Pending => return Ok(()), // will retry on next poll
        }
    }
    Ok(())
}

/// Convert a [`TlsError`](nexus_net::tls::TlsError) into an [`io::Error`].
#[cfg(feature = "tls")]
fn tls_to_io(e: nexus_net::tls::TlsError) -> io::Error {
    match e {
        nexus_net::tls::TlsError::Io(io_err) => io_err,
        other => io::Error::other(other),
    }
}
