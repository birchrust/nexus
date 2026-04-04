//! Async TCP stream and listener.
//!
//! Wraps mio's TCP types with the runtime's IO driver for readiness-based
//! async IO. Sockets register with mio lazily on first poll — the task
//! pointer comes from the `Context`'s waker.

use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use mio::{Interest, Token};

use super::io_traits::{AsyncRead, AsyncWrite};
use crate::io::IoHandle;

// =============================================================================
// TcpStream
// =============================================================================

/// Async TCP stream backed by mio.
///
/// Created via [`TcpListener::accept`] or [`TcpStream::connect`].
/// Implements [`AsyncRead`] and [`AsyncWrite`].
///
/// The stream registers with mio lazily on the first read or write
/// attempt. On `WouldBlock`, the current task's waker is re-registered
/// so the IO driver can wake it when the socket becomes ready.
pub struct TcpStream {
    inner: mio::net::TcpStream,
    io: IoHandle,
    token: Option<Token>,
}

impl TcpStream {
    /// Wrap a mio TcpStream. Registration deferred to first poll.
    pub(crate) fn new(inner: mio::net::TcpStream, io: IoHandle) -> Self {
        Self {
            inner,
            io,
            token: None,
        }
    }

    /// Initiate an async TCP connection to `addr`.
    ///
    /// The connection completes asynchronously. The first `poll_write`
    /// or `poll_read` will register with mio and detect when the
    /// connection is established.
    pub fn connect(addr: SocketAddr, io: IoHandle) -> io::Result<Self> {
        let inner = mio::net::TcpStream::connect(addr)?;
        Ok(Self::new(inner, io))
    }

    /// Returns the local address of this stream.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    /// Returns the remote address of this stream.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.inner.peer_addr()
    }

    /// Set TCP_NODELAY (disable Nagle's algorithm).
    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.inner.set_nodelay(nodelay)
    }

    /// Ensure registered with mio. Registers on first call, re-registers
    /// on subsequent calls (to update the task pointer from waker).
    fn ensure_registered(&mut self, cx: &Context<'_>) -> io::Result<()> {
        let task_ptr = waker_to_ptr(cx);
        let interest = Interest::READABLE | Interest::WRITABLE;

        match self.token {
            None => {
                // SAFETY: IoHandle valid (Runtime lifetime). task_ptr
                // extracted from waker (valid for task lifetime).
                let token = unsafe {
                    self.io.register(&mut self.inner, interest, task_ptr)?
                };
                self.token = Some(token);
                Ok(())
            }
            Some(token) => {
                // SAFETY: same invariants. Re-register to update task ptr.
                unsafe {
                    self.io.reregister(&mut self.inner, token, interest, task_ptr)?;
                }
                Ok(())
            }
        }
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match this.inner.read(buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if let Err(e) = this.ensure_registered(cx) {
                    return Poll::Ready(Err(e));
                }
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match this.inner.write(buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if let Err(e) = this.ensure_registered(cx) {
                    return Poll::Ready(Err(e));
                }
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.inner.flush() {
            Ok(()) => Poll::Ready(Ok(())),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if let Err(e) = this.ensure_registered(cx) {
                    return Poll::Ready(Err(e));
                }
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.inner.shutdown(std::net::Shutdown::Write) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(e) if e.kind() == io::ErrorKind::NotConnected => {
                Poll::Ready(Ok(())) // already shut down
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl TcpStream {
    /// Read bytes from the stream. Returns when at least 1 byte is read
    /// or EOF (0 bytes).
    pub async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_read(cx, buf)).await
    }

    /// Write bytes to the stream. Returns when at least 1 byte is written.
    pub async fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_write(cx, buf)).await
    }

    /// Write all bytes to the stream.
    pub async fn write_all(&mut self, mut buf: &[u8]) -> io::Result<()> {
        while !buf.is_empty() {
            let n = self.write(buf).await?;
            buf = &buf[n..];
        }
        Ok(())
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        if let Some(token) = self.token {
            // SAFETY: IoHandle valid (Runtime lifetime).
            let _ = unsafe { self.io.deregister(&mut self.inner, token) };
        }
    }
}

// =============================================================================
// TcpListener
// =============================================================================

/// Async TCP listener backed by mio.
///
/// Bind with [`TcpListener::bind`], then call [`accept`](Self::accept)
/// to await incoming connections.
pub struct TcpListener {
    inner: mio::net::TcpListener,
    io: IoHandle,
    token: Option<Token>,
}

impl TcpListener {
    /// Bind to `addr`. Registration deferred to first `accept` poll.
    pub fn bind(addr: SocketAddr, io: IoHandle) -> io::Result<Self> {
        let inner = mio::net::TcpListener::bind(addr)?;
        Ok(Self {
            inner,
            io,
            token: None,
        })
    }

    /// Returns the local address this listener is bound to.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    /// Accept a new TCP connection.
    pub fn accept(&mut self) -> Accept<'_> {
        Accept { listener: self }
    }

    /// Ensure registered with mio for READABLE interest.
    fn ensure_registered(&mut self, cx: &Context<'_>) -> io::Result<()> {
        let task_ptr = waker_to_ptr(cx);
        match self.token {
            None => {
                // SAFETY: IoHandle valid, task_ptr from waker.
                let token = unsafe {
                    self.io.register(&mut self.inner, Interest::READABLE, task_ptr)?
                };
                self.token = Some(token);
                Ok(())
            }
            Some(token) => {
                // SAFETY: same invariants.
                unsafe {
                    self.io.reregister(&mut self.inner, token, Interest::READABLE, task_ptr)?;
                }
                Ok(())
            }
        }
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        if let Some(token) = self.token {
            let _ = unsafe { self.io.deregister(&mut self.inner, token) };
        }
    }
}

