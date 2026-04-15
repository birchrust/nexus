# nexus-queue docs

Lock-free bounded ring buffers with three topologies: SPSC, MPSC, SPMC.

## Start here

- [overview.md](./overview.md) — Ring buffer design, when to use which variant
- [spsc.md](./spsc.md) — Single-producer single-consumer (the fastest path)
- [mpsc.md](./mpsc.md) — Multi-producer single-consumer (CAS-claimed tail)
- [spmc.md](./spmc.md) — Single-producer multi-consumer (fan-out)
- [patterns.md](./patterns.md) — Cookbook: feed fan-out, order entry, log aggregation
- [performance.md](./performance.md) — Cycles, comparisons, decision criteria

## TL;DR

```rust
use nexus_queue::{spsc, mpsc, spmc};

// One writer, one reader. Fastest.
let (tx, rx) = spsc::ring_buffer::<u64>(1024);

// Many writers, one reader.
let (tx, rx) = mpsc::ring_buffer::<u64>(1024);
let tx2 = tx.clone();

// One writer, many readers (each value goes to exactly one consumer).
let (tx, rx) = spmc::ring_buffer::<u64>(1024);
let rx2 = rx.clone();
```

All three use per-slot lap counters (Vyukov turn sequences) for
synchronization and return `Result<(), Full<T>>` from `push` — the
value comes back on failure, never dropped.

## Related crates

- [`nexus-channel`](../../nexus-channel/) — blocking wrapper on top of SPSC
- [`nexus-logbuf`](../../nexus-logbuf/) — byte-oriented variable-length variant
- [`nexus-pool`](../../nexus-pool/) — reuse the `T` objects that flow through these queues
