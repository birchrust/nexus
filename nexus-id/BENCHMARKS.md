# nexus-id Performance Benchmarks

CPU cycles measured via `rdtscp` on x86_64. Pinned to a single core via
`taskset -c 0` for stable measurements.

**System:** AMD Ryzen 9 7950X. 1M iterations, 10k warmup.

Run benchmarks yourself:
```bash
cargo build --release -p nexus-id --examples
taskset -c 0 ./target/release/examples/perf_benchmark
taskset -c 0 ./target/release/examples/perf_snowflake
taskset -c 0 ./target/release/examples/perf_uuid
taskset -c 0 ./target/release/examples/perf_id_hashing
```

---

## ID Generation

### Snowflake

| Operation | p50 | p99 | p999 | Notes |
|-----------|-----|-----|------|-------|
| `next()` new timestamp | 20 | 24 | 26 | Sequence reset path |
| `next()` same timestamp | 22 | 24 | 28 | Sequence increment (burst) |
| `next()` realistic trading | 22-28 | 24-34 | 28-44 | Bursts of 50, mixed ts |

All layouts (`<42,6,16>`, `<41,10,12>`, `<20,4,8>`) perform identically at p50.

### UUID

| Operation | p50 | p99 | p999 | Notes |
|-----------|-----|-----|------|-------|
| `UuidV4::next_raw()` | 22 | 24 | 66 | Returns `(u64, u64)` |
| `UuidV4::next()` | 58 | 96 | 138 | Returns `Uuid` (36-char) |
| `UuidV4::next_compact()` | 48 | 74 | 136 | Returns `UuidCompact` (32-char) |
| `UuidV7::next_raw()` same ts | 30 | 36 | 68 | Monotonic sequence path |
| `UuidV7::next()` same ts | 68 | 164 | 170 | Returns `Uuid` |
| `UuidV7::next()` new ts | 70 | 94 | 134 | Timestamp advanced |

### ULID

| Operation | p50 | p99 | p999 | Notes |
|-----------|-----|-----|------|-------|
| `next()` same timestamp | 82 | 114 | 146 | Monotonic increment |
| `next()` new timestamp | 82 | 168 | 176 | Timestamp advanced |

ULID is slower than UUID due to Crockford Base32 encoding (26 chars, 5-bit
groups) vs hex encoding (32/36 chars, 4-bit groups with lookup table).

---

## Newtype Operations

| Operation | p50 | p99 | p999 | Notes |
|-----------|-----|-----|------|-------|
| `mixed()` | 20 | 22 | 24 | Fibonacci multiply (~1 cycle, measurement floor) |
| `unmix()` | 20 | 24 | 28 | Inverse multiply |
| `unpack()` | 20 | 36 | 40 | 3 shifts + masks |

These are at the `rdtscp` measurement floor (~20 cycles). The actual operation
cost is 1-3 cycles; the rest is measurement overhead.

---

## String Encoding

| Operation | p50 | p99 | p999 | Notes |
|-----------|-----|-----|------|-------|
| `HexId64::encode(u64)` | 34 | 40 | 80 | Lookup table, 16 chars |
| `SnowflakeId64::to_hex()` | 20 | 40 | 48 | Same as above (inlined) |
| `Base62Id::encode(u64)` | 64 | 82 | 146 | Digit-pair decomposition |
| `SnowflakeId64::to_base62()` | 44 | 76 | 106 | Same as above (inlined) |
| `Base36Id::encode(u64)` | 68 | 82 | 158 | Digit-pair decomposition |

Hex encoding uses a 256-byte lookup table (byte → 2 hex chars). Base62/36 use
a digit-pair decomposition that reduces division count (5 divmod ops for base62,
6 for base36).

---

## String Parsing

| Operation | p50 | p99 | p999 | Notes |
|-----------|-----|-----|------|-------|
| `HexId64::parse()` | 92 | 134 | 282 | 16-char hex validation |
| `Base62Id::parse()` | 86 | 106 | 178 | 11-char with multiply-accumulate |
| `Base36Id::parse()` | 104 | 310 | 328 | 13-char |
| `UuidCompact::parse()` | 86 | 142 | 270 | 32-char hex |
| `Ulid::parse()` | 90 | 208 | 242 | 26-char Crockford (256-byte LUT) |
| `Uuid::parse()` | 102 | 170 | 266 | 36-char with dash validation |
| `TypeId::parse()` | 138 | 168 | 342 | Prefix + ULID suffix |

All parsing is single-pass: validate and decode simultaneously. No allocation.

---

## TypeId

| Operation | p50 | p99 | p999 | Notes |
|-----------|-----|-----|------|-------|
| `TypeId::new("user", ulid)` | 54 | 70 | 424 | Construct from prefix + ULID |
| `TypeId::parse("user_...")` | 138 | 168 | 342 | Full string parse |
| `TypeId::prefix()` | 22 | 24 | 28 | Slice into stored string |

---

## Combined Operations

| Operation | p50 | p99 | p999 | Notes |
|-----------|-----|-----|------|-------|
| `next_id() + to_hex()` | 34 | 38 | 48 | Generate + hex format |
| `next_id() + to_base62()` | 66 | 134 | 144 | Generate + base62 format |
| `next_id() + mixed()` | 22 | 32 | 36 | Generate + hash-ready |
| `TypeId::new()` | 54 | 70 | 424 | Generate + TypeId format |

The common hot-path pattern — generate a snowflake and mix it for HashMap
lookup — costs 22 cycles (p50). This is the cost of two multiplies.

---

## HashMap Performance

Demonstrates why bit distribution matters for hash table performance.

**Setup:** 100k IDs inserted, 1M random lookups measured.

### Lookup Latency (cycles)

| ID Pattern | Identity | FxHash | AHash |
|------------|----------|--------|-------|
| Snowflake (sequential bits) | 3535 | 52 | 60 |
| Sequential u64 | 30 | 64 | 60 |

Snowflake IDs with identity hashers are **catastrophic** — 3535 cycles/lookup
due to clustering in power-of-2 bucket tables. Use either:
1. A real hasher (FxHash, AHash) — 52-64 cycles
2. `MixedId64` with identity hasher — distributes bits uniformly

### Insert Throughput (cycles/insert)

| ID Pattern | Identity | FxHash |
|------------|----------|--------|
| Snowflake (sequential bits) | 3438 | 16 |
| Sequential u64 | 16 | 14 |

---

## Cost Summary

| What you're doing | p50 cycles | Recommendation |
|-------------------|-----------|----------------|
| Generate a numeric ID | 20-22 | `Snowflake64::next_id()` |
| Generate + hash-ready | 22 | `next_id() + mixed()` |
| Generate + hex string | 34 | `next_id() + to_hex()` |
| Generate a UUID v4 | 58 | `UuidV4::next()` |
| Generate a UUID v7 | 68-70 | `UuidV7::next()` |
| Generate a ULID | 82 | `UlidGenerator::next()` |
| Parse a UUID string | 102 | `Uuid::parse()` |
| Parse a ULID string | 90 | `Ulid::parse()` |
| Mix/unmix for hashing | 20 | At measurement floor |

All operations are allocation-free, stack-only, and syscall-free.
