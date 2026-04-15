# nexus-net Documentation

Sans-IO WebSocket and HTTP/1.1 primitives with optional TLS. Zero-copy,
SIMD-accelerated, no async runtime, no sockets baked in.

## Contents

1. [overview.md](./overview.md) — What nexus-net is, why sans-IO, when to use it
2. [websocket.md](./websocket.md) — WebSocket framing, Client, FrameReader/FrameWriter
3. [http.md](./http.md) — HTTP/1.1 REST client, request/response builders, chunked transfer
4. [tls.md](./tls.md) — Rustls integration, certificate handling, ALPN
5. [buffers.md](./buffers.md) — ReadBuf and WriteBuf semantics
6. [errors.md](./errors.md) — Error types, connection poisoning, recovery
7. [patterns.md](./patterns.md) — Cookbook: exchange client, REST with retry, mio integration
8. [performance.md](./performance.md) — Benchmarks vs tungstenite/reqwest, SIMD details

## Quick pointers

- Connecting to an exchange WebSocket: [patterns.md — Exchange Client](./patterns.md#exchange-websocket-client)
- Parsing frames from a raw byte stream (mio, io_uring, kernel bypass): [websocket.md — Sans-IO Loop](./websocket.md#sans-io-parse-loop)
- REST keep-alive with a pooled connection: [patterns.md — REST client](./patterns.md#rest-client-with-retry)
- Async version of all of this: see the sibling crate
  [nexus-async-net](../../nexus-async-net/docs/INDEX.md)

## Source tour

```
src/
  buf/         ReadBuf / WriteBuf — byte buffers with prepend headroom
  ws/          WebSocket: FrameReader, FrameWriter, Client, handshake
  http/        HTTP/1.x: ResponseReader, ChunkedDecoder, writers
  rest/        REST Client + RequestWriter (typestate builder)
  tls/         Rustls codec + TlsStream (feature: tls)
```
