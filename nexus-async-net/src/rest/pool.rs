//! Single-threaded async HTTP client pool.
//!
//! Uses [`nexus_pool::local::Pool`] for LIFO acquire/release with RAII guards.
//! Inline reconnect on acquire when a connection dies.
//!
//! For `current_thread` runtime + `LocalSet`.

use nexus_net::http::ResponseReader;
use nexus_net::rest::{RequestWriter, RestError};
use nexus_net::tls::TlsConfig;
use nexus_pool::local::{Pool, Pooled};

use super::connection::{AsyncHttpConnection, AsyncHttpConnectionBuilder};
use crate::maybe_tls::MaybeTls;

// =============================================================================
// ClientSlot — the item stored in the pool
// =============================================================================

/// A complete request/response pipeline: writer + reader + transport.
///
/// Each slot in the pool owns its own set of protocol primitives.
/// Acquired via [`ClientPool::acquire`], auto-returned on drop.
///
/// Fields are public for split borrows through `Pooled<T>`'s `DerefMut`.
/// Deref explicitly to split, then build + send:
///
/// ```ignore
/// let s: &mut ClientSlot = &mut slot;  // explicit deref, enables split borrows
/// let req = s.writer.post("/order").body(json).finish()?;
/// let (conn, reader) = (s.conn.as_mut().unwrap(), &mut s.reader);
///
/// // With timeout (recommended for production):
/// let resp = tokio::time::timeout(timeout, conn.send(req, reader)).await??;
///
/// // Without timeout (prototyping only):
/// let resp = conn.send(req, reader).await?;
/// ```
pub struct ClientSlot {
    /// Request encoder (sans-IO). Build requests here.
    pub writer: RequestWriter,
    /// Response parser. Fed by the connection during send.
    pub reader: ResponseReader,
    /// Transport. `None` if connection died and needs reconnect.
    pub conn: Option<AsyncHttpConnection<MaybeTls>>,
}

impl ClientSlot {
    /// Whether the connection is dead and needs reconnect.
    pub fn needs_reconnect(&self) -> bool {
        self.conn
            .as_ref()
            .is_none_or(AsyncHttpConnection::is_poisoned)
    }

    /// Split borrow: get mutable references to conn + reader
    /// while writer is borrowed by a `Request<'_>`.
    ///
    /// This exists because `Pooled<ClientSlot>` goes through `DerefMut`
    /// which prevents the compiler from seeing disjoint field borrows.
    pub fn conn_and_reader(
        &mut self,
    ) -> Result<(&mut AsyncHttpConnection<MaybeTls>, &mut ResponseReader), RestError> {
        let conn = self.conn.as_mut().ok_or(RestError::ConnectionPoisoned)?;
        Ok((conn, &mut self.reader))
    }

}

// =============================================================================
// ClientPool
// =============================================================================

/// Single-threaded async HTTP client pool.
///
/// Pre-allocated slots with LIFO acquire for cache locality. Each slot
/// owns a [`RequestWriter`], [`ResponseReader`], and
/// [`AsyncHttpConnection`].
///
/// # Usage
///
/// ```ignore
/// let pool = ClientPool::builder()
///     .url("https://api.binance.com")
///     .base_path("/api/v3")
///     .default_header("X-API-KEY", &key)?
///     .connections(4)
///     .tls(&tls)
///     .build()
///     .await?;
///
/// let mut slot = pool.try_acquire().await?.unwrap();
/// let req = slot.writer().post("/order").body(json).finish()?;
/// let (conn, reader) = (s.conn.as_mut().unwrap(), &mut s.reader);
/// let resp = conn.send(req, reader).await?;
/// // drop(slot) returns to pool
/// ```
pub struct ClientPool {
    pool: Pool<ClientSlot>,
    reconnect_config: ReconnectConfig,
}

#[derive(Clone)]
struct ReconnectConfig {
    url: String,
    tls_config: Option<TlsConfig>,
    nodelay: bool,
}

#[allow(clippy::future_not_send)] // Intentionally !Send — single-threaded pool for LocalSet.
impl ClientPool {
    /// Create a builder.
    #[must_use]
    pub fn builder() -> ClientPoolBuilder {
        ClientPoolBuilder::new()
    }

