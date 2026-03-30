# nexus-async-net

Async adapters for [nexus-net](../nexus-net). Tokio-compatible.

Same sans-IO primitives, same performance — just `.await` on socket I/O.

- **WebSocket** — `WsStream<S>` wrapping nexus-net's FrameReader/FrameWriter
- **REST HTTP/1.1** — `AsyncHttpConnection<S>` wrapping nexus-net's RequestWriter/ResponseReader
- **Client Pool** — `ClientPool` (single-threaded) and `AtomicClientPool` (thread-safe) for connection reuse with LIFO acquire, inline reconnect, and RAII guards

## Quick Start

```rust
use nexus_async_net::ws::WsStream;
use nexus_net::ws::Message;

let mut ws = WsStream::connect("wss://exchange.com/ws").await?;

ws.send_text("subscribe").await?;

while let Some(msg) = ws.recv().await? {
    match msg {
        Message::Text(s) => println!("{s}"),     // zero-copy — borrows from internal buffer
        Message::Binary(b) => process(b),        // zero-copy
        Message::Ping(p) => ws.send_pong(p).await?,
        Message::Close(_) => break,
        _ => {}
    }
}
```

### REST Client (async)

```rust
use nexus_net::rest::RequestWriter;
use nexus_net::http::ResponseReader;
use nexus_async_net::rest::AsyncHttpConnection;

// Same sans-IO primitives as blocking nexus-net
let mut writer = RequestWriter::new("httpbin.org")?;
writer.default_header("Accept", "application/json")?;
let mut reader = ResponseReader::new(32 * 1024).max_body_size(32 * 1024);

// Async transport — TLS auto-detected from URL scheme
let mut conn = AsyncHttpConnection::connect("https://httpbin.org").await?;

// GET with query params
let req = writer.get("/get")
    .query("symbol", "BTC-USD")
    .finish()?;
let resp = conn.send(req, &mut reader).await?;
println!("{}", resp.body_str()?);
drop(resp);

// POST with body
let req = writer.post("/post")
    .header("Content-Type", "application/json")
    .body(br#"{"action":"buy"}"#)
    .finish()?;
let resp = conn.send(req, &mut reader).await?;
```

The `RequestWriter` and `ResponseReader` are the same types used by
blocking `nexus-net`. The only difference is `.await` on the transport.

### REST Builder (connect timeout, TLS, socket options)

```rust
use std::time::Duration;
use nexus_async_net::rest::AsyncHttpConnectionBuilder;

let mut conn = AsyncHttpConnectionBuilder::new()
    .connect_timeout(Duration::from_secs(5))
    .disable_nagle()
    .connect("https://api.binance.com")
    .await?;
```

### Server-Side WebSocket (accept)

```rust
use nexus_async_net::ws::WsStream;
use tokio::net::TcpListener;

let listener = TcpListener::bind("127.0.0.1:8080").await?;
let (tcp, _addr) = listener.accept().await?;
let mut ws = WsStream::accept(tcp).await?;

while let Some(msg) = ws.recv().await? {
    // handle messages
}
```

### Client Pool (connection reuse)

```rust
use nexus_async_net::rest::ClientPool;

// Build pool — connects all slots at startup
let pool = ClientPool::builder()
    .url("https://api.binance.com")
    .base_path("/api/v3")
    .default_header("X-API-KEY", &key)?
    .default_header("Content-Type", "application/json")?
    .connections(4)
    .tls(&tls)           // requires "tls" feature (enabled by default)
    .disable_nagle()
    .build()
    .await?;

// Acquire slot — LIFO, auto-reconnects if connection died
let mut slot = pool.acquire().await?;

// Build request using the slot's writer
let req = slot.writer.post("/order")
    .header("X-Timestamp", &ts)
    .body(order_json)
    .finish()?;

// Send using the slot's connection + reader (split borrow)
let (conn, reader) = slot.conn_and_reader()?;
let resp = conn.send(req, reader).await?;
println!("{}", resp.body_str()?);

// drop(slot) — returns to pool. If poisoned, reconnects on next acquire.
```

