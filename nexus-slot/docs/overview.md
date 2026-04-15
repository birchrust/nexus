# Overview

A conflation slot is a shared memory cell where:

- The writer *always* succeeds. Writes never block and never fail.
- The reader sees the *latest* value that was written.
- Each new value is delivered to each reader exactly once.

That last property is the thing that distinguishes `nexus-slot` from a
plain shared variable, and it's also the thing that distinguishes it from
"a queue of size 1".

## Why not just a queue of size 1?

A bounded queue of capacity 1 has totally different semantics under
backpressure. If the slot is full and the writer tries to push, a queue
will either:

- Block the writer (SPSC channel).
- Reject the write (try_send).
- Drop the *new* value (which is exactly the wrong thing for a
  latest-value-wins workload).

A conflation slot does none of those. The writer *always* installs the new
value, overwriting whatever was there. The reader always sees the newest
value on the next read. If the reader is slow, intermediate writes are
simply gone — which is *correct* for the workloads this crate targets.

## The `Pod` requirement

Conflation is implemented as a seqlock: the writer bumps a sequence
counter, copies bytes word-at-a-time, bumps the counter again; the reader
speculatively copies bytes and retries if the sequence changed mid-copy.

For this to be sound, the stored value must be:

- Byte-copyable (no drop glue, no heap pointers).
- Safe to observe in any intermediate state (because the reader might see
  a torn copy before retrying).

The `Pod` marker trait enforces this. `Copy` types implement `Pod`
automatically; non-`Copy` types require a manual `unsafe impl Pod for T`
with a justification that the bytes are safe to copy.

```rust
#[repr(C)]
pub struct Quote {
    pub bid: f64,
    pub ask: f64,
    pub seq: u64,
}

// Auto: Quote is Copy (derived), so Pod comes for free.
impl Copy for Quote {}
impl Clone for Quote { fn clone(&self) -> Self { *self } }
```

If you're storing a large struct you don't want to derive `Copy` on,
implement `Pod` manually:

```rust
use nexus_slot::Pod;

#[repr(C)]
pub struct OrderBook {
    pub bids: [(f64, f64); 20],
    pub asks: [(f64, f64); 20],
}

// SAFETY: OrderBook is `repr(C)`, contains only `f64`, no drop glue,
// no heap pointers. Bytes are safe to memcpy.
unsafe impl Pod for OrderBook {}
```

## SPSC vs SPMC

Two variants:

- **`spsc::slot<T>()`** — one writer, one reader. Lower overhead; no
  reader fan-out machinery. `~159 cy p50` on a modern x86 core.
- **`spmc::shared_slot<T>()`** — one writer, many readers. Each
  `SharedReader` has its own consumption state, so every reader sees
  every new value once. `SharedReader: Clone` — clone gives you a new
  reader tracked independently.

Use SPSC unless you need multiple consumers of the same stream.

## When to reach for a slot

- Market data snapshots. The consumer wants "the latest book", not every
  intermediate update.
- Sensor readings and gauges. You only care about the current value, not
  the history.
- Configuration updates. Readers check-and-apply; intermediate states are
  fine to drop.
- Any "latest value wins" pattern where the consumer's work rate is
  independent of the producer's.

## When *not* to use a slot

- You need every value (order fills, logs, trade ticks). Use
  [`nexus-queue`](../../nexus-queue) or [`nexus-logbuf`](../../nexus-logbuf).
- You need backpressure. A slot never applies backpressure — if the reader
  can't keep up, it misses updates silently. Use
  [`nexus-channel`](../../nexus-channel) if slow consumers should slow
  producers down.
- Your type isn't `Pod`. Box-containing types, strings, and `Arc`s don't fit.