    /// Acquire a client slot (LIFO).
    ///
    /// Returns `None` if all slots are currently in use.
    /// If the acquired slot has a dead connection, reconnects inline.
    ///
    /// The pool is bounded — it will never create connections beyond
    /// the configured `connections` count.
    pub async fn try_acquire(&self) -> Result<Option<Pooled<ClientSlot>>, RestError> {
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

    /// Number of slots currently available (not acquired).
    pub fn available(&self) -> usize {
        self.pool.available()
    }

    async fn reconnect(&self, slot: &mut ClientSlot) -> Result<(), RestError> {
        let conn = self.connect_one().await?;
        slot.conn = Some(conn);
        Ok(())
    }

    async fn connect_one(&self) -> Result<AsyncHttpConnection<MaybeTls>, RestError> {
        let mut builder = AsyncHttpConnectionBuilder::new();
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

/// Builder for [`ClientPool`].
pub struct ClientPoolBuilder {
    url: String,
    base_path: String,
    default_headers: Vec<(String, String)>,
    connections: usize,
    tls_config: Option<TlsConfig>,
    nodelay: bool,
    write_buffer_capacity: usize,
    response_buffer_capacity: usize,
    max_body_size: usize,
}

impl ClientPoolBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            url: String::new(),
            base_path: String::new(),
            default_headers: Vec::new(),
            connections: 1,
            tls_config: None,
            nodelay: false,
            write_buffer_capacity: 32 * 1024,
            response_buffer_capacity: 32 * 1024,
            max_body_size: 0,
        }
    }

    /// Target URL (scheme + host + optional port + optional path).
    #[must_use]
    pub fn url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    /// Base path prefix for all requests.
    #[must_use]
    pub fn base_path(mut self, path: &str) -> Self {
        self.base_path = path.to_string();
        self
    }

    /// Add a default header sent with every request.
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

    /// Number of pre-allocated connections. Default: 1.
    #[must_use]
    pub fn connections(mut self, n: usize) -> Self {
        self.connections = n;
        self
    }

    /// Custom TLS configuration.
    #[must_use]
    pub fn tls(mut self, config: &TlsConfig) -> Self {
        self.tls_config = Some(config.clone());
        self
    }

    /// Disable Nagle's algorithm on each connection.
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
    pub async fn build(self) -> Result<ClientPool, RestError> {
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
            tls_config: self.tls_config.clone(),
            nodelay: self.nodelay,
        };

        // Connect all slots sequentially (cold path — startup only).
        let mut initial_slots = Vec::with_capacity(self.connections);
        for _ in 0..self.connections {
            let mut builder = AsyncHttpConnectionBuilder::new();
            if let Some(ref tls) = self.tls_config {
                builder = builder.tls(tls);
            }
            if self.nodelay {
                builder = builder.disable_nagle();
            }
            let conn = builder.connect(&self.url).await?;

            let mut writer = RequestWriter::new(&host_header)?;
            if !self.base_path.is_empty() {
                writer.set_base_path(&self.base_path)?;
            }
            writer.set_write_buffer_capacity(self.write_buffer_capacity);
            for (name, value) in &self.default_headers {
                writer.default_header(name, value)?;
            }

            let reader = ResponseReader::new(self.response_buffer_capacity)
                .max_body_size(self.max_body_size);

            initial_slots.push(ClientSlot {
                writer,
                reader,
                conn: Some(conn),
            });
        }

        // Create pool with factory + reset.
        let host = host_header.clone();
        let base = self.base_path.clone();
        let headers = self.default_headers.clone();
        let wbuf_cap = self.write_buffer_capacity;
        let rbuf_cap = self.response_buffer_capacity;
        let max_body = self.max_body_size;

        let pool = Pool::new(
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
                ClientSlot {
                    writer,
                    reader: ResponseReader::new(rbuf_cap).max_body_size(max_body),
                    conn: None,
                }
            },
            |slot: &mut ClientSlot| {
                if slot.needs_reconnect() {
                    slot.conn = None;
                    // Clear stale response data so the next request
                    // after reconnect starts with a clean buffer.
                    slot.reader.reset();
                }
            },
        );

        // Pre-populate with connected slots.
        for slot in initial_slots {
            pool.put(slot);
        }

        Ok(ClientPool {
            pool,
            reconnect_config,
        })
    }
}

