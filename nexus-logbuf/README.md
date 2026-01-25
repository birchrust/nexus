# nexus-logbuf

High-performance lock-free ring buffers for variable-length messages.

## Purpose

A bytes-in, bytes-out ring buffer primitive for archival and logging. Producer
writes raw bytes into a pre-allocated buffer; consumer reads them out on a
background thread. No policy decisions—you define the framing, serialization,
and I/O strategy on top.

**Target use cases:**
- WebSocket message archival
- Structured binary logging
- Event sourcing / journaling
- FIX message archival

## Design

- **Flat byte buffer** with free-running offsets, power-of-2 capacity
- **len-as-commit**: Record's len field is the commit marker (non-zero = ready)
- **Skip markers**: High bit of len distinguishes padding/aborted claims
- **Consumer zeroing**: Required for variable-length records across laps
- **Claim-based API**: `WriteClaim`/`ReadClaim` with RAII semantics

## Modules

### `queue` — Low-level primitives

Maximum control, no blocking, no backpressure handling.

```rust
use nexus_logbuf::queue::spsc;

let (mut producer, mut consumer) = spsc::new(4096);

// Producer (hot path)
let payload = b"hello world";
if let Ok(mut claim) = producer.try_claim(payload.len()) {
    claim.copy_from_slice(payload);
    claim.commit();
}

// Consumer (background thread)
if let Some(record) = consumer.try_claim() {
    assert_eq!(&*record, b"hello world");
    // record dropped -> zeros region, advances head
}
```

### `channel` — Ergonomic blocking API

Wraps queues with backoff and parking for receivers.

```rust
use nexus_logbuf::channel::spsc;
use std::thread;

let (mut tx, mut rx) = spsc::channel(4096);

thread::spawn(move || {
    let payload = b"hello";
    let mut claim = tx.send(payload.len()).unwrap();
    claim.copy_from_slice(payload);
    claim.commit();
    tx.notify();
});

let record = rx.recv(None).unwrap();
assert_eq!(&*record, b"hello");
```

## Variants

| Variant | Producer | Consumer | Use Case |
|---------|----------|----------|----------|
| `spsc` | Single | Single | Lowest latency, dedicated archiver thread |
| `mpsc` | Multiple | Single | Multiple hot threads → single archiver |

## Philosophy

**Senders are never slowed down.** They use immediate `try_send()` or brief
backoff, never syscalls. If the buffer is full, they return an error.

**Receivers can block.** They use `park_timeout` to wait without burning CPU.

## Performance

See [BENCHMARKS.md](./BENCHMARKS.md) for detailed numbers.

**Summary (64-byte payload):**

| Metric | SPSC | MPSC (1P) | MPSC (4P) |
|--------|------|-----------|-----------|
| Producer p50 | 40 cycles | 42 cycles | 340 cycles |
| Consumer p50 | 26 cycles | 28 cycles | 28 cycles |
| Throughput | 20.7 GB/s | 38M msg/s | 13M msg/s |

## License

MIT OR Apache-2.0
