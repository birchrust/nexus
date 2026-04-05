# nexus-net

Low-latency network protocol primitives. Sans-IO. Zero-copy where possible.
Framework-agnostic — works with mio, io_uring, tokio, or raw syscalls.

## Performance

### vs tungstenite (in-memory parse, pinned to cores 0,2)

| Payload | Type | nexus-net | tungstenite | Speedup |
|---------|------|-----------|-------------|---------|
| 40B | binary parse | 19ns (52M/s) | 61ns (16M/s) | **3.2x** |
| 128B | binary parse | 24ns (42M/s) | 75ns (13M/s) | **3.1x** |
| 512B | binary parse | 49ns (20M/s) | 105ns (10M/s) | **2.1x** |
| 77B | JSON quote parse+deser | 146ns (6.9M/s) | 205ns (4.9M/s) | **1.4x** |
| 148B | JSON order parse+deser | 331ns (3.0M/s) | 382ns (2.6M/s) | **1.2x** |
| 40B | binary TCP loopback | 30ns (33M/s) | 66ns (15M/s) | **2.2x** |
| 77B | JSON TLS+parse+deser | 165ns (6.1M/s) | 221ns (4.5M/s) | **1.3x** |

JSON deserialization uses sonic-rs. At the quote tick hot path (77B),
WS framing is 18% of nexus-net's total vs 43% of tungstenite's.

### rdtsc cycle distribution (pinned to core 0, batch=64)

| Path | p50 | p90 | p99 | p99.9 |
|------|-----|-----|-----|-------|
| text unmasked 128B | 39 | 39 | 43 | 65 |
| binary unmasked 128B | 35 | 36 | 44 | 129 |
| text masked 128B | 52 | 53 | 58 | 124 |
| apply_mask 128B | 12 | 12 | 16 | 31 |
| encode_text 128B server | 10 | 11 | 22 | 39 |
| throughput 100×128B /msg | 28 | 28 | 44 | 91 |

At 3GHz: 39 cycles = 13ns. In-memory throughput: 107M msg/sec (28 cycles/msg).
TCP loopback throughput: 33M msg/sec (30ns/msg, 40B binary, pinned cores 0,2).
The gap is kernel TCP overhead — protocol parsing is ~13ns of the 30ns round-trip.

### TLS loopback (all three: async, blocking, tokio-tungstenite)

| Payload | nexus-async-net | nexus-net (blocking) | tokio-tungstenite |
|---------|-----------------|---------------------|-------------------|
| 40B | 32ns (31M/s) | 34ns (29M/s) | 112ns (9.0M/s) |
| 128B | 80ns (13M/s) | 78ns (13M/s) | 183ns (5.5M/s) |

3.5x faster than tokio-tungstenite over TLS. No meaningful async overhead —
the async path matches blocking. See [nexus-async-net](../nexus-async-net)
for the tokio adapter.

517/517 Autobahn conformance tests passed (0 failed, 216 unimplemented
compression — intentionally unsupported).

### vs reqwest (REST HTTP/1.1 client)

| Benchmark | nexus-net | reqwest | Speedup |
|-----------|-----------|---------|---------|
| POST build+write+parse (mock) p50 | 494 cycles (165ns) | 1,549 cycles (516ns) build-only | **3.1x** |
| POST build+write+parse (mock) p99 | 763 cycles (254ns) | 2,445 cycles (815ns) build-only | **3.2x** |
| Protocol throughput (mock, single-threaded) | 5.3M req/sec | N/A | — |
| TCP loopback round-trip p50 | 22,924 cycles (7.6μs) | 62,802 cycles (20.9μs) | **2.7x** |
| TCP loopback round-trip p99.9 | 59,860 cycles (20.0μs) | 198,120 cycles (66.0μs) | **3.3x** |
| TCP loopback throughput | 114K req/sec | 39K req/sec | **2.9x** |

All measurements pinned to physical P-cores (taskset -c 0 for mock, -c 0,2 for
loopback). Workload: Binance-style order entry — POST + 4 headers + JSON body
(~100B) + 200 OK JSON response. Zero per-request allocation. nexus-net measures
full round-trip; reqwest measures build-only (no write/parse).

16/16 httpbin.org conformance tests passed (GET, POST, PUT, DELETE, PATCH, query
params, custom headers, keep-alive, status codes, chunked transfer encoding).

## Architecture

