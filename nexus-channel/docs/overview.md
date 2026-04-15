# Overview

`nexus-channel` is a blocking SPSC channel. One sender, one receiver,
bounded capacity, values delivered in FIFO order. The wire-level transport
is [`nexus_queue::spsc::ring_buffer`](../../nexus-queue) — this crate adds
blocking semantics and backpressure on top.

## When to use this instead of a raw queue

Use `nexus-queue` directly when:

- Both sides are polling — the consumer has its own poll loop and can
  tolerate empty reads cheaply.
- You never want the producer or consumer to block.
- You're building your own wakeup mechanism (mio, io_uring, a notify
  queue, etc.).

Use `nexus-channel` when:

- The producer and consumer are on different threads and at least one
  side wants to *block* when the other isn't ready.
- You want clean shutdown semantics — dropping one end wakes the other
  with an error.
- You want backpressure: if the queue is full, `send()` blocks until the
  consumer drains.

## Why not crossbeam-channel?

`crossbeam-channel` is excellent for MPMC. For pure SPSC, `nexus-channel`
is faster on the tail:

| Metric | nexus-channel | crossbeam-channel | Ratio |
|--------|---------------|-------------------|-------|
| p50 latency | 665 cy | 1344 cy | 2.0× |
| p999 latency | 2501 cy | 37023 cy | **14.8×** |
| Throughput | 64M msg/s | 34M msg/s | 1.9× |

The big p999 difference comes from *conditional parking*: nexus-channel
only issues an unpark syscall when it can prove the other end is actually
sleeping. Crossbeam unparks on every send/recv transition for correctness
across all topologies, which is the right choice for MPMC but too
pessimistic for SPSC.

If you're MPMC, use crossbeam. If you're MPSC specifically, use
[`nexus_queue::mpsc`](../../nexus-queue) directly. If you're SPSC and want
blocking semantics, this crate is the right tool.

## What "blocking" means here

The API is synchronous — no `.await`, no runtime. `send()` and `recv()`
block the *calling OS thread* until the operation can complete. When the
blocking path fires, the implementation progresses through three stages:

1. **Fast path.** Just try the queue operation. This is the 99% case.
2. **Spin.** Use `crossbeam_utils::Backoff` to snooze for a few iterations.
   No syscalls, no atomics beyond the queue itself.
3. **Park.** `std::thread::park()` until the other end explicitly unparks.
   This is the only path that touches the kernel.

Conditional parking (only unpark if the other side has registered as
parked) means step 3 *never* fires for the common case where both sides
are busy. See [backoff.md](backoff.md).

## Semantics at a glance

```rust
use nexus_channel::channel;

let (tx, rx) = channel::<u64>(1024);

tx.send(42).unwrap();              // FIFO
assert_eq!(rx.recv().unwrap(), 42);

drop(rx);
assert!(tx.send(7).is_err());      // receiver gone → SendError(7)
```

- **Capacity** rounds up to the next power of two (ring buffer requirement).
- **Drop wakes the other side.** Dropping the sender causes pending
  `recv()` calls to return `RecvError`. Dropping the receiver causes
  pending `send()` calls to return `SendError(value)`, with the unsent
  value returned to the caller.
- **FIFO.** Values are delivered in the order they were sent.
- **Bounded.** `send()` blocks when the queue is full. Use `try_send()`
  for non-blocking insert.
