//! Thread-safe async HTTP client pool.
//!
//! Uses [`nexus_pool::sync::Pool`] for LIFO acquire/release with RAII guards.
//! Single acquirer, any thread can return. Inline reconnect on acquire.
//!
//! For multi-threaded tokio runtime.

use nexus_net::http::ResponseReader;
use nexus_net::rest::{RequestWriter, RestError};
#[cfg(feature = "tls")]
use nexus_net::tls::TlsConfig;
use nexus_pool::sync::{Pool, Pooled};

use super::connection::{AsyncHttpConnection, AsyncHttpConnectionBuilder};
use crate::maybe_tls::MaybeTls;

// =============================================================================
// AtomicClientSlot
// =============================================================================

/// Thread-safe client slot. Same type as [`ClientSlot`](super::ClientSlot) —
/// the slot is identical regardless of pool type.
pub type AtomicClientSlot = super::ClientSlot;

// =============================================================================
// AtomicClientPool
// =============================================================================

/// Thread-safe async HTTP client pool.
///
/// Pre-allocated slots with LIFO acquire. **Single acquirer, any
/// returner** — acquire from one task, guards can be dropped from
/// any thread. Uses atomic CAS for lock-free release.
///
/// Note: `sync::Pool` is `Send` but not `Sync`. Wrap in `Mutex` if
/// you need shared acquire across multiple tasks on different threads.
///
/// # Usage
///
/// ```ignore
/// let pool = AtomicClientPool::builder()
///     .url("https://api.binance.com")
///     .base_path("/api/v3")
///     .default_header("X-API-KEY", &key)?
///     .connections(4)
///     .tls(&tls)
///     .build()
///     .await?;
///
/// let mut slot = pool.try_acquire().unwrap();
/// let req = slot.writer.post("/order").body(json).finish()?;
/// let conn = slot.conn.as_mut().unwrap();
/// let resp = conn.send(req, &mut slot.reader).await?;
/// // drop(slot) returns to pool from any thread
/// ```
pub struct AtomicClientPool {
    pool: Pool<AtomicClientSlot>,
    reconnect_config: ReconnectConfig,
}

#[derive(Clone)]
struct ReconnectConfig {
    url: String,
    #[cfg(feature = "tls")]
    tls_config: Option<TlsConfig>,
    nodelay: bool,
}

impl AtomicClientPool {
    /// Create a builder.
    #[must_use]
    pub fn builder() -> AtomicClientPoolBuilder {
        AtomicClientPoolBuilder::new()
    }

    /// Try to acquire a slot. Returns `None` if all slots are in use.
    ///
    /// If the acquired slot has a dead connection, reconnects inline.
    pub async fn try_acquire(
        &self,
    ) -> Result<Option<Pooled<AtomicClientSlot>>, RestError> {
        match self.pool.try_acquire() {
            Some(mut slot) => {
                if slot.needs_reconnect() {
                    self.reconnect(&mut slot).await?;
                }
                Ok(Some(slot))
            }
            None => Ok(None),
        }
    }

    /// Number of slots currently available.
    pub fn available(&self) -> usize {
        self.pool.available()
    }

    async fn reconnect(&self, slot: &mut AtomicClientSlot) -> Result<(), RestError> {
        let conn = self.connect_one().await?;
        slot.conn = Some(conn);
        Ok(())
    }

    async fn connect_one(&self) -> Result<AsyncHttpConnection<MaybeTls>, RestError> {
        let mut builder = AsyncHttpConnectionBuilder::new();
        #[cfg(feature = "tls")]
        if let Some(ref tls) = self.reconnect_config.tls_config {
            builder = builder.tls(tls);
        }
        if self.reconnect_config.nodelay {
            builder = builder.disable_nagle();
        }
        builder.connect(&self.reconnect_config.url).await
    }
}

// =============================================================================
// Builder
// =============================================================================