```
Application
    |
    ├── WebSocket                    REST HTTP/1.1
    |   ^ Message<'a>               ^ Request<'a> / RestResponse<'a>
    |   FrameReader / FrameWriter   RequestWriter / ResponseReader    (sans-IO)
    |   ^ plaintext bytes           ^ plaintext bytes
    |   └──────────┬────────────────┘
    |              TlsCodec                     (optional, feature-gated)
    |              ^ encrypted bytes
    └──────────────┘
                   I/O                          (your choice)
```

Each layer is a pure state machine. No syscalls, no sockets, no async.
Bytes in, messages out. The I/O layer is yours — mio, io_uring, tokio,
raw `libc::read`, kernel bypass.

## Async Runtimes

nexus-net supports two async runtimes via mutually exclusive feature flags.
Without either flag, you get the blocking sync API.

```toml
[dependencies]
# Blocking sync API (default)
nexus-net = { version = "0.3", features = ["tls"] }

# nexus-async-rt — single-threaded, zero-alloc dispatch, 58 cy p50
# Best for dedicated trading threads where every microsecond matters.
nexus-net = { version = "0.3", features = ["nexus-rt", "tls"] }

# tokio — multi-threaded, ecosystem compatibility
# Best when integrating with existing tokio services.
nexus-net = { version = "0.3", features = ["tokio", "tls"] }
```

Both runtimes expose the same API — same method names, same types:

```rust
// This code works with either runtime. The feature flag selects the impl.
let mut ws = ws::Client::connect_with(tcp, "ws://exchange.com/ws").await?;
ws.send_text(r#"{"subscribe":"trades"}"#).await?;
while let Some(msg) = ws.recv().await? {
    handle(msg);
}
```

| | nexus-async-rt | tokio |
|---|---|---|
| Threading | Single-threaded | Multi-threaded or current_thread |
| Dispatch p50 | 58 cycles | 146 cycles |
| Task alloc | Slab (pre-allocated) | Box (heap) |
| Waker | Zero-alloc (raw pointer) | Arc-based |
| IO driver | mio (direct) | mio (through tokio) |
| Ecosystem | Standalone | Full tokio ecosystem |
| Use case | Hot-path trading | Everything else |

The codec is identical — same `FrameReader`, same `FrameWriter`, same zero-copy
`Message<'_>`. The runtime only affects how `.await` is scheduled and how IO
events are dispatched.

### nexus-async-rt + ClientPool

The `nexus-rt` feature also enables `rest::ClientPool` — a pre-allocated
connection pool with LIFO acquire, bounded reconnect, and RAII guards:

```rust
let pool = rest::ClientPool::builder()
    .url("https://api.exchange.com")
    .base_path("/api/v3")
    .connections(4)
    .build()
    .await?;

let mut slot = pool.try_acquire().unwrap();
let req = slot.writer.post("/order").body(json).finish()?;
let (conn, reader) = slot.conn_and_reader()?;
let resp = conn.send(req, reader).await?;
```

## Quick Start

```toml
[dependencies]
# WebSocket + HTTP, no TLS
nexus-net = "0.3"

# With TLS (rustls + aws-lc-rs)
nexus-net = { version = "0.3", features = ["tls"] }

# Everything (TLS + socket options + bytes)
nexus-net = { version = "0.3", features = ["full"] }
```

### WebSocket Client (ws://)

```rust
use nexus_net::ws::{Client, Message, CloseCode};

let mut ws = Client::builder().connect("ws://exchange.com:80/ws/v1")?;

ws.send_text(r#"{"subscribe":"trades.BTC-USD"}"#)?;

loop {
    match ws.recv()? {
        Some(Message::Text(json)) => process(json),     // &str, zero-copy
        Some(Message::Binary(data)) => process(data),   // &[u8], zero-copy
        Some(Message::Ping(p)) => ws.send_pong(p)?,
        Some(Message::Close(frame)) => {
            ws.close(CloseCode::Normal, "")?;
            break;
        }
        Some(Message::Pong(_)) => {}
        None => break,
    }
}
```

### WebSocket Client (wss://)

```rust
use nexus_net::ws::Client;
use nexus_net::tls::TlsConfig;

// TLS detected from wss:// scheme — create TlsConfig once at startup
let tls = TlsConfig::new()?;
let mut ws = Client::builder().tls(&tls).connect("wss://exchange.com/ws/v1")?;

// Same API — recv(), send_text(), send_binary(), etc.
```

Or with custom TLS config:

```rust
use nexus_net::ws::Client;
use nexus_net::tls::TlsConfig;

let tls = TlsConfig::builder().tls13_only().build()?;
let mut ws = Client::builder()
    .tls(&tls)
    .disable_nagle()
    .connect("wss://exchange.com/ws/v1")?;
```

### REST Client (HTTP/1.1, blocking)

