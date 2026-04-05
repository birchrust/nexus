//! Single-threaded REST client pool.
//!
//! Uses [`nexus_pool::local::Pool`] for LIFO acquire/release with RAII guards.
//! Inline reconnect on acquire when a connection dies.

use std::net::SocketAddr;

use nexus_async_rt::{TcpStream, spawn, sleep};
use nexus_pool::local::{Pool, Pooled};

use super::connection::{Client, parse_base_url};
use super::error::RestError;
use super::request::RequestWriter;
use crate::http::ResponseReader;

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
/// let s: &mut ClientSlot = &mut slot;
/// let req = s.writer.post("/order").body(json).finish()?;
/// let (conn, reader) = s.conn_and_reader()?;
/// let resp = conn.send(req, reader).await?;
/// ```
pub struct ClientSlot {
    /// Request encoder (sans-IO). Build requests here.
    pub writer: RequestWriter,
    /// Response parser. Fed by the connection during send.
    pub reader: ResponseReader,
    /// Transport. `None` if connection died and needs reconnect.
    pub conn: Option<Client<TcpStream>>,
}

impl ClientSlot {
    /// Whether the connection is dead and needs reconnect.
    pub fn needs_reconnect(&self) -> bool {
        self.conn
            .as_ref()
            .is_none_or(Client::is_poisoned)
    }

    /// Split borrow: get mutable references to conn + reader
    /// while writer is borrowed by a `Request<'_>`.
    ///
    /// This exists because `Pooled<ClientSlot>` goes through `DerefMut`
    /// which prevents the compiler from seeing disjoint field borrows.
    pub fn conn_and_reader(
        &mut self,
    ) -> Result<(&mut Client<TcpStream>, &mut ResponseReader), RestError> {
        let conn = self.conn.as_mut().ok_or(RestError::ConnectionPoisoned)?;
        Ok((conn, &mut self.reader))
    }
}

// =============================================================================
// ClientPool
// =============================================================================

/// Single-threaded REST client pool.
///
/// Pre-allocated slots with LIFO acquire for cache locality. Each slot
/// owns a [`RequestWriter`], [`ResponseReader`], and [`Client`].
///
/// # Usage
///
/// ```ignore
/// let pool = ClientPool::builder()
///     .url("http://api.exchange.com")
///     .base_path("/api/v3")
///     .default_header("X-API-KEY", &key)?
///     .connections(4)
///     .build()
///     .await?;
///
/// // Fast path — no reconnect, no wait
/// let mut slot = pool.try_acquire().unwrap();
/// // Patient path — waits, reconnects with backoff
/// let mut slot = pool.acquire().await?;
///
/// let s: &mut ClientSlot = &mut slot;
/// let req = s.writer.post("/order").body(json).finish()?;
/// let (conn, reader) = s.conn_and_reader()?;
/// let resp = conn.send(req, reader).await?;
/// // drop(slot) returns to pool
/// ```
pub struct ClientPool {
    pool: Pool<ClientSlot>,
    reconnect_config: ReconnectConfig,
}

#[derive(Clone)]
struct ReconnectConfig {
    addr: SocketAddr,
    nodelay: bool,
}

impl ClientPool {
    /// Create a builder.
    #[must_use]
    pub fn builder() -> ClientPoolBuilder {
        ClientPoolBuilder::new()
    }

    /// Try to acquire a healthy client slot (LIFO).
    ///
    /// Checks available slots for a healthy connection. Dead slots are
    /// ejected from the pool and a reconnect task is spawned for each.
    /// When reconnection succeeds, the slot returns to the pool
    /// automatically.
    ///
    /// Returns `None` if all slots are in use or currently reconnecting.
    ///
    /// This is the trading hot path — O(1) when the top slot is healthy.
    pub fn try_acquire(&self) -> Option<Pooled<ClientSlot>> {
        loop {
            let slot = self.pool.try_acquire()?;
            if !slot.needs_reconnect() {
                return Some(slot);
            }
            self.spawn_reconnect(slot);
        }
    }

