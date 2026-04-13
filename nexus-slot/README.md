# nexus-slot

High-performance conflation slots for "latest value wins" scenarios.

## Overview

`nexus-slot` provides single-value slots optimized for the common pattern where only the most recent value matters: market data snapshots, sensor readings, configuration updates, position state, etc.

- **Writer** overwrites with newest data (never blocks)
- **Reader** gets each new value exactly once
- **Old values** are silently discarded (conflated)

Two variants based on reader topology:

| Variant | Readers | Overhead |
|---------|---------|----------|
| [`spsc`] | Single reader | Lowest — disconnect via refcount |
| [`spmc`] | Multiple readers (`Clone`) | +1 `AtomicBool` for writer-dropped flag |

## Performance

Benchmarked on Intel Core Ultra 7 155H, pinned to physical cores with turbo disabled:

| Implementation | p50 Latency | Notes |
|----------------|-------------|-------|
| **nexus-slot** | 159 cycles (59 ns) | SPSC only |
| `seqlock` crate | 355 cycles (132 ns) | Supports multiple writers |
| `ArrayQueue(1)` | 540 cycles (201 ns) | General-purpose bounded queue |

The **2.2x speedup** over `seqlock` comes from specializing for single-producer:

- No writer contention → no CAS loops
- Cached sequence number → writer avoids atomic load
- No queue machinery → just a sequence counter

## Usage

### SPSC — Single Reader

```rust
use nexus_slot::spsc;

#[derive(Copy, Clone, Default)]
struct Quote {
    bid: f64,
    ask: f64,
    sequence: u64,
}

let (mut writer, mut reader) = spsc::slot::<Quote>();

// Writer side (e.g., market data thread)
writer.write(Quote { bid: 100.0, ask: 100.05, sequence: 1 });
writer.write(Quote { bid: 100.1, ask: 100.15, sequence: 2 });  // Overwrites previous

// Reader side (e.g., trading logic thread)
let quote = reader.read();  // Gets sequence 2, skipped 1
assert_eq!(quote.unwrap().sequence, 2);

// Already consumed - returns None until next write
assert!(reader.read().is_none());
```

### SPMC — Multiple Readers

```rust
use nexus_slot::spmc;

let (mut writer, mut reader1) = spmc::shared_slot::<u64>();
let mut reader2 = reader1.clone();

writer.write(42);

// Each reader consumes independently
assert_eq!(reader1.read(), Some(42));
assert_eq!(reader2.read(), Some(42));

// Both consumed
assert!(reader1.read().is_none());
assert!(reader2.read().is_none());
```

## Semantics

### Conflation

Multiple writes before a read result in only the latest value being observed:

```text
writer.write(value_1);
writer.write(value_2);
writer.write(value_3);

// Reader only sees value_3
assert_eq!(reader.read(), Some(value_3));
assert!(reader.read().is_none());
```

### Exactly-Once Delivery

Each written value can be read at most once per reader:

```text
writer.write(value);

assert!(reader.read().is_some());  // Consumes it
assert!(reader.read().is_none());  // Already consumed
```

### Check Without Consuming

```text
if reader.has_update() {
    // New data available, but not consumed yet
    let value = reader.read();  // Now consumed
}
```

## The `Pod` Trait

Types must implement `Pod` (Plain Old Data) — no heap allocations or drop glue.

Any `Copy` type automatically implements `Pod`:

```rust
use nexus_slot::spsc::slot;

// These work automatically
let (w, r) = slot::<u64>();
let (w, r) = slot::<[f64; 4]>();
let (w, r) = slot::<(i32, i32)>();
```

For non-`Copy` types that are still just bytes:

```rust
use nexus_slot::Pod;

#[repr(C)]
struct OrderBook {
    bids: [f64; 20],
    asks: [f64; 20],
    sequence: u64,
}

// SAFETY: OrderBook is just bytes, no heap allocations
unsafe impl Pod for OrderBook {}

let (mut writer, mut reader) = nexus_slot::spsc::slot::<OrderBook>();
```

**Pod requirements:**
- No `Vec`, `String`, `Box`, `Arc`, or other heap types
- No `File`, `TcpStream`, `Mutex`, or other resources
- `std::mem::needs_drop::<T>()` must be `false`

## API

### SPSC

| Type | Method | Description |
|------|--------|-------------|
| `Writer` | `write(value)` | Overwrite with new value (never blocks) |
| `Writer` | `is_disconnected()` | Returns true if reader was dropped |
| `Reader` | `read() -> Option<T>` | Get latest value if new, consuming it |
| `Reader` | `has_update() -> bool` | Check for new data without consuming |
| `Reader` | `is_disconnected()` | Returns true if writer was dropped |

### SPMC

| Type | Method | Description |
|------|--------|-------------|
| `Writer` | `write(value)` | Overwrite with new value (never blocks) |
| `Writer` | `is_disconnected()` | Returns true if **all** readers were dropped |
| `SharedReader` | `read() -> Option<T>` | Get latest value if new, consuming it |
| `SharedReader` | `has_update() -> bool` | Check for new data without consuming |
| `SharedReader` | `is_disconnected()` | Returns true if writer was dropped |
| `SharedReader` | `clone()` | Create independent reader at same position |

## Implementation

The sequence counter starts at 2 (not 0) to provide 32-bit wrap protection. Starting at 0 would mean a full 32-bit wrap lands back on 0, which is indistinguishable from "no writes have occurred." Starting at 2 shifts the wrap point away from the initial state.

Uses a sequence lock (seqlock) internally:

1. Writer increments sequence to odd (write in progress)
2. Writer copies data via word-at-a-time atomic stores (`Relaxed`)
3. Writer increments sequence to even (write complete)
4. Reader loads sequence, copies data via word-at-a-time atomic loads, checks sequence unchanged
5. If sequence changed during read, retry

The single-producer constraint allows caching the sequence on the writer side, eliminating an atomic load per write. Standalone `Release`/`Acquire` fences provide ordering.

## Thread Safety

| Type | `Send` | `Sync` | Notes |
|------|--------|--------|-------|
| `spsc::Writer<T>` | Yes | No | One thread only |
| `spsc::Reader<T>` | Yes | No | One thread only |
| `spmc::Writer<T>` | Yes | No | One thread only |
| `spmc::SharedReader<T>` | Yes | No | Clone for each thread |

## License

MIT OR Apache-2.0
