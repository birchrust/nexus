# nexus-ascii

Fixed-capacity ASCII strings for high-performance systems.

## Overview

`nexus-ascii` provides stack-allocated, fixed-capacity ASCII string types optimized for trading systems and other latency-sensitive applications. Every string stores a precomputed hash in its header, enabling fast equality checks and optimal HashMap performance.

## Key Features

- **Zero allocation after construction** - Fixed capacity, stack allocated
- **Precomputed hash** - 48-bit XXH3 hash stored in header, computed once at construction
- **Fast equality** - Single 64-bit header comparison rejects most non-equal strings
- **Copy semantics** - All types implement `Copy` for zero-cost moves
- **Immutable** - Strings cannot be modified after creation, guaranteeing hash validity
- **nohash-hasher support** - Ideal for identity hashing in HashMaps (feature-gated)

## Types

| Type | Description |
|------|-------------|
| `AsciiString<N>` | Fixed-capacity ASCII string (bytes 0x00-0x7F) |
| `AsciiText<N>` | Printable ASCII only (bytes 0x20-0x7E) |
| `AsciiStr` | Borrowed reference to ASCII data (DST) |
| `AsciiTextStr` | Borrowed reference to printable ASCII data (DST) |
| `AsciiChar` | Single ASCII character with classification methods |
| `AsciiStringBuilder<N>` | Mutable builder for constructing strings |

## Usage

```rust
use nexus_ascii::{AsciiString, AsciiError};

// Construction
let symbol: AsciiString<32> = AsciiString::try_from("BTC-USD")?;

// Fast equality (header comparison first)
let other: AsciiString<32> = AsciiString::try_from("BTC-USD")?;
assert_eq!(symbol, other);

// Access data
assert_eq!(symbol.as_str(), "BTC-USD");
assert_eq!(symbol.len(), 7);

// Compile-time construction
const SYMBOL: AsciiString<32> = AsciiString::from_static("ETH-USD");
```

## Type Aliases

Common capacities have convenient aliases:

```rust
use nexus_ascii::{AsciiString8, AsciiString16, AsciiString32, AsciiString64};

let short: AsciiString8 = AsciiString8::try_from("BTC")?;
let symbol: AsciiString32 = AsciiString32::try_from("BTC-USD-PERP")?;
```

## String Operations

```rust
use nexus_ascii::AsciiString;

let symbol: AsciiString<32> = AsciiString::try_from("BTC-USD")?;

// Case-insensitive comparison
let other: AsciiString<32> = AsciiString::try_from("btc-usd")?;
assert!(symbol.eq_ignore_ascii_case(&other));

// Case conversion (returns new string)
let upper = symbol.to_ascii_uppercase();
let lower = symbol.to_ascii_lowercase();

// Validation helpers
assert!(!symbol.contains_control_chars());
assert!(symbol.is_all_printable());
```

## nohash-hasher Support

Enable the `nohash` feature for optimal HashMap performance:

```toml
[dependencies]
nexus-ascii = { version = "1.3", features = ["nohash"] }
```

```rust
use nexus_ascii::{AsciiString, AsciiHashMap};

let mut map: AsciiHashMap<32, u64> = AsciiHashMap::default();
let key: AsciiString<32> = AsciiString::try_from("BTC-USD")?;
map.insert(key, 42);
```

Since `AsciiString` stores a precomputed 48-bit XXH3 hash in its header, using
`nohash-hasher` avoids redundant hash computation during HashMap lookups. The
`Hash` impl applies a Fibonacci multiply finalizer for optimal bucket distribution
and SIMD group filtering — matching ahash/fxhash performance at all table sizes.

## Header Layout

Each `AsciiString` has an 8-byte header:
- **Bits 0-47**: XXH3 hash (lower 48 bits)
- **Bits 48-63**: String length

This layout ensures:
- Single 64-bit comparison for fast equality rejection
- Length accessible without touching the data buffer
- A Fibonacci multiply finalizer (`header * 0x9E3779B97F4A7C15`) provides uniform
  bucket distribution and proper h2 control byte entropy for hashbrown's SIMD filtering

## Performance

All operations are designed for predictable, low-latency performance.
Measured in CPU cycles via `rdtsc`, pinned to a single core:

| Operation | p50 | p99 |
|-----------|-----|-----|
| Construction (7B symbol) | 18 | 38 |
| Construction (32B, full cap) | 24 | 56 |
| Equality (same content) | 18 | 20 |
| Equality (different, header rejects) | 18 | 20 |
| HashMap get (nohash, n=1000) | 20 | 22 |
| HashMap insert (nohash, n=1000) | 18,938 | 28,910 |
| `cmp()` (7B) | 16 | 20 |
| `eq_ignore_ascii_case()` (38B) | 18 | 22 |
| `to_ascii_uppercase()` (20B) | 18 | 20 |

See [BENCHMARKS.md](BENCHMARKS.md) for the full benchmark suite.

## Collision Rate

With 48 bits of hash, collision probability follows the birthday paradox:

| Unique Strings | Expected Collisions |
|----------------|---------------------|
| 1 million | ~0.002 |
| 10 million | ~0.18 |
| 50 million | ~4.4 |

For typical workloads (< 1M unique strings), collisions are effectively impossible.

## When to Use

**Good fit:**
- Trading symbols, order IDs, session tokens
- Fixed-format protocol fields
- Keys in latency-sensitive HashMaps
- Any ASCII data with known maximum length

**Not ideal for:**
- Variable-length text of unknown size
- UTF-8 content
- Strings that need mutation after creation

## Features

| Feature | Description |
|---------|-------------|
| `std` (default) | Enable `std::error::Error` impls and `TryFrom<String>` |
| `nohash` | Enable nohash-hasher support for identity hashing (implies `std`) |
| `serde` | Enable `Serialize`/`Deserialize` for all types |
| `bytes` | Enable conversion to/from `bytes::Bytes` (implies `std`) |

## Serde Support

Enable the `serde` feature for serialization:

```toml
[dependencies]
nexus-ascii = { version = "1.3", features = ["serde"] }
```

```rust
use nexus_ascii::AsciiString;

let symbol: AsciiString<32> = AsciiString::try_from("BTC-USD")?;

// Serialize as string
let json = serde_json::to_string(&symbol)?; // "\"BTC-USD\""

// Deserialize with validation
let restored: AsciiString<32> = serde_json::from_str(&json)?;
```

Deserialization returns an error (not panic) if:
- The string exceeds capacity
- The string contains non-ASCII bytes
- For `AsciiText`, the string contains non-printable characters

## Bytes Crate Integration

Enable the `bytes` feature for async I/O integration:

```toml
[dependencies]
nexus-ascii = { version = "1.3", features = ["bytes"] }
```

```rust
use nexus_ascii::AsciiString;
use bytes::Bytes;

let symbol: AsciiString<32> = AsciiString::try_from("BTC-USD")?;

// Convert to Bytes
let b: Bytes = symbol.into();

// Convert from Bytes (with validation)
let restored: AsciiString<32> = AsciiString::try_from(b)?;
```

## `no_std` Support

This crate is `no_std` compatible. Disable default features to use in `no_std` environments:

```toml
[dependencies]
nexus-ascii = { version = "1.3", default-features = false }
```

Note: Without `std`, `Error` trait impls and `TryFrom<String>` conversions are unavailable.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
