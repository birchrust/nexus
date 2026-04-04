//! Async network primitives.
//!
//! Wraps mio's TCP, UDP, and Unix socket types with async read/write
//! backed by the runtime's IO driver. Each socket registers with mio
//! on first IO attempt and re-registers on `WouldBlock`.
//!
//! # IO Traits
//!
//! [`AsyncRead`] and [`AsyncWrite`] are the core abstractions. They
//! mirror `std::io::Read`/`Write` but return `Poll` and take a `Context`
//! for waker registration. nexus-net codecs program against these traits.

mod io_traits;
mod tcp;

pub use io_traits::{AsyncRead, AsyncWrite};
pub use tcp::{TcpListener, TcpStream};