impl Default for ClientPoolBuilder {
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

    fn make_disconnected_slot() -> ClientSlot {
        ClientSlot {
            writer: RequestWriter::new("host").unwrap(),
            reader: ResponseReader::new(4096),
            conn: None,
        }
    }

    #[test]
    fn slot_needs_reconnect_when_no_conn() {
        let slot = make_disconnected_slot();
        assert!(slot.needs_reconnect());
    }

    #[test]
    fn pool_acquire_release_cycle() {
        let pool = Pool::new(make_disconnected_slot, |_| {});
        pool.put(make_disconnected_slot());

        assert_eq!(pool.available(), 1);

        let slot = pool.acquire();
        assert_eq!(pool.available(), 0);

        drop(slot);
        assert_eq!(pool.available(), 1);
    }

    #[test]
    fn pool_acquire_returns_available() {
        let pool: Pool<ClientSlot> = Pool::new(make_disconnected_slot, |_| {});
        pool.put(make_disconnected_slot());
        pool.put(make_disconnected_slot());

        assert_eq!(pool.available(), 2);
        let _s1 = pool.acquire();
        assert_eq!(pool.available(), 1);
        let _s2 = pool.acquire();
        assert_eq!(pool.available(), 0);
    }

    #[test]
    fn pool_reset_clears_dead_conn() {
        let pool = Pool::new(make_disconnected_slot, |slot| {
            if slot.needs_reconnect() {
                slot.conn = None;
            }
        });
        pool.put(make_disconnected_slot());

        let slot = pool.acquire();
        assert!(slot.conn.is_none());
        assert!(slot.needs_reconnect());
        drop(slot);

        // After return + reset, slot is still disconnected.
        let slot = pool.acquire();
        assert!(slot.conn.is_none());
    }

    #[test]
    fn pool_multiple_slots() {
        let pool = Pool::new(make_disconnected_slot, |_| {});
        for _ in 0..4 {
            pool.put(make_disconnected_slot());
        }
        assert_eq!(pool.available(), 4);

        let s1 = pool.acquire();
        let s2 = pool.acquire();
        assert_eq!(pool.available(), 2);

        drop(s1);
        assert_eq!(pool.available(), 3);
        drop(s2);
        assert_eq!(pool.available(), 4);
    }