Each slot owns a complete pipeline: `RequestWriter` + `ResponseReader` +
`AsyncHttpConnection`. No shared state between slots.

## Client Pool Performance

| Config | Throughput | Pool Overhead |
|--------|-----------|---------------|
| Single connection (no pool) | 255K req/sec | — |
| Pool (1 conn, sequential) | 248K req/sec | ~0% |
| Pool (4 conn, 4 concurrent tasks) | 279K req/sec | **+9.5%** throughput |
| Pool (8 conn, 8 concurrent tasks) | 289K req/sec | **+13.3%** throughput |

Pool acquire/release: **26 cycles** (local), **42 cycles** (atomic).

Measured on localhost TCP where the round-trip is ~9μs. On a real
network (1-10ms round-trip), the concurrency benefit is dramatically
larger — overlapping I/O wait across N connections gives close to Nx
throughput. The localhost benchmark is bottlenecked by the echo server,
not the client.

## Client Pool Design

### Two variants

**`ClientPool`** — single-threaded (`!Send`). Uses `Rc`-based
`nexus_pool::local::Pool`. For `current_thread` runtime + `LocalSet`.
26-cycle acquire/release. This is the primary variant for trading
systems where the hot path runs on a dedicated thread.

**`AtomicClientPool`** — thread-safe (`Send`). Uses atomic CAS-based
`nexus_pool::sync::Pool`. 42-cycle acquire/release. **Single acquirer,
any returner** — one task dispatches requests, guards can be dropped
from any thread.

The `AtomicClientPool` is designed for an architecture where a single
task owns the pool and dispatches requests. It is NOT a global pool
that arbitrary tasks acquire from concurrently — `sync::Pool` is
`Send` but not `Sync`. If you need shared acquire, wrap in a `Mutex`,
but consider whether a single dispatcher task is the better design.

### Failure model

1. **Request fails** — caller gets the error. No retry. The request is
   late (stale timestamp, wrong nonce). Caller decides: log, resubmit
   with fresh params, escalate to another venue.

2. **Connection dies** — slot is poisoned. On drop, the guard returns
   the slot to the pool and the reset closure clears the dead connection
   and response buffer. Next `acquire()` reconnects inline.

3. **Reconnect fails** — `acquire()` returns the error. Slot stays
   disconnected in the pool. Next `acquire()` tries again. When the
   server comes back, the first successful acquire recovers.

4. **All connections dead** — every `acquire()` attempts reconnect.
   Natural recovery when the server returns. No circuit breaker state
   machine — the reconnect-on-acquire pattern IS the recovery.

### Invariants

- The pool **never hands out a poisoned connection**. Every `acquire()`
  checks `needs_reconnect()` and reconnects inline if needed.
- The **reset closure clears stale state** — dead connections are
  dropped and the response reader buffer is reset on return.
- **Writer + reader survive reconnect** — only the transport is replaced.
  Host headers, default headers, base path, buffer capacity are preserved.
- **LIFO acquire** — the most recently used (warmest cache lines)
  connection is acquired first.
- Slots have **public fields** for split borrows through `Pooled<T>`'s
  `DerefMut`. Use `conn_and_reader()` for the common pattern, or
  `let s: &mut ClientSlot = &mut slot;` for direct field access.

## Two API Paths (WebSocket)

### Zero-copy `recv()` (recommended)

`recv()` returns `Message<'_>` borrowing directly from the internal buffer. No allocation per message. Use this for latency-sensitive code — trading systems, market data feeds, high-throughput pipelines.

```rust
while let Some(msg) = ws.recv().await? {
    match msg {
        Message::Text(s) => handle(s),  // s: &str, borrows from ReadBuf
        _ => {}
    }
}
```

### Stream/Sink (ergonomic)

`Stream<Item = Result<OwnedMessage, WsError>>` allocates per message but enables the full `StreamExt`/`SinkExt` combinator API. Use this when ergonomics matter more than nanoseconds.

