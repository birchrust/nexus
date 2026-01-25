# Nexus

Low-latency primitives for building high-performance systems.

## Philosophy

These crates are born from years of building trading infrastructure, where certain patterns become clear: most systems don't need unbounded queues, dynamic allocation, or multi-producer flexibility. They need **predictable, bounded, specialized primitives** that do one thing well and never surprise you at runtime.

The core philosophy is **predictability over generality**:

- **SPSC over MPMC** — When you have one producer and one consumer, don't pay for synchronization you don't need
- **Pre-allocation over dynamic growth** — Allocate at startup, never on the hot path
- **Bounded over unbounded** — Know your capacity, reject rather than allocate
- **Specialization over abstraction** — A conflation slot isn't a queue of size 1, it's a different thing entirely

The goal isn't "fastest in microbenchmarks." It's **consistent, low-latency behavior** under real workloads — minimizing tail latency, avoiding syscalls, eliminating allocation jitter.

Each crate is small, focused, and honest about its constraints. No kitchen sinks.

## Crates

### Communication

| Crate | Description |
|-------|-------------|
| [**nexus-queue**](./nexus-queue) | Lock-free SPSC ring buffer with per-slot lap counters. Two implementations: index-based (NUMA-friendly) and slot-based (shared-L3 friendly). |
| [**nexus-channel**](./nexus-channel) | Blocking SPSC channel built on nexus-queue. Three-phase backoff (spin → yield → park) minimizes syscalls under load. |
| [**nexus-slot**](./nexus-slot) | Single-value conflation slot. Writer always overwrites, reader gets latest value exactly once. For "latest wins" patterns like market data snapshots. |

### Storage & Allocation

| Crate | Description |
|-------|-------------|
| [**nexus-slab**](./nexus-slab) | Pre-allocated slab allocator. Fixed-capacity `BoundedSlab` for deterministic latency, growable `Slab` via independent chunks (no copy on growth). |
| [**nexus-pool**](./nexus-pool) | Object pools with RAII guards. Single-threaded `BoundedPool` and thread-safe `sync::Pool` (one acquirer, any returner). |

### Collections

| Crate | Description |
|-------|-------------|
| [**nexus-collections**](./nexus-collections) | Index-linked data structures with external storage. O(1) linked lists, O(log n) heaps, skip lists. Storage is separate from structure — move elements between collections without deallocation. |

### Identity & Strings

| Crate | Description |
|-------|-------------|
| [**nexus-id**](./nexus-id) | High-performance ID generators: Snowflake, UUID v4/v7, ULID. SIMD-accelerated hex encode/decode. Fibonacci mixing for identity hashers. |
| [**nexus-ascii**](./nexus-ascii) | Fixed-capacity ASCII strings. Stack-allocated, immutable, with precomputed 48-bit XXH3 hash. Identity-hashable via `nohash` feature for zero-cost lookups. |
| [**nexus-bits**](./nexus-bits) | Bit-packed integer newtypes via derive macros. Structs, tagged enums, `IntEnum` for discriminants. Zero-cost `#[repr(transparent)]` with compile-time validation. |

## Planned

| Crate | Description |
|-------|-------------|
| **nexus-logbuf** | Bounded SPSC and MPSC byte ring buffer for variable-length messages. Offset-based claiming, 8-byte aligned records, claim-based API (`WriteClaim`/`ReadClaim`). Returns `Err(Full)` on exhaustion — caller decides policy. The hot-path primitive for getting data off the trading loop. |
| **MPSC queue** | Bounded multi-producer, single-consumer lock-free queue. CAS-based tail claiming, wait-free consumer. For buffer return paths and aggregation where producers are not on the hot path. |

## Design Principles

### No allocation on the hot path

Every crate that manages memory supports pre-allocation. You pay the cost at startup, not when processing the millionth message.

### Honest constraints

SPSC means SPSC. Don't sneak in an extra producer and expect it to work. The constraints enable the performance.

### Benchmark what matters

Synthetic throughput is easy to game. We optimize for realistic workloads: ping-pong latency, p99/p999 tail latency, jitter under load.

### Minimal dependencies

These are foundational crates. Dependency trees are kept small and intentional.

## Platform Support

- **Linux** — Primary target, fully supported
- **macOS** — Supported
- **Windows** — Experimental where noted, typically behind feature flags

## Contributing

Please read [CONTRIBUTING.md](./CONTRIBUTING.md) before submitting changes.

The short version: we build specialized primitives, not general-purpose ones. Different constraints mean different problems, and different problems deserve different solutions. If you're proposing a feature, be ready to justify why it belongs in a tuned, minimal implementation.

We also have specific benchmarking standards — cycles not time, turbo boost disabled, cores pinned, jitter eliminated. Details in the contributing guide.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