/// Builder for [`AtomicClientPool`].
pub struct AtomicClientPoolBuilder {
    url: String,
    base_path: String,
    default_headers: Vec<(String, String)>,
    connections: usize,
    #[cfg(feature = "tls")]
    tls_config: Option<TlsConfig>,
    nodelay: bool,
    write_buffer_capacity: usize,
    response_buffer_capacity: usize,
    max_body_size: usize,
}

impl AtomicClientPoolBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            url: String::new(),
            base_path: String::new(),
            default_headers: Vec::new(),
            connections: 1,
            #[cfg(feature = "tls")]
            tls_config: None,
            nodelay: false,
            write_buffer_capacity: 32 * 1024,
            response_buffer_capacity: 32 * 1024,
            max_body_size: 0,
        }
    }

    /// Target URL.
    #[must_use]
    pub fn url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    /// Base path prefix.
    #[must_use]
    pub fn base_path(mut self, path: &str) -> Self {
        self.base_path = path.to_string();
        self
    }

    /// Add a default header.
    pub fn default_header(mut self, name: &str, value: &str) -> Result<Self, RestError> {
        if name.bytes().any(|b| b == b'\r' || b == b'\n')
            || value.bytes().any(|b| b == b'\r' || b == b'\n')
        {
            return Err(RestError::CrlfInjection);
        }
        self.default_headers
            .push((name.to_string(), value.to_string()));
        Ok(self)
    }

    /// Number of connections. Default: 1.
    #[must_use]
    pub fn connections(mut self, n: usize) -> Self {
        self.connections = n;
        self
    }

    /// Custom TLS configuration.
    #[must_use]
    #[cfg(feature = "tls")]
    pub fn tls(mut self, config: &TlsConfig) -> Self {
        self.tls_config = Some(config.clone());
        self
    }

    /// Disable Nagle's algorithm.
    #[must_use]
    pub fn disable_nagle(mut self) -> Self {
        self.nodelay = true;
        self
    }

    /// Write buffer capacity per slot. Default: 32KB.
    #[must_use]
    pub fn write_buffer_capacity(mut self, n: usize) -> Self {
        self.write_buffer_capacity = n;
        self
    }

    /// Response buffer capacity per slot. Default: 32KB.
    #[must_use]
    pub fn response_buffer_capacity(mut self, n: usize) -> Self {
        self.response_buffer_capacity = n;
        self
    }

    /// Maximum response body size per slot. Default: 0 (no limit).
    #[must_use]
    pub fn max_body_size(mut self, n: usize) -> Self {
        self.max_body_size = n;
        self
    }

    /// Build the pool, establishing all connections.
    pub async fn build(self) -> Result<AtomicClientPool, RestError> {
        if self.url.is_empty() {
            return Err(RestError::InvalidUrl("url is required".to_string()));
        }
        if self.connections == 0 {
            return Err(RestError::InvalidUrl("connections must be > 0".to_string()));
        }

        let parsed = nexus_net::rest::parse_base_url(&self.url)?;
        let host_header = parsed.host_header();

        let reconnect_config = ReconnectConfig {
            url: self.url.clone(),
            #[cfg(feature = "tls")]
            tls_config: self.tls_config.clone(),
            nodelay: self.nodelay,
        };

        // Build the init and reset closures for sync::Pool.
        let host = host_header.clone();
        let base = self.base_path.clone();
        let headers = self.default_headers.clone();
        let wbuf_cap = self.write_buffer_capacity;
        let rbuf_cap = self.response_buffer_capacity;
        let max_body = self.max_body_size;

        let pool = Pool::new(
            self.connections,
            // Init — creates disconnected slots.
            move || {
                let mut writer =
                    RequestWriter::new(&host).expect("host already validated");
                if !base.is_empty() {
                    writer
                        .set_base_path(&base)
                        .expect("base_path already validated");
                }
                writer.set_write_buffer_capacity(wbuf_cap);
                for (name, value) in &headers {
                    writer
                        .default_header(name, value)
                        .expect("headers already validated");
                }
                AtomicClientSlot {
                    writer,
                    reader: ResponseReader::new(rbuf_cap).max_body_size(max_body),
                    conn: None,
                }
            },
            // Reset — clear dead connections on return.
            |slot: &mut AtomicClientSlot| {
                if slot.needs_reconnect() {
                    slot.conn = None;
                    slot.reader.reset();
                }
            },
        );

        // Replace disconnected slots with connected ones.
        for _ in 0..self.connections {
            {
                let mut slot = pool.try_acquire()
                    .expect("pool should have slots during initial setup");
                let mut builder = AsyncHttpConnectionBuilder::new();
            #[cfg(feature = "tls")]
                if let Some(ref tls) = self.tls_config {
                    builder = builder.tls(tls);
                }
                if self.nodelay {
                    builder = builder.disable_nagle();
                }
                let conn = builder.connect(&self.url).await?;
                slot.conn = Some(conn);
                // Drop returns it to the pool (reset runs but conn is healthy).
            }
        }

        Ok(AtomicClientPool {
            pool,
            reconnect_config,
        })
    }
}

