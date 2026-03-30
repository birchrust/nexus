# nexus-net

Low-latency network protocol primitives. Sans-IO. Zero-copy where possible.
Framework-agnostic — works with mio, io_uring, tokio, or raw syscalls.

## Performance

### vs tungstenite (blocking, in-memory parse)

| Payload | Type | nexus-net | tungstenite | Speedup |
|---------|------|-----------|-------------|---------|
| 40B | binary parse | 18ns | 61ns | **3.4x** |
| 128B | binary parse | 23ns | 76ns | **3.3x** |
| 512B | binary parse | 49ns | 106ns | **2.2x** |
| 77B | JSON quote parse+deser | 142ns | 204ns | **1.4x** |
| 148B | JSON order parse+deser | 326ns | 374ns | **1.1x** |
| 40B | binary TCP loopback | 25ns | 62ns | **2.5x** |
| 77B | JSON TLS+parse+deser | 179ns | 244ns | **1.4x** |

JSON deserialization uses sonic-rs. At the quote tick hot path (77B),
WS framing is 18% of nexus-net's total vs 43% of tungstenite's.

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
| POST build+write+parse (mock) p50 | 471 cycles (157ns) | 1,484 cycles (495ns) build-only | **3.1x** |
| POST build+write+parse (mock) p99 | 745 cycles (248ns) | 2,035 cycles (678ns) build-only | **2.7x** |
| Protocol throughput (mock, single-threaded) | 5.05M req/sec | N/A | — |
| TCP loopback round-trip p50 | 24,822 cycles (8.3μs) | 64,334 cycles (21.4μs) | **2.6x** |
| TCP loopback round-trip p99.9 | 97,204 cycles (32.4μs) | 215,146 cycles (71.7μs) | **2.2x** |
| TCP loopback throughput | 108,108 req/sec | 37,798 req/sec | **2.9x** |

Workload: Binance-style order entry — POST + 4 headers + JSON body (~100B) + 200 OK
JSON response. Zero per-request allocation. nexus-net measures full round-trip;
reqwest measures build-only (no write/parse).

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

## Quick Start

```toml
[dependencies]
# WebSocket + HTTP, no TLS
nexus-net = "0.2"

# With TLS (rustls + aws-lc-rs)
nexus-net = { version = "0.2", features = ["tls"] }

# Everything (TLS + socket options + bytes)
nexus-net = { version = "0.2", features = ["full"] }
```

### WebSocket Client (ws://)

```rust
use nexus_net::ws::{WsStream, Message, CloseCode};

let mut ws = WsStream::connect("ws://exchange.com:80/ws/v1")?;

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
use nexus_net::ws::WsStream;

// TLS detected from wss:// scheme — automatic with system root certs
let mut ws = WsStream::connect("wss://exchange.com/ws/v1")?;

// Same API — recv(), send_text(), send_binary(), etc.
```

Or with custom TLS config:

```rust
use nexus_net::ws::WsStream;
use nexus_net::tls::TlsConfig;

let tls = TlsConfig::builder().tls13_only().build()?;
let mut ws = WsStream::builder()
    .tls(&tls)
    .disable_nagle()
    .connect("wss://exchange.com/ws/v1")?;
```

### REST Client (HTTP/1.1, blocking)

```rust
use nexus_net::rest::{HttpConnection, RequestWriter};
use nexus_net::http::ResponseReader;

// Protocol (sans-IO) — configured once at startup
let mut writer = RequestWriter::new("httpbin.org")?;
writer.default_header("Accept", "application/json")?;

// Response reader — caller-owned, reused across requests
let mut reader = ResponseReader::new(32 * 1024).max_body_size(32 * 1024);

// Transport — just a socket (TLS auto-detected from URL scheme)
let mut conn = HttpConnection::connect("https://httpbin.org")?;

// GET with query parameters
let req = writer.get("/get")
    .query("symbol", "BTC-USD")
    .query("limit", "100")
    .finish()?;
let resp = conn.send(&req, &mut reader)?;
println!("status: {}", resp.status());
println!("body: {}", resp.body_str()?);
drop(resp);  // release reader borrow before next request

// POST with JSON body
let json = br#"{"symbol":"BTC-USD","side":"buy"}"#;
let req = writer.post("/post")
    .header("Content-Type", "application/json")
    .body(json)
    .finish()?;
let resp = conn.send(&req, &mut reader)?;
println!("rate limit: {:?}", resp.header("X-RateLimit-Remaining"));
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
- **`WsStream<S>`** — convenience I/O wrapper over any `Read + Write`.
  HTTP upgrade handshake built in.
- **`WsStream<S>` with TLS** — `wss://` URLs enable TLS transparently.
  Requires `tls` feature.
- **`Message<'a>`** — `Text(&str)`, `Binary(&[u8])`, `Ping(&[u8])`,
  `Pong(&[u8])`, `Close(CloseFrame)`. Text is validated UTF-8. Close
  codes are parsed into `CloseCode` enum.

### `rest` — HTTP/1.1 REST Client

- **`RequestWriter`** — sans-IO request encoder with typestate builder.
  Produces `Request<'a>` (zero-copy borrow of wire bytes). Supports
  query params (percent-encoded), per-request headers, body, base path.
- **`HttpConnection<S>`** — pure transport. 3 fields: stream, TLS,
  poisoned. `send(&req, &mut reader)` is the whole API.
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
| `socket-opts` | No | Socket options (SO_RCVBUF, SO_SNDBUF) via socket2 |
| `bytes` | No | `bytes::Bytes` conversion on `OwnedMessage` and `RestResponse` |
| `full` | No | All features enabled |

Without features: zero TLS compile time. The `ws` and `http`
modules work standalone.

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

**Layered, not coupled.** `ReadBuf` → `FrameReader` → `WsStream` are
independent layers. Use any combination. `TlsCodec` slots between
socket and `FrameReader` without changing either.

## Testing

```bash
cargo test -p nexus-net                          # unit tests (190)
cargo test -p nexus-net --all-features           # with TLS

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

# Benchmarks
cargo run --release -p nexus-net --example perf_ws
cargo run --release -p nexus-net --example perf_vs_tungstenite
cargo run --release -p nexus-net --features tls --example perf_tls
cargo run --release -p nexus-net --example perf_rest
cargo run --release -p nexus-net --example throughput_rest
cargo run --release -p nexus-net --example bench_loopback_rest
```
