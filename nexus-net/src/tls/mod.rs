//! TLS codec — sans-IO encrypt/decrypt via rustls.
//!
//! Sits between the socket and [`FrameReader`](crate::ws::FrameReader)/[`FrameWriter`](crate::ws::FrameWriter):
//!
//! ```text
//! socket → TlsCodec (decrypt) → FrameReader → Message
//! Message → FrameWriter → TlsCodec (encrypt) → socket
//! ```
//!
//! # Quick Start
//!
//! ```ignore
//! use nexus_net::tls::TlsConfig;
//! use nexus_net::ws::WsTlsStream;
//!
//! let tcp = TcpStream::connect("exchange.com:443")?;
//! let tls_config = TlsConfig::new()?;
//! let mut ws = WsTlsStream::connect(tcp, &tls_config, "wss://exchange.com/ws/v1")?;
//!
//! while let Some(msg) = ws.next()? {
//!     process(msg);
//! }
//! ```

mod codec;
mod config;
mod error;

pub use codec::TlsCodec;
pub use config::{TlsConfig, TlsConfigBuilder};
pub use error::TlsError;
