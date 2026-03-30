# nexus-async-net

Async adapters for [nexus-net](../nexus-net). Tokio-compatible.

Same sans-IO primitives, same performance — just `.await` on socket I/O.

- **WebSocket** — `WsStream<S>` wrapping nexus-net's FrameReader/FrameWriter
- **REST HTTP/1.1** — `AsyncHttpConnection<S>` wrapping nexus-net's RequestWriter/ResponseReader

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
let resp = conn.send(&req, &mut reader).await?;
println!("{}", resp.body_str()?);
drop(resp);

// POST with body
let req = writer.post("/post")
    .header("Content-Type", "application/json")
    .body(br#"{"action":"buy"}"#)
    .finish()?;
let resp = conn.send(&req, &mut reader).await?;
```

The `RequestWriter` and `ResponseReader` are the same types used by
blocking `nexus-net`. The only difference is `.await` on the transport.

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
| 40B     | 19ns            | 103ns             | **5.4x** |
| 128B    | 24ns            | 114ns             | **4.8x** |
| 512B    | 49ns            | 143ns             | **2.9x** |

### vs tokio-tungstenite (JSON parse + sonic-rs deserialize)

| Payload | nexus-async-net | tokio-tungstenite | Speedup |
|---------|-----------------|-------------------|---------|
| 77B quote tick  | 149ns  | 245ns             | **1.6x** |
| 148B order update | 326ns | 424ns            | **1.3x** |
| 676B book snapshot | 1671ns | 1759ns          | **1.1x** |

### Three-way comparison: async vs blocking vs tokio-tungstenite

**TCP loopback (no TLS):**

| Payload | nexus-async-net | nexus-net (blocking) | tokio-tungstenite |
|---------|-----------------|---------------------|-------------------|
| 40B     | 21ns (49M/s)    | 24ns (41M/s)        | 104ns (9.6M/s)    |
| 128B    | 33ns (30M/s)    | 45ns (22M/s)        | 119ns (8.4M/s)    |

**TLS loopback:**

| Payload | nexus-async-net | nexus-net (blocking) | tokio-tungstenite |
|---------|-----------------|---------------------|-------------------|
| 40B     | 32ns (31M/s)    | 34ns (29M/s)        | 112ns (9.0M/s)    |
| 128B    | 80ns (13M/s)    | 78ns (13M/s)        | 183ns (5.5M/s)    |

**There is no meaningful tokio overhead.** The async path matches or beats blocking across all configurations — TCP and TLS. Both nexus paths are 3-5x faster than tokio-tungstenite.

Teams already on tokio should use nexus-async-net directly. There is no performance reason to avoid async.

## Builder

```rust
use nexus_async_net::ws::WsStreamBuilder;
use nexus_net::tls::TlsConfig;

let tls = TlsConfig::new()?;
let mut ws = WsStreamBuilder::new()
    .tls(&tls)
    .disable_nagle()
    .buffer_capacity(2 * 1024 * 1024)
    .connect("wss://exchange.com/ws")
    .await?;
```

## Features

- **Zero-copy WebSocket** — `Message<'_>` borrows from the internal buffer via `recv()`
- **Stream/Sink** — `OwnedMessage` for `StreamExt`/`SinkExt` ergonomics
- **Zero-alloc REST** — same `RequestWriter`/`ResponseReader` as blocking, just `.await` on I/O
- **Automatic TLS** — `wss://` and `https://` URLs handled transparently via tokio-rustls
- **Chunked transfer encoding** — decoded transparently for REST responses
- **Same sans-IO primitives** — identical parse path as blocking nexus-net
- **Single-threaded friendly** — works with `current_thread` runtime + `LocalSet`

## Dependencies

- `nexus-net` — sans-IO WebSocket + HTTP primitives
- `tokio` — async runtime (io-util, net, rt)
- `tokio-rustls` — async TLS
- `futures-core` / `futures-sink` — Stream + Sink traits
