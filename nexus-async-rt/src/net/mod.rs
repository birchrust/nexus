//! Async network primitives.
//!
//! Wraps mio's TCP and UDP types with async read/write backed by the
//! runtime's IO driver. Sockets register with mio lazily on first IO
//! attempt and re-register on `WouldBlock`.
//!
//! # IO Traits
//!
//! [`AsyncRead`] and [`AsyncWrite`] are the core abstractions. They
//! mirror `std::io::Read`/`Write` but return `Poll` and take a `Context`
//! for waker registration. nexus-net codecs program against these traits.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

pub mod tcp;
pub mod udp;

pub use tcp::{
    Accept, OwnedReadHalf, OwnedWriteHalf, ReadHalf, ReuniteError, TcpListener, TcpSocket,
    TcpStream, WriteHalf,
};
pub use udp::UdpSocket;

// =============================================================================
// AsyncRead / AsyncWrite
// =============================================================================

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

// =============================================================================
// Helper: extract task pointer from waker
// =============================================================================

/// Extract the task pointer from a `Context`'s waker.
///
/// Our wakers store the task pointer as the `RawWaker` data field.
/// `Waker` layout is `[vtable, data]` — data is at offset 8.
/// Validated by the build script at compile time.
pub(crate) fn waker_to_ptr(cx: &Context<'_>) -> *mut u8 {
    // SAFETY: Waker layout validated by build script. data at offset 8.
    let waker_ptr = cx.waker() as *const std::task::Waker as *const [*const (); 2];
    let data = unsafe { (*waker_ptr)[1] };
    data as *mut u8
}
