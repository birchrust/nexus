//! TLS codec — sans-IO encrypt/decrypt via rustls.
//!
//! Sits between the socket and protocol parsers:
//!
//! ```text
//! socket → TlsCodec (decrypt) → FrameReader / ResponseReader → Message
//! Request → TlsCodec (encrypt) → socket
//! ```
//!
//! # Quick Start
//!
//! ```ignore
//! use nexus_net::tls::TlsConfig;
//! use nexus_net::ws::Client;
//!
//! let tls = TlsConfig::new()?;
//! let mut ws = Client::builder()
//!     .tls(&tls)
//!     .connect("wss://exchange.com/ws/v1")?;
//!
//! while let Some(msg) = ws.recv()? {
//!     process(msg);
//! }
//! ```

mod codec;
mod config;
mod error;
mod stream;

pub use codec::TlsCodec;
pub use config::{TlsConfig, TlsConfigBuilder};
pub use error::TlsError;
pub use stream::TlsStream;
