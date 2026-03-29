# nexus-net

Low-latency network protocol primitives. Sans-IO. Zero-copy where possible.
Framework-agnostic — works with mio, io_uring, tokio, or raw syscalls.

## Performance

**5-9x faster than tungstenite** for WebSocket frame parsing.
**4.6x faster** including TLS decrypt (same rustls backend).

| Path | nexus-net | tungstenite | Speedup |
|------|----------|-------------|---------|
| ws:// text 128B parse | 45 cycles | 497 cycles | 11x |
| ws:// binary 128B parse | 41 cycles | 467 cycles | 11x |
| ws:// throughput (batch) | 36 cycles/msg | 182 cycles/msg | 5x |
| wss:// decrypt+parse 128B | 723 cycles | 3,336 cycles | 4.6x |

At 3GHz: 45 cycles = **15ns per message**. 36 cycles batched = **~83M msg/sec**.

517/517 Autobahn conformance tests passed (0 failed, 216 unimplemented
compression — intentionally unsupported).

## Architecture

```
Application
    ^ Message<'a> / OwnedMessage
FrameReader / FrameWriter       (ws, sans-IO)
    ^ plaintext bytes
TlsCodec                        (optional, feature-gated)
    ^ encrypted bytes
I/O                              (your choice)
```

Each layer is a pure state machine. No syscalls, no sockets, no async.
Bytes in, messages out. The I/O layer is yours — mio, io_uring, tokio,
raw `libc::read`, kernel bypass.

## Quick Start

```toml
[dependencies]
nexus-net = "0.1"                      # ws:// only
nexus-net = { version = "0.1", features = ["full"] }  # everything including TLS
nexus-net = { version = "0.1", features = ["tls"] }   # ws:// + wss://
```

### WebSocket Client (ws://)

```rust
use std::net::TcpStream;
use nexus_net::ws::{WsStream, Message, CloseCode};

let tcp = TcpStream::connect("exchange.com:80")?;
let mut ws = WsStream::connect(tcp, "ws://exchange.com/ws/v1")?;

ws.send_text(r#"{"subscribe":"trades.BTC-USD"}"#)?;

loop {
    match ws.next()? {
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
use std::net::TcpStream;
use nexus_net::tls::TlsConfig;
use nexus_net::ws::{WsTlsStream, Message};

let tls = TlsConfig::new()?;  // system root certs, TLS 1.2+1.3
let tcp = TcpStream::connect("exchange.com:443")?;
let mut ws = WsTlsStream::connect(tcp, &tls, "wss://exchange.com/ws/v1")?;

// Same API from here — next(), send_text(), send_binary(), etc.
```

### Sans-IO (decoupled from sockets)

```rust
use nexus_net::ws::{FrameReader, FrameWriter, Message, Role};

let mut reader = FrameReader::builder()
    .role(Role::Client)
    .buffer_capacity(256 * 1024)
    .build();
let writer = FrameWriter::new(Role::Client);

// You own the I/O — feed bytes however you want
reader.read_from(&mut socket)?;

// Drain messages with poll limit
for _ in 0..8 {
    match reader.next()? {
        Some(Message::Text(s)) => handle(s),
        Some(Message::Ping(p)) => {
            let mut dst = [0u8; 131];
            let n = writer.encode_pong(p, &mut dst);
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
- **`WsTlsStream<S>`** — same as `WsStream` but with TLS. Requires
  `tls` feature.
- **`Message<'a>`** — `Text(&str)`, `Binary(&[u8])`, `Ping(&[u8])`,
  `Pong(&[u8])`, `Close(CloseFrame)`. Text is validated UTF-8. Close
  codes are parsed into `CloseCode` enum.

### `http` — HTTP/1.1 (minimal)

- **`RequestReader`** / **`ResponseReader`** — sans-IO HTTP parsers
  backed by `httparse` (SIMD-accelerated). Zero-copy header access.
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
| `full` | No | All features enabled |

Without `tls`: zero TLS-related compile time. The `ws` and `http`
modules work standalone.

## Design Decisions

**Zero-copy inbound.** `Message::Text(&str)` borrows from the reader's
internal buffer. No heap allocation per message. Drop the message,
call `next()` again.

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
cargo test -p nexus-net                          # unit tests (143)
cargo test -p nexus-net --all-features           # with TLS

# Autobahn conformance (requires Podman)
podman run --rm -d --network=host \
    -v "${PWD}/nexus-net/tests/autobahn:/config:Z" \
    -v "${PWD}/target/autobahn-reports:/reports:Z" \
    docker.io/crossbario/autobahn-testsuite \
    wstest -m fuzzingserver -s /config/fuzzingserver.json
cargo test -p nexus-net --test autobahn -- --ignored --nocapture

# wss:// echo test (requires network)
cargo test -p nexus-net --features tls --test wss_echo -- --ignored --nocapture

# Benchmarks
cargo run --release -p nexus-net --example perf_ws
cargo run --release -p nexus-net --example perf_vs_tungstenite
cargo run --release -p nexus-net --features tls --example perf_tls
```