/// Future returned by [`TcpListener::accept`].
pub struct Accept<'a> {
    listener: &'a mut TcpListener,
}

impl std::future::Future for Accept<'_> {
    type Output = io::Result<(TcpStream, SocketAddr)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match this.listener.inner.accept() {
            Ok((stream, addr)) => {
                let tcp = TcpStream::new(stream, this.listener.io);
                Poll::Ready(Ok((tcp, addr)))
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if let Err(e) = this.listener.ensure_registered(cx) {
                    return Poll::Ready(Err(e));
                }
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

// =============================================================================
// Helper
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DefaultRuntime, spawn};
    use nexus_rt::WorldBuilder;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    #[test]
    fn tcp_echo() {
        let wb = WorldBuilder::new();
        let mut world = wb.build();
        let mut rt = DefaultRuntime::new(&mut world, 16);
        let handle = rt.handle();

        // Bind listener before block_on so we know the port.
        let listener = TcpListener::bind(
            "127.0.0.1:0".parse().unwrap(),
            handle.io(),
        ).expect("bind failed");
        let addr = listener.local_addr().unwrap();

        let done = Rc::new(Cell::new(false));
        let done2 = done.clone();

        rt.block_on(async move {
            // Server task: accept one connection, echo back.
            spawn(async move {
                let mut listener = listener;
                let (mut stream, _peer) = listener.accept().await.unwrap();
                let mut buf = [0u8; 64];
                let n = stream.read(&mut buf).await.unwrap();
                stream.write_all(&buf[..n]).await.unwrap();
            });

            // Client task: connect, send, read echo, signal done.
            let io = handle.io();
            let flag = done2;
            spawn(async move {
                // Small delay for the server to start accepting.
                handle.sleep(Duration::from_millis(10)).await;
                let mut client = TcpStream::connect(addr, io).unwrap();
                client.write_all(b"hello").await.unwrap();
                let mut buf = [0u8; 64];
                let n = client.read(&mut buf).await.unwrap();
                assert_eq!(&buf[..n], b"hello");
                flag.set(true);
            });

            // Root future waits for both tasks to complete.
            // Simple approach: sleep long enough for them to finish.
            handle.sleep(Duration::from_millis(500)).await;
        });

        assert!(done.get(), "echo exchange never completed");
    }
}

/// Extract the task pointer from a `Context`'s waker.
///
/// Our wakers store the task pointer as the `RawWaker` data field.
/// `Waker` layout is `[vtable, data]` — data is at offset 8.
/// Validated by the build script at compile time.
fn waker_to_ptr(cx: &Context<'_>) -> *mut u8 {
    // SAFETY: Waker layout validated by build script. data at offset 8.
    let waker_ptr = cx.waker() as *const std::task::Waker as *const [*const (); 2];
    let data = unsafe { (*waker_ptr)[1] };
    data as *mut u8
}