```rust
use nexus_net::rest::{Client, RequestWriter};
use nexus_net::http::ResponseReader;

// Protocol (sans-IO) — configured once at startup
let mut writer = RequestWriter::new("httpbin.org")?;
writer.default_header("Accept", "application/json")?;

// Response reader — caller-owned, reused across requests
let mut reader = ResponseReader::new(32 * 1024).max_body_size(32 * 1024);

// Transport — TLS config created once, builder for connection
let tls = nexus_net::tls::TlsConfig::new()?;
let mut conn = Client::builder().tls(&tls).connect("https://httpbin.org")?;

// GET with query parameters
let req = writer.get("/get")
    .query("symbol", "BTC-USD")
    .query("limit", "100")
    .finish()?;
let resp = conn.send(req, &mut reader)?;
println!("status: {}", resp.status());
println!("body: {}", resp.body_str()?);
drop(resp);  // release reader borrow before next request

// POST with JSON body
let json = br#"{"symbol":"BTC-USD","side":"buy"}"#;
let req = writer.post("/post")
    .header("Content-Type", "application/json")
    .body(json)
    .finish()?;
let resp = conn.send(req, &mut reader)?;
println!("rate limit: {:?}", resp.header("X-RateLimit-Remaining"));

// POST with body_writer (serialize directly into wire buffer)
let req = writer.post("/order")
    .header("Content-Type", "application/json")
    .body_writer(|w| serde_json::to_writer(w, &order))
    .finish()?;
let resp = conn.send(req, &mut reader)?;

// POST with body_fixed (known-size binary, zero-copy write)
let req = writer.post("/order")
    .body_fixed(128, |buf| {
        order.encode_sbe(buf);
    })
    .finish()?;
let resp = conn.send(req, &mut reader)?;
```

Three objects, clear ownership:
- **`writer`** — protocol encoder (sans-IO). Build request, get wire bytes.
- **`reader`** — protocol decoder (sans-IO). Feed bytes, parse response.
- **`conn`** — transport. Send bytes, receive bytes. No protocol knowledge.

The same `writer` and `reader` work with both sync and async transports.

### Sans-IO (decoupled from sockets)

```rust
use nexus_net::ws::{self, Message, Role};

let (mut reader, writer) = ws::pair(Role::Client);

// You own the I/O — feed bytes however you want
reader.read_from(&mut socket)?;

// Drain messages with poll limit
for _ in 0..8 {
    match reader.next()? {
        Some(Message::Text(s)) => handle(s),
        Some(Message::Ping(p)) => {
            let mut dst = [0u8; 131];
            let n = writer.encode_pong(p, &mut dst)?;
            socket.write_all(&dst[..n])?;
        }
        None => break,
        _ => {}
    }
}
```

### Send Path (borrow, don't own)

```rust
let order = serialize_order(&order);   // you own this
ws.send_text(&order)?;                 // we borrow
archive.write(&order)?;               // still yours — archive after send
```

## Modules

### `buf` — Buffer Primitives

- **`ReadBuf`** — flat byte slab for inbound parsing. Pre/post padding.
  Pointer advancement, auto-reset when empty.
- **`WriteBuf`** — headroom buffer for outbound framing. Payload appended,
  protocol headers prepended. One contiguous slice for the syscall.

### `ws` — WebSocket (RFC 6455)

- **`FrameReader`** — sans-IO inbound parser. Handles frame parsing,
  fragment assembly, control frame interleaving, SIMD masking, UTF-8
  validation. Returns `Message<'a>` (zero-copy borrowed) or `OwnedMessage`.
- **`FrameWriter`** — sans-IO outbound encoder. Encodes into `&mut [u8]`
  or `WriteBuf`.
- **`Client<S>`** — convenience I/O wrapper over any `Read + Write`.
  HTTP upgrade handshake built in.
- **`Client<S>` with TLS** — `wss://` URLs enable TLS transparently.
  Requires `tls` feature.
- **`Message<'a>`** — `Text(&str)`, `Binary(&[u8])`, `Ping(&[u8])`,
  `Pong(&[u8])`, `Close(CloseFrame)`. Text is validated UTF-8. Close
  codes are parsed into `CloseCode` enum.

### `rest` — HTTP/1.1 REST Client

- **`RequestWriter`** — sans-IO request encoder with typestate builder.
  Produces `Request<'a>` (zero-copy borrow of wire bytes). Supports
  query params (percent-encoded), per-request headers, `body()` (slice),
  `body_writer()` (serialize directly via `std::io::Write`),
  `body_fixed()` (known-size direct write), base path.