impl Default for AtomicClientPoolBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pool(n: usize) -> Pool<AtomicClientSlot> {
        Pool::new(
            n,
            || AtomicClientSlot {
                writer: RequestWriter::new("host").unwrap(),
                reader: ResponseReader::new(4096),
                conn: None,
            },
            |slot| {
                if slot.needs_reconnect() {
                    slot.conn = None;
                }
            },
        )
    }

    #[test]
    fn atomic_pool_acquire_release() {
        let pool = make_pool(2);

        assert_eq!(pool.available(), 2);
        let s1 = pool.try_acquire().unwrap();
        assert_eq!(pool.available(), 1);
        let s2 = pool.try_acquire().unwrap();
        assert_eq!(pool.available(), 0);
        assert!(pool.try_acquire().is_none());

        drop(s1);
        assert_eq!(pool.available(), 1);
        drop(s2);
        assert_eq!(pool.available(), 2);
    }

    #[test]
    fn atomic_slot_needs_reconnect() {
        let slot = AtomicClientSlot {
            writer: RequestWriter::new("host").unwrap(),
            reader: ResponseReader::new(4096),
            conn: None,
        };
        assert!(slot.needs_reconnect());
    }

    #[tokio::test]
    async fn atomic_pool_loopback() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = tcp.read(&mut buf).await.unwrap();
            let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
            tcp.write_all(resp).await.unwrap();
        });

        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let stream = MaybeTls::Plain(tcp);
        let conn = AsyncHttpConnection::new(stream);

        let pool = make_pool(1);
        {
            let mut slot = pool.try_acquire().unwrap();
            slot.writer = RequestWriter::new(&addr.to_string()).unwrap();
            slot.conn = Some(conn);
        } // return to pool

        let mut slot = pool.try_acquire().unwrap();
        let s: &mut AtomicClientSlot = &mut slot;
        let req = s.writer.get("/test").finish().unwrap();
        let conn = s.conn.as_mut().unwrap();
        let resp = conn.send(req, &mut s.reader).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.body_str().unwrap(), "ok");
    }

    #[test]
    fn atomic_try_acquire_none_when_exhausted() {
        let pool = make_pool(1);
        let _s1 = pool.try_acquire().unwrap();
        assert!(pool.try_acquire().is_none());
    }

    #[test]
    fn atomic_conn_and_reader_error_when_no_conn() {
        let pool = make_pool(1);
        let mut slot = pool.try_acquire().unwrap();
        assert!(slot.conn_and_reader().is_err());
    }

    #[tokio::test]
    async fn atomic_builder_validates_empty_url() {
        let result = AtomicClientPool::builder()
            .connections(1)
            .build()
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn atomic_builder_validates_zero_connections() {
        let result = AtomicClientPool::builder()
            .url("http://localhost")
            .connections(0)
            .build()
            .await;
        assert!(result.is_err());
    }
}
