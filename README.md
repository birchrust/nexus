# Nexus

Low-latency primitives and runtime for building high-performance systems.

## Philosophy

These crates are born from years of building trading infrastructure, where
certain patterns become clear: most systems don't need unbounded queues,
dynamic allocation, or multi-producer flexibility. They need **predictable,
bounded, specialized primitives** that do one thing well and never surprise
you at runtime.

The core philosophy is **predictability over generality**:

- **SPSC over MPMC** — When you have one producer and one consumer, don't pay for synchronization you don't need
- **Pre-allocation over dynamic growth** — Allocate at startup, never on the hot path
- **Bounded over unbounded** — Know your capacity, reject rather than allocate
- **Specialization over abstraction** — A conflation slot isn't a queue of size 1, it's a different thing entirely

The goal isn't "fastest in microbenchmarks." It's **consistent, low-latency
behavior** under real workloads — minimizing tail latency, avoiding syscalls,
eliminating allocation jitter.

Each crate is small, focused, and honest about its constraints. No kitchen sinks.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Applications                         │
│            (Trading systems, event loops)               │
└──────────────────────┬──────────────────────────────────┘
                       │
         ┌─────────────┴────────────────┐
         ▼                              ▼
┌──────────────────┐          ┌──────────────────┐
│    nexus-rt      │          │ nexus-stats      │
│  (runtime)       │          │ nexus-rate       │
│                  │          │ (monitoring &    │
│  World, Handlers │          │  flow control)   │
│  Pipelines, DAGs │          │                  │
│  Drivers, Clock  │          │                  │
└────────┬─────────┘          └──────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────────────┐
│                     Primitives                          │
│                                                         │
│  nexus-queue    nexus-slab     nexus-id     nexus-bits  │
│  nexus-channel  nexus-pool     nexus-ascii              │
│  nexus-notify   nexus-timer    nexus-logbuf             │
│  nexus-slot     nexus-collections  nexus-smartptr       │
│  nexus-decimal                                          │
└─────────────────────────────────────────────────────────┘
```

## Crates

### Runtime

| Crate | Description |
|-------|-------------|
| [**nexus-rt**](./nexus-rt) | Event-driven runtime. World/ECS resource model, handler dispatch, pipelines, DAGs, driver system, clock. No async/await — explicit poll loops with monomorphized zero-cost dispatch. |

### Monitoring & Flow Control

| Crate | Description |
|-------|-------------|
| [**nexus-stats**](./nexus-stats) | 45 streaming statistics algorithms. EMA, CUSUM, Welford, Kalman, KAMA, change detection, anomaly filtering, and more. O(1) per update, fixed memory, `no_std`. [Full docs](./nexus-stats/docs/INDEX.md). |
| [**nexus-rate**](./nexus-rate) | Rate limiting. GCRA, token bucket, sliding window counter. Single-threaded and thread-safe variants. Weighted requests. ~2-4 cycle hot path. |

### Communication & Notification

| Crate | Description |
|-------|-------------|
| [**nexus-queue**](./nexus-queue) | Lock-free SPSC, MPSC, and SPMC ring buffers with per-slot lap counters. Index-based (NUMA-friendly) and slot-based (shared-L3 friendly) implementations. |
| [**nexus-channel**](./nexus-channel) | Blocking SPSC channel built on nexus-queue. Three-phase backoff (spin → yield → park) minimizes syscalls under load. |
| [**nexus-notify**](./nexus-notify) | Cross-thread event queue with conflation and FIFO delivery. Non-blocking `event_queue` and blocking `event_channel`. Dedup flags + MPSC ring buffer — O(limit) poll, ~5 cycles/token. |
| [**nexus-slot**](./nexus-slot) | Single-value conflation slot. Writer always overwrites, reader gets latest value exactly once. For "latest wins" patterns like market data snapshots. |
| [**nexus-logbuf**](./nexus-logbuf) | Bounded SPSC and MPSC byte ring buffers. Claim-based API for variable-length messages. The hot-path primitive for getting data off the event loop without syscalls. |

### Storage & Allocation

| Crate | Description |
|-------|-------------|
| [**nexus-slab**](./nexus-slab) | Pre-allocated slab allocator. Fixed-capacity `BoundedSlab` for deterministic latency, growable `Slab` via independent chunks (no copy on growth). |
| [**nexus-pool**](./nexus-pool) | Object pools with RAII guards. Single-threaded `BoundedPool` and thread-safe `sync::Pool` (one acquirer, any returner). |
| [**nexus-timer**](./nexus-timer) | Hierarchical timer wheel with O(1) insert and cancel. No-cascade design inspired by the Linux kernel. Slab-backed, zero allocation after init. |
| [**nexus-smartptr**](./nexus-smartptr) | Inline and flexible smart pointers for type-erased storage. `FlatBox` (fixed inline), `FlexBox` (inline or heap). Avoids boxing for small handler types. |

### Collections

| Crate | Description |
|-------|-------------|
| [**nexus-collections**](./nexus-collections) | Slab-backed intrusive collections. O(1) linked lists, O(log n) heaps, red-black trees, B-trees. Internal allocation via `nexus-slab` — user sees keys and values, not nodes. |

### Numeric

| Crate | Description |
|-------|-------------|
| [**nexus-decimal**](./nexus-decimal) | Fixed-point decimal arithmetic with compile-time precision. `Decimal<i64, 8>` for prices, `Decimal<i128, 12>` for DeFi. Const fn, `no_std`, zero allocation. Financial methods: midpoint, tick rounding, basis points. Chunked magic division avoids `__divti3`. |

### Identity & Encoding

| Crate | Description |
|-------|-------------|
| [**nexus-id**](./nexus-id) | High-performance ID generators: Snowflake, UUID v4/v7, ULID. SIMD-accelerated hex encode/decode. Fibonacci mixing for identity hashers. |
| [**nexus-ascii**](./nexus-ascii) | Fixed-capacity ASCII strings. Stack-allocated, immutable, with precomputed 48-bit XXH3 hash. Identity-hashable via `nohash` feature for zero-cost lookups. |
| [**nexus-bits**](./nexus-bits) | Bit-packed integer newtypes via derive macros. Structs, tagged enums, `IntEnum` for discriminants. Zero-cost `#[repr(transparent)]` with compile-time validation. |

## Design Principles

### No allocation on the hot path

Every crate that manages memory supports pre-allocation. You pay the cost
at startup, not when processing the millionth message.

### Honest constraints

SPSC means SPSC. Don't sneak in an extra producer and expect it to work.
The constraints enable the performance.

### Benchmark what matters

Synthetic throughput is easy to game. We optimize for realistic workloads:
ping-pong latency, p99/p999 tail latency, jitter under load. See
individual crate `BENCHMARKS.md` files for methodology and results.

### Minimal dependencies

These are foundational crates. Dependency trees are kept small and intentional.

## Platform Support

- **Linux** — Primary target, fully supported
- **macOS** — Supported
- **Windows** — Experimental where noted, typically behind feature flags

## Contributing

Please read [CONTRIBUTING.md](./CONTRIBUTING.md) before submitting changes.

The short version: we build specialized primitives, not general-purpose ones.
Different constraints mean different problems, and different problems deserve
different solutions. If you're proposing a feature, be ready to justify why
it belongs in a tuned, minimal implementation.

We also have specific benchmarking standards — cycles not time, turbo boost
disabled, cores pinned, jitter eliminated. Details in the contributing guide.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
