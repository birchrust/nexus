//! Async IO traits.
//!
//! Simpler than tokio's — no `ReadBuf`, just `&mut [u8]`. These are
//! the traits that nexus-net codecs and user code program against.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Async read half of a byte stream.
///
/// Mirrors `std::io::Read` but returns `Poll` for non-blocking use
/// with the executor.
///
/// # Contract
///
/// - `Poll::Ready(Ok(0))` means EOF — the peer closed its write half.
/// - `Poll::Ready(Ok(n))` means `n` bytes were read into `buf[..n]`.
/// - `Poll::Pending` means no data is available yet — the waker will
///   be notified when the stream becomes readable.
/// - `Poll::Ready(Err(e))` is a fatal IO error.
pub trait AsyncRead {
    /// Attempt to read from the stream into `buf`.
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>>;
}

/// Async write half of a byte stream.
///
/// Mirrors `std::io::Write` but returns `Poll` for non-blocking use.
///
/// # Contract
///
/// - `Poll::Ready(Ok(n))` means `n` bytes from `buf[..n]` were written.
/// - `Poll::Pending` means the write buffer is full — the waker will
///   be notified when the stream becomes writable.
/// - `poll_flush` ensures all buffered data reaches the OS send buffer.
/// - `poll_shutdown` signals that no more data will be written (TCP FIN).
pub trait AsyncWrite {
    /// Attempt to write `buf` to the stream.
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>>;

    /// Flush any buffered data to the underlying transport.
    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>>;

    /// Initiate graceful shutdown of the write half.
    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>>;
}