```rust
use futures::StreamExt;
use nexus_net::ws::OwnedMessage;

while let Some(msg) = ws.next().await {
    match msg? {
        OwnedMessage::Text(s) => handle(&s),  // s: String, owned
        _ => {}
    }
}
```

## Performance

### vs tokio-tungstenite (in-memory parse, binary frames)

| Payload | nexus-async-net | tokio-tungstenite | Speedup |
|---------|-----------------|-------------------|---------|
| 40B     | 19ns (52M/s)    | 61ns (16M/s)      | **3.2x** |
| 128B    | 24ns (42M/s)    | 75ns (13M/s)      | **3.1x** |
| 512B    | 49ns (20M/s)    | 105ns (10M/s)     | **2.1x** |

### vs tokio-tungstenite (JSON parse + sonic-rs deserialize)

| Payload | nexus-async-net | tokio-tungstenite | Speedup |
|---------|-----------------|-------------------|---------|
| 77B quote tick  | 146ns (6.9M/s) | 205ns (4.9M/s) | **1.4x** |
| 148B order update | 331ns (3.0M/s) | 382ns (2.6M/s) | **1.2x** |
| 676B book snapshot | 1637ns (611K/s) | 1720ns (581K/s) | **1.1x** |

### Three-way comparison: async vs blocking vs tokio-tungstenite

**TCP loopback (no TLS, pinned to cores 0,2):**

| Payload | nexus-async-net | nexus-net (blocking) | tokio-tungstenite |
|---------|-----------------|---------------------|-------------------|
| 40B     | 21ns (49M/s)    | 30ns (33M/s)        | 66ns (15M/s)      |

**TLS loopback (pinned to cores 0,2):**

| Payload | nexus-async-net | nexus-net (blocking) | tokio-tungstenite |
|---------|-----------------|---------------------|-------------------|
| 40B     | 32ns (31M/s)    | 34ns (29M/s)        | 112ns (9.0M/s)    |
| 128B    | 80ns (13M/s)    | 78ns (13M/s)        | 183ns (5.5M/s)    |

**There is no meaningful tokio overhead.** The async path matches or beats blocking across all configurations — TCP and TLS. Both nexus paths are 2-3x faster than tungstenite.

Teams already on tokio should use nexus-async-net directly. There is no performance reason to avoid async.

## Builder

```rust
use std::time::Duration;
use nexus_async_net::ws::WsStreamBuilder;
use nexus_net::tls::TlsConfig;

let tls = TlsConfig::new()?;
let mut ws = WsStreamBuilder::new()
    .tls(&tls)                              // requires "tls" feature (default)
    .disable_nagle()
    .buffer_capacity(2 * 1024 * 1024)
    .connect_timeout(Duration::from_secs(5))
    .connect("wss://exchange.com/ws")
    .await?;
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `tls` | **Yes** | TLS support via tokio-rustls + aws-lc-rs. `wss://` and `https://` URLs auto-detected. |

Disable with `default-features = false` for TLS-free builds.

- **Zero-copy WebSocket** — `Message<'_>` borrows from the internal buffer via `recv()`
- **Stream/Sink** — `OwnedMessage` for `StreamExt`/`SinkExt` ergonomics
- **Zero-alloc REST** — same `RequestWriter`/`ResponseReader` as blocking, just `.await` on I/O
- **Automatic TLS** — `wss://` and `https://` URLs handled transparently via tokio-rustls
- **Connect timeout** — `WsStreamBuilder::connect_timeout()` and `AsyncHttpConnectionBuilder::connect_timeout()`
- **Server-side WebSocket** — `WsStream::accept(stream)` for incoming connections
- **Chunked transfer encoding** — decoded transparently for REST responses
- **Same sans-IO primitives** — identical parse path as blocking nexus-net
- **Single-threaded friendly** — works with `current_thread` runtime + `LocalSet`

## Dependencies

- `nexus-net` — sans-IO WebSocket + HTTP primitives
- `tokio` — async runtime (io-util, net, rt)
- `tokio-rustls` — async TLS
- `futures-core` / `futures-sink` — Stream + Sink traits