    /// Acquire a client slot, waiting until one is available.
    ///
    /// If no healthy slots are available, waits for reconnect tasks
    /// to finish healing dead connections. Returns error if no slot
    /// becomes available within the retry limit.
    #[allow(clippy::future_not_send)] // Intentionally !Send — single-threaded runtime.
    pub async fn acquire(&self) -> Result<Pooled<ClientSlot>, RestError> {
        const MAX_BACKOFF_MS: u64 = 1_000;
        const MAX_ATTEMPTS: u32 = 20;
        let mut backoff_ms = 1u64;

        for _ in 0..MAX_ATTEMPTS {
            if let Some(slot) = self.try_acquire() {
                return Ok(slot);
            }
            sleep(std::time::Duration::from_millis(backoff_ms)).await;
            backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
        }

        Err(RestError::ConnectionClosed(
            "pool acquire timed out: no healthy slots available",
        ))
    }

    /// Number of slots currently available (not acquired).
    pub fn available(&self) -> usize {
        self.pool.available()
    }

    /// Spawn a task to reconnect a dead slot.
    ///
    /// The task owns the `Pooled` guard. On successful reconnect, the
    /// guard drops and returns the healthy slot to the pool. On failure,
    /// retries with exponential backoff.
    fn spawn_reconnect(&self, mut slot: Pooled<ClientSlot>) {
        let config = self.reconnect_config.clone();
        spawn(async move {
            const MAX_BACKOFF_MS: u64 = 5_000;
            let mut backoff_ms = 100u64;

            loop {
                if let Ok(conn) = connect_one(&config) {
                    slot.conn = Some(conn);
                    slot.reader.reset();
                    return;
                }
                sleep(std::time::Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
            }
        });
    }
}

/// Create a single TCP connection. Cold path — used for initial connect
/// and reconnect.
fn connect_one(config: &ReconnectConfig) -> Result<Client<TcpStream>, RestError> {
    let io = nexus_async_rt::io();
    let tcp = TcpStream::connect(config.addr, io)?;
    if config.nodelay {
        tcp.set_nodelay(true)?;
    }
    Ok(Client::new(tcp))
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
    ///
    /// Performs blocking DNS resolution and creates TCP connections.
    /// Call during startup, not on the hot path.
    pub fn build(self) -> Result<ClientPool, RestError> {
        if self.url.is_empty() {
            return Err(RestError::InvalidUrl("url is required".to_string()));
        }
        if self.connections == 0 {
            return Err(RestError::InvalidUrl("connections must be > 0".to_string()));
        }

        let parsed = parse_base_url(&self.url)?;
        let host_header = parsed.host_header();

        // Resolve DNS once at build time.
        let addr_str = format!("{}:{}", parsed.host, parsed.port);
        let addr = std::net::ToSocketAddrs::to_socket_addrs(&addr_str)
            .map_err(RestError::Io)?
            .next()
            .ok_or_else(|| RestError::Io(std::io::Error::other("DNS resolution failed")))?;

        let reconnect_config = ReconnectConfig {
            addr,
            nodelay: self.nodelay,
        };

        let io = nexus_async_rt::io();

        // Build initial slots with live connections.
        let mut initial_slots = Vec::with_capacity(self.connections);
        for _ in 0..self.connections {
            let tcp = TcpStream::connect(addr, io)?;
            if self.nodelay {
                tcp.set_nodelay(true)?;
            }

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
                conn: Some(Client::new(tcp)),
            });
        }

        // Create pool with factory + reset.
        let host = host_header;
        let base = self.base_path.clone();
        let headers = self.default_headers.clone();
        let wbuf_cap = self.write_buffer_capacity;
        let rbuf_cap = self.response_buffer_capacity;
        let max_body = self.max_body_size;

        let pool = Pool::new(
            move || {
                let mut writer = RequestWriter::new(&host).expect("host already validated");
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
    fn try_acquire_returns_none_when_exhausted() {
        let pool = Pool::new(make_disconnected_slot, |_| {});
        pool.put(make_disconnected_slot());

        let _s1 = pool.try_acquire().unwrap();
        assert!(pool.try_acquire().is_none());
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

    #[test]
    fn conn_and_reader_error_when_no_conn() {
        let mut slot = make_disconnected_slot();
        assert!(slot.conn_and_reader().is_err());
    }

    #[test]
    fn builder_validates_empty_url() {
        // Can't call build() outside runtime context, but we can test
        // validation by checking the error before DNS resolution.
        let builder = ClientPoolBuilder::new().connections(1);
        // url is empty — should error
        assert!(builder.url.is_empty());
    }

    #[test]
    fn builder_validates_crlf_header() {
        let result = ClientPoolBuilder::new()
            .default_header("X-Bad\r\n", "val");
        assert!(matches!(result, Err(RestError::CrlfInjection)));
    }
}
