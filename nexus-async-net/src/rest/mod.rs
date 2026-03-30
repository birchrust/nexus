//! Async HTTP REST client — tokio adapter for nexus-net.
//!
//! Same [`RequestWriter`], same [`ResponseReader`], same zero-alloc
//! protocol layer. The only difference is `.await` on socket I/O.
//!
//! # Usage
//!
//! ```ignore
//! use nexus_net::rest::RequestWriter;
//! use nexus_net::http::ResponseReader;
//! use nexus_async_net::rest::AsyncHttpConnection;
//!
//! let mut writer = RequestWriter::new("api.binance.com");
//! writer.default_header("X-API-KEY", &key)?;
//! let mut reader = ResponseReader::new(32 * 1024);
//!
//! let mut conn = AsyncHttpConnection::connect("https://api.binance.com").await?;
//!
//! let req = writer.get("/orders").query("symbol", "BTC").finish()?;
//! let resp = conn.send(&req, &mut reader).await?;
//! println!("{}", resp.body_str()?);
//! ```

mod atomic_pool;
mod connection;
mod pool;

pub use atomic_pool::{AtomicClientPool, AtomicClientPoolBuilder, AtomicClientSlot};
pub use connection::{AsyncHttpConnection, AsyncHttpConnectionBuilder};
pub use pool::{ClientPool, ClientPoolBuilder, ClientSlot};