- **`Client<S>`** — pure transport. `send(req, &mut reader)` is the
  whole API. TLS handled at the stream level via `TlsStream<S>`.
- **`RestResponse<'a>`** — borrows from `ResponseReader`. Status, headers,
  body. Supports Content-Length and chunked transfer encoding.

### `http` — HTTP/1.1 Primitives

- **`RequestReader`** / **`ResponseReader`** — sans-IO HTTP parsers
  backed by `httparse` (SIMD-accelerated). Zero-copy header access.
  Cached Content-Length and Transfer-Encoding from parse.
- **`ChunkedDecoder`** — sans-IO chunked transfer encoding decoder.
- **`write_request`** / **`write_response`** — zero-alloc HTTP construction.

### `tls` — TLS (feature: `tls`)

- **`TlsConfig`** — shared config (`Arc<ClientConfig>`). System root
  certs, custom certs, `danger_no_verify()`, TLS 1.3 only.
- **`TlsCodec`** — sans-IO decrypt/encrypt wrapping rustls.
  `process_into(&mut FrameReader)` feeds decrypted plaintext directly
  into the WS parser.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `tls` | No | TLS support via rustls + aws-lc-rs |
| `nexus-rt` | No | Async API via nexus-async-rt (single-threaded, low-latency) |
| `tokio` | No | Async API via tokio |
| `socket-opts` | No | Socket options (SO_RCVBUF, SO_SNDBUF) via socket2 |
| `bytes` | No | `bytes::Bytes` conversion on `OwnedMessage` and `RestResponse` |
| `full` | No | `tls` + `socket-opts` + `bytes` |

`nexus-rt` and `tokio` are mutually exclusive — enabling both is a compile error.
Without either, you get the blocking sync API. The `tls` feature is orthogonal
and works with any mode.

## Design Decisions

**Zero-copy inbound.** `Message::Text(&str)` borrows from the reader's
internal buffer. No heap allocation per message. Drop the message,
call `recv()` again.

**Borrow, don't own.** Send APIs take `&str` / `&[u8]`. You keep
ownership for archival after send. Works across `.await` points.

**Sans-IO.** Protocol logic is a pure state machine. The same
`FrameReader` works with blocking sockets, mio, io_uring, tokio, or
kernel bypass. No runtime coupling.

**SIMD-accelerated.** XOR masking uses SSE2/AVX2. UTF-8 validation
uses `simdutf8`. HTTP header parsing uses `httparse` (SIMD vectorized).

**No permessage-deflate.** WebSocket compression adds latency and
no crypto exchange uses it. Exchanges that compress use
application-level gzip (e.g., OKX sends gzipped binary frames).

**Layered, not coupled.** `ReadBuf` → `FrameReader` → `Client` are
independent layers. Use any combination. `TlsCodec` slots between
socket and `FrameReader` without changing either.

## Testing

```bash
cargo test -p nexus-net                          # sync (200 tests)
cargo test -p nexus-net --features nexus-rt      # nexus-async-rt (180 tests)
cargo test -p nexus-net --features tokio         # tokio (174 tests)
cargo test -p nexus-net --features tls           # sync + TLS

# WebSocket: Autobahn conformance (requires Podman)
podman run --rm -d --network=host \
    -v "${PWD}/nexus-net/tests/autobahn:/config:Z" \
    -v "${PWD}/target/autobahn-reports:/reports:Z" \
    docker.io/crossbario/autobahn-testsuite \
    wstest -m fuzzingserver -s /config/fuzzingserver.json
cargo test -p nexus-net --test autobahn -- --ignored --nocapture

# REST: httpbin.org conformance (requires network)
cargo test -p nexus-net --all-features --test httpbin -- --ignored --test-threads=1

# wss:// echo test (requires network)
cargo test -p nexus-net --features tls --test wss_echo -- --ignored --nocapture

# Fuzzing (requires nightly)
cargo +nightly fuzz run fuzz_response_reader -- -max_total_time=60
cargo +nightly fuzz run fuzz_request_writer -- -max_total_time=60
cargo +nightly fuzz run fuzz_keepalive_sequence -- -max_total_time=60

# Benchmarks (sans-IO)
cargo run --release -p nexus-net --example perf_ws
cargo run --release -p nexus-net --example perf_vs_tungstenite
cargo run --release -p nexus-net --features tls --example perf_tls
cargo run --release -p nexus-net --example perf_rest

# Runtime comparison (nexus-rt vs tokio vs competition)
cargo run --release -p nexus-net --features nexus-rt --example perf_ws_compare
cargo run --release -p nexus-net --features nexus-rt --example perf_rest_compare
```
