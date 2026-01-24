# nexus-id

High-performance ID generators for low-latency systems.

All generators avoid syscalls on the hot path, produce stack-allocated output, and operate entirely in registers where possible. Hex encode/decode uses SIMD acceleration (SSE2/SSSE3) on x86_64.

## Generators

| Generator | p50 (cycles) | Time-ordered | Output |
|-----------|-------------|--------------|--------|
| `Snowflake64` | 22 | Yes | `SnowflakeId64` |
| `Snowflake32` | 22 | Yes | `SnowflakeId32` |
| `UuidV4` | 48 | No | `Uuid` |
| `UuidV7` | 62 | Yes | `Uuid` |
| `UlidGenerator` | 80 | Yes | `Ulid` |

## ID Types

| Type | Format | Parsing (p50) |
|------|--------|---------------|
| `SnowflakeId64` | Packed u64 | N/A |
| `MixedId64` | Fibonacci-mixed u64 | N/A |
| `Uuid` | `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` | 70 cycles |
| `UuidCompact` | 32-char hex | 48 cycles |
| `Ulid` | 26-char Crockford Base32 | 90 cycles |
| `HexId64` | 16-char hex | 42 cycles |
| `Base62Id` | 11-char alphanumeric | 86 cycles |
| `Base36Id` | 13-char alphanumeric | 108 cycles |
| `TypeId` | `prefix_suffix` | 136 cycles |

## Usage

### Snowflake IDs

```rust
use nexus_id::{Snowflake64, SnowflakeId64, MixedId64};

// <42 timestamp bits, 6 worker bits, 16 sequence bits>
let mut generator: Snowflake64<42, 6, 16> = Snowflake64::new(5);

let id: SnowflakeId64<42, 6, 16> = generator.next_id(current_tick).unwrap();
assert_eq!(id.worker(), 5);
assert_eq!(id.sequence(), 0);

// Fibonacci mixing for identity hashers (~1 cycle, bijective)
let mixed: MixedId64<42, 6, 16> = id.mixed();
let recovered = mixed.unmix();
assert_eq!(recovered, id);
```

### UUIDs

```rust
use nexus_id::{Uuid, UuidCompact};
use nexus_id::uuid::UuidV7;
use std::time::Instant;

let epoch = Instant::now();
let mut generator = UuidV7::new(epoch, 1_700_000_000_000, 42);

let uuid: Uuid = generator.next(Instant::now());
let compact: UuidCompact = uuid.to_compact();

// Parse from string
let parsed: Uuid = "01234567-89ab-cdef-fedc-ba9876543210".parse().unwrap();
```

### ULIDs

```rust
use nexus_id::Ulid;
use nexus_id::ulid::UlidGenerator;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

let epoch = Instant::now();
let unix_base = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap()
    .as_millis() as u64;

let mut generator = UlidGenerator::new(epoch, unix_base, 42);
let ulid: Ulid = generator.next(Instant::now());

assert_eq!(ulid.len(), 26);
assert!(ulid.timestamp_ms() >= unix_base);
```

### TypeIDs

```rust
use nexus_id::{TypeId, Ulid};

// Type-prefixed sortable IDs (prefix + ULID suffix)
let id: TypeId<32> = TypeId::new("user", ulid).unwrap();
assert!(id.as_str().starts_with("user_"));
assert_eq!(id.prefix(), "user");
```

### Parsing

All string ID types implement `FromStr`:

```rust
use nexus_id::{Uuid, UuidCompact, Ulid, HexId64, Base62Id, Base36Id};

let uuid: Uuid = "01234567-89ab-cdef-fedc-ba9876543210".parse().unwrap();
let compact: UuidCompact = "0123456789abcdeffedcba9876543210".parse().unwrap();
let hex: HexId64 = "deadbeefcafebabe".parse().unwrap();
```

## Features

| Feature | Description |
|---------|-------------|
| `std` (default) | UUID/ULID generators, `Error` impls, `from_entropy()` |
| `serde` | `Serialize`/`Deserialize` for all types |
| `uuid` | Interop with the [`uuid`](https://docs.rs/uuid) crate |
| `bytes` | `BufMut` writing via the [`bytes`](https://docs.rs/bytes) crate |

All ID types work in `no_std` (parsing, encoding, conversions). Generators require `std` for entropy.

## Performance

Measured on Intel Core Ultra 7 165U, single-core pinned. All operations are allocation-free and syscall-free on the hot path.

SIMD acceleration is compile-time dispatched:
- **SSE2** (x86_64 baseline): parallel hex decode
- **SSSE3** (Core 2+, 2006): `pshufb` hex encode + UUID dash compaction

Build with native features for best performance:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## `no_std` Support

```toml
[dependencies]
nexus-id = { version = "1", default-features = false }
```

All types, parsing, and encoding work without `std`. Only generators (which need entropy) require the `std` feature.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
