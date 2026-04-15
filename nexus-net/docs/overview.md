# Overview

nexus-net is a collection of **sans-IO** protocol primitives for
WebSocket (RFC 6455) and HTTP/1.1. "Sans-IO" means: the protocol state
machines operate on byte slices — they don't own sockets, don't call
`read`/`write`, don't block, and don't care what runtime you use.

On top of the sans-IO layer, nexus-net also ships a convenience
`Client` for each protocol that does own a socket (`TcpStream` or
`TlsStream<TcpStream>`). You can use the convenience layer when you
just need a working blocking client, and drop down to the sans-IO
layer when you're plugging into mio, io_uring, or a custom event
loop.

## Why sans-IO

Most WebSocket / HTTP libraries bake in an IO model (tokio, async-std,
blocking threads). That coupling prevents you from using them in:

- mio-based event loops (nexus-rt, custom reactors)
- io_uring runtimes
- Kernel bypass (DPDK, ef_vi, Solarflare Onload)
- Testing without a network (pure protocol replay)

A sans-IO codec separates **what bytes go on the wire** from **how
they get there**. The `FrameReader` takes bytes you've already read
from somewhere and yields `Message`s. The `FrameWriter` encodes
`Message`s into a byte buffer you can later hand to any writer.

```text
your_transport.read()  -->  FrameReader::read()  -->  Message<'_>
your_transport.write() <--  FrameWriter::encode_*_into()  --  WriteBuf
```

The wrappers (`ws::Client`, `rest::Client`) are a thin shim that owns
a `TcpStream` and drives the codec for you.

## What you get

**WebSocket (`ws`):**
- `FrameReader` — RFC 6455 parser with zero-copy `Message<'_>`. SIMD
  masking (SSE2/AVX2), SIMD UTF-8 validation (simdutf8), fragmentation
  reassembly, ping/pong/close handling, size limits, role-aware masking.
- `FrameWriter` — single-frame encoder for text/binary/ping/pong/close.
  ChaCha8-based masking for client role.
- `Client<S>` — high-level blocking client; `connect()`, `recv()`,
  `send_text()`, `send_binary()`, `send_ping()`, `close()`.
- `handshake` — HTTP Upgrade handshake producer and parser.

**HTTP/1.1 (`http` + `rest`):**
- `ResponseReader` — httparse-backed parser for inbound responses.
- `ChunkedDecoder` — streaming chunked transfer decoder.
- `RequestWriter` — typestate builder (`get`/`post`/...) → `Request<'_>`.
- `rest::Client<S>` — blocking REST client with keep-alive, `send()`.

**Buffers (`buf`):**
- `ReadBuf` — flat inbound buffer with pre/post padding, zero-copy
  parsing surface, compaction support.
- `WriteBuf` — outbound buffer with **prepend headroom** so frame
  headers can be written in-place after the payload is known.

**TLS (`tls`, feature-gated):**
- `TlsConfig` / `TlsConfigBuilder` — rustls configuration wrapper.
- `TlsCodec` — sans-IO encrypt/decrypt adapter around rustls.
- `TlsStream<S>` — blocking adapter implementing `Read + Write`.

## When to use nexus-net vs nexus-async-net

| Situation | Crate |
|-----------|-------|
| Blocking thread-per-connection (trading hot thread) | **nexus-net** |
| mio / io_uring / custom event loop | **nexus-net** (sans-IO layer) |
| Kernel bypass, DPDK, ef_vi | **nexus-net** (sans-IO layer) |
| Tokio ecosystem (Stream/Sink, axum, reqwest alternative) | **nexus-async-net** |
| Single-threaded tokio current_thread + LocalSet | **nexus-async-net** |

Both crates share the same `FrameReader`/`FrameWriter`/`RequestWriter`
under the hood — nexus-async-net is just a thin `.await` wrapper.

## Design principles

- **No allocation on the hot path.** Buffers are sized at construction.
  `recv()` returns `Message<'_>` that borrows from `ReadBuf`.
- **Zero-copy where the wire format allows.** Single-frame unmasked
  text becomes a `&str` pointing directly into the ReadBuf.
- **Sans-IO first, convenience second.** `Client` is a convenience;
  the primitives are the product.
- **Honest failure modes.** IO errors mid-frame poison the connection.
  The caller decides whether to reconnect.
