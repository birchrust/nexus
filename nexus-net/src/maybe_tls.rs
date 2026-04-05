//! Stream that may or may not be wrapped in TLS.
//!
//! Implements `Read + Write` (and `AsyncRead + AsyncWrite` when
//! `nexus-rt` is enabled) by delegating to either the plain stream
//! or the [`TlsStream`] wrapper.
//!
//! Protocol clients use `MaybeTls<S>` as their stream type when the
//! TLS decision happens at runtime (`ws://` vs `wss://`).

use std::io::{self, Read, Write};

#[cfg(feature = "tls")]
use crate::tls::TlsStream;

/// A stream that may or may not be wrapped in TLS.
///
/// The `Tls` variant is boxed because `TlsStream` includes rustls's
/// ~1KB connection state. TLS connections are established once at
/// startup — the box indirection is not on the hot path.
pub enum MaybeTls<S> {
    /// Plaintext stream.
    Plain(S),
    /// TLS-wrapped stream.
    #[cfg(feature = "tls")]
    Tls(Box<TlsStream<S>>),
}

impl<S> MaybeTls<S> {
    /// Whether this is a TLS-wrapped stream.
    pub fn is_tls(&self) -> bool {
        #[cfg(feature = "tls")]
        if matches!(self, Self::Tls(_)) {
            return true;
        }
        false
    }
}

// =============================================================================
// Read + Write (blocking)
// =============================================================================

impl<S: Read + Write> Read for MaybeTls<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf),
            #[cfg(feature = "tls")]
            Self::Tls(s) => s.read(buf),
        }
    }
}

impl<S: Read + Write> Write for MaybeTls<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(s) => s.write(buf),
            #[cfg(feature = "tls")]
            Self::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(s) => s.flush(),
            #[cfg(feature = "tls")]
            Self::Tls(s) => s.flush(),
        }
    }
}

// =============================================================================
// AsyncRead + AsyncWrite (nexus-async-rt)
// =============================================================================

#[cfg(feature = "nexus-rt")]
impl<S: nexus_async_rt::AsyncRead + nexus_async_rt::AsyncWrite + Unpin>
    nexus_async_rt::AsyncRead for MaybeTls<S>
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(s) => std::pin::Pin::new(&mut **s).poll_read(cx, buf),
        }
    }
}

#[cfg(feature = "nexus-rt")]
impl<S: nexus_async_rt::AsyncRead + nexus_async_rt::AsyncWrite + Unpin>
    nexus_async_rt::AsyncWrite for MaybeTls<S>
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(s) => std::pin::Pin::new(&mut **s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(s) => std::pin::Pin::new(&mut **s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(s) => std::pin::Pin::new(&mut **s).poll_shutdown(cx),
        }
    }
}

// =============================================================================
// AsyncRead + AsyncWrite (tokio)
// =============================================================================

#[cfg(feature = "tokio")]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> tokio::io::AsyncRead
    for MaybeTls<S>
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(s) => std::pin::Pin::new(&mut **s).poll_read(cx, buf),
        }
    }
}

#[cfg(feature = "tokio")]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite
    for MaybeTls<S>
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(s) => std::pin::Pin::new(&mut **s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(s) => std::pin::Pin::new(&mut **s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "tls")]
            MaybeTls::Tls(s) => std::pin::Pin::new(&mut **s).poll_shutdown(cx),
        }
    }
}