    // Integration test with real TCP is in tests/httpbin.rs (ignored, needs network).
    // The loopback test below uses tokio to verify the full send path.
    #[tokio::test(flavor = "current_thread")]
    async fn pool_loopback_send() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Server: read request, send canned response.
        tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = tcp.read(&mut buf).await.unwrap();
            let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"orderId\":123}";
            tcp.write_all(resp).await.unwrap();
        });

        // Client: connect, wrap in MaybeTls, create slot, send.
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let stream = MaybeTls::Plain(tcp);
        let conn = AsyncHttpConnection::new(stream);

        let mut slot = ClientSlot {
            writer: RequestWriter::new(&addr.to_string()).unwrap(),
            reader: ResponseReader::new(4096),
            conn: Some(conn),
        };

        let req = slot.writer.get("/test").finish().unwrap();
        let conn = slot.conn.as_mut().unwrap();
        let resp = conn.send(req, &mut slot.reader).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.body_str().unwrap(), r#"{"orderId":123}"#);
    }

    #[test]
    fn try_acquire_returns_none_when_exhausted() {
        let pool = Pool::new(make_disconnected_slot, |_| {});
        pool.put(make_disconnected_slot());

        let _s1 = pool.try_acquire().unwrap();
        assert!(pool.try_acquire().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pool_keep_alive_multiple_requests() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Server: handle two sequential requests on the same connection.
        tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];

            // Request 1
            let _ = tcp.read(&mut buf).await.unwrap();
            tcp.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\n{\"r\":1}")
                .await
                .unwrap();

            // Request 2 — same TCP connection
            let _ = tcp.read(&mut buf).await.unwrap();
            tcp.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\n{\"r\":2}")
                .await
                .unwrap();
        });

        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let stream = MaybeTls::Plain(tcp);
        let conn = AsyncHttpConnection::new(stream);

        let pool = Pool::new(make_disconnected_slot, |slot| {
            if slot.needs_reconnect() {
                slot.conn = None;
                slot.reader.reset();
            }
        });
        pool.put(ClientSlot {
            writer: RequestWriter::new(&addr.to_string()).unwrap(),
            reader: ResponseReader::new(4096),
            conn: Some(conn),
        });

        // First request
        {
            let mut slot = pool.acquire();
            let s: &mut ClientSlot = &mut slot;
            let req = s.writer.get("/first").finish().unwrap();
            let conn = s.conn.as_mut().unwrap();
            let resp = conn.send(req, &mut s.reader).await.unwrap();
            assert_eq!(resp.body_str().unwrap(), r#"{"r":1}"#);
        } // slot returned

        // Second request — same slot, same connection (keep-alive)
        {
            let mut slot = pool.acquire();
            let s: &mut ClientSlot = &mut slot;
            let req = s.writer.get("/second").finish().unwrap();
            let conn = s.conn.as_mut().unwrap();
            let resp = conn.send(req, &mut s.reader).await.unwrap();
            assert_eq!(resp.body_str().unwrap(), r#"{"r":2}"#);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn builder_validates_empty_url() {
        let result = ClientPool::builder()
            .connections(1)
            .build()
            .await;
        assert!(result.is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn builder_validates_zero_connections() {
        let result = ClientPool::builder()
            .url("http://localhost")
            .connections(0)
            .build()
            .await;
        assert!(result.is_err());
    }

    /// Reproduces the reqwest connection pool bug: server closes the
    /// connection while idle, client writes into the dead socket,
    /// read hangs forever. Our timeout prevents the hang.
    #[tokio::test(flavor = "current_thread")]
    async fn stale_connection_timeout_not_hang() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Server: accept, respond to first request, then close the connection.
        tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];

            // First request — respond normally.
            let _ = tcp.read(&mut buf).await.unwrap();
            tcp.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await
                .unwrap();

            // Close the connection — simulates server idle timeout.
            drop(tcp);
        });

        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let stream = MaybeTls::Plain(tcp);
        let conn = AsyncHttpConnection::new(stream);

        let pool = Pool::new(
            make_disconnected_slot,
            |slot| {
                if slot.needs_reconnect() {
                    slot.conn = None;
                    slot.reader.reset();
                }
            },
        );
        pool.put(ClientSlot {
            writer: RequestWriter::new(&addr.to_string()).unwrap(),
            reader: ResponseReader::new(4096),
            conn: Some(conn),
        });

        // First request — succeeds.
        {
            let mut slot = pool.acquire();
            let s: &mut ClientSlot = &mut slot;
            let req = s.writer.get("/first").finish().unwrap();
            let conn = s.conn.as_mut().unwrap();
            let resp = conn.send(req, &mut s.reader).await.unwrap();
            assert_eq!(resp.status(), 200);
        }

        // Small delay to let server close.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Second request — server has closed. Without timeout this hangs.
        // With tokio::time::timeout, it fails fast.
        {
            let mut slot = pool.acquire();
            let s: &mut ClientSlot = &mut slot;
            let req = s.writer.get("/second").finish().unwrap();
            let conn = s.conn.as_mut().unwrap();

            let timeout = std::time::Duration::from_millis(500);
            let result = tokio::time::timeout(timeout, conn.send(req, &mut s.reader)).await;

            match result {
                Ok(Ok(_)) => panic!("stale connection should not succeed"),
                Ok(Err(_)) => {} // Connection error — correct (ConnectionClosed)
                Err(_elapsed) => {
                    // Timeout — also correct. This is what reqwest hangs on.
                    // We don't hang — we fail.
                }
            }

            // The connection should be dead.
            assert!(
                s.needs_reconnect(),
                "slot should be poisoned after stale connection"
            );
        }
    }
}
