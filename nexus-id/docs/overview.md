# Overview

`nexus-id` is a grab bag of ID generators, each tuned for a specific use case.
Pick the cheapest one that satisfies your constraints.

## Decision table

| Need | Type | Why |
|------|------|-----|
| Numeric ID with timestamp + worker + sequence | [`Snowflake64`](snowflake.md) | ~22 cy, const-generic bit layout |
| Compact numeric ID (fits in u32) | `Snowflake32` | ~22 cy, smaller state |
| Random 128-bit unique ID | `UuidV4` | ~48 cy, no time leak |
| Time-sortable UUID | `UuidV7` | ~62 cy, RFC 9562, k-sortable |
| Sortable 26-char string ID | `Ulid` | ~80 cy, monotonic within ms |
| Hash-map-friendly integer ID | [`MixedId64`](mixed-id.md) | ~1 cy wrap over `Snowflake*` |
| Hex-encoded u64 for logs/URLs | `HexId64` | SIMD hex encode/decode |
| URL-safe short string ID | `Base62Id` | 11 chars, alphanumeric |
| Case-insensitive string ID | `Base36Id` | 13 chars |
| Human-friendly typed ID (`ord_01HKX...`) | `TypeId` | prefixed, sortable |

## Performance characteristics

All numbers are approximate p50 cycles on a modern x86 core. Nothing here
allocates or calls the kernel on the hot path:

```text
MixedId64::mix          ~1 cy    (Fibonacci multiply)
Snowflake64::next_id   ~22 cy    (increment + pack)
UuidV4::generate       ~48 cy    (128 bits of PCG)
UuidV7::generate       ~62 cy    (timestamp + random tail)
UlidGenerator::next    ~80 cy    (timestamp + monotonic random)
```

SIMD hex encode/decode (SSE2 decode, SSSE3 encode) keeps `HexId64::parse` and
`HexId64::to_string` fast enough to use on wire-protocol hot paths.

## `no_std` support

Core types (`SnowflakeId64`, `Uuid`, `Ulid`, `HexId64`, etc.) and their parsers
are `no_std`. The *generators* (`UuidV4`, `UuidV7`, `UlidGenerator`) require
the `std` feature because they pull entropy and time from the OS.

```toml
[dependencies]
nexus-id = { version = "1", default-features = false }          # no_std
nexus-id = { version = "1", features = ["serde", "bytes"] }     # with std
```

## What nexus-id does *not* do

- No UUIDv1/v6 (MAC-based, not needed in modern systems).
- No cryptographic randomness. `UuidV4` uses PCG, which is fine for
  collision resistance but not for secrets. Use `rand::rngs::OsRng` or
  similar if you need unpredictability.
- No distributed coordination. Worker IDs in Snowflake are your problem —
  usually a config value or a leased token from ZooKeeper/etcd.
