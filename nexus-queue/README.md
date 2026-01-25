# nexus-queue

A high-performance SPSC (Single-Producer Single-Consumer) ring buffer for Rust, optimized for ultra-low-latency messaging.

## Performance

Benchmarked on AMD Ryzen (single-socket), 2.69 GHz base clock, pinned to physical cores:

| Metric | nexus-queue | rtrb | crossbeam (MPMC) |
|--------|-------------|------|------------------|
| **p50 latency** | 68 cycles (25 ns) | 67 cycles (25 ns) | 83 cycles (31 ns) |
| **p99 latency** | 130 cycles | 123 cycles | 160 cycles |
| **Throughput** | 640 M msgs/sec | 485 M msgs/sec | 92 M msgs/sec |

See [BENCHMARKS.md](./BENCHMARKS.md) for detailed methodology and results.

## Usage

```rust
use nexus_queue::spsc;

let (mut tx, mut rx) = spsc::ring_buffer::<u64>(1024);

// Producer thread
tx.push(42).unwrap();

// Consumer thread
assert_eq!(rx.pop(), Some(42));
```

### Handling backpressure

```rust
use nexus_queue::Full;

// Spin until space is available
while tx.push(msg).is_err() {
    std::hint::spin_loop();
}

// Or handle the full case
match tx.push(msg) {
    Ok(()) => { /* sent */ }
    Err(Full(returned_msg)) => { /* queue full, msg returned */ }
}
```

### Disconnection detection

```rust
// Check if the other end has been dropped
if rx.is_disconnected() {
    // Producer was dropped, drain remaining messages
}

if tx.is_disconnected() {
    // Consumer was dropped, stop producing
}
```

## Design

```
┌─────────────────────────────────────────────────────────────┐
│ Shared (Arc):                                               │
│   tail: CachePadded<AtomicUsize>   ← Producer writes        │
│   head: CachePadded<AtomicUsize>   ← Consumer writes        │
│   buffer: *mut T                                            │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────┐     ┌─────────────────────┐
│ Producer:           │     │ Consumer:           │
│   local_tail        │     │   local_head        │
│   cached_head       │     │   cached_tail       │
│   buffer (cached)   │     │   buffer (cached)   │
└─────────────────────┘     └─────────────────────┘
```

Producer and consumer write to **separate cache lines** (128-byte padding). Each endpoint caches the buffer pointer, mask, and the other's index locally, only refreshing from atomics when the cache indicates full/empty.

This design performs well on multi-socket NUMA systems where cache line ownership is important for latency.

## Benchmarking

For accurate results, disable turbo boost and pin to physical cores:

```bash
# Build
cargo build -p nexus-queue --examples --release

# Run pinned to two cores
taskset -c 0,1 ./target/release/examples/bench_spsc

# For more stable results, disable turbo boost:
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
# Re-enable after:
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
```

Verify your core topology with `lscpu -e` — you want cores with different CORE numbers to avoid hyperthreading siblings.

## Memory Ordering

Uses manual fencing for clarity and portability:

- **Producer**: `fence(Release)` before publishing tail
- **Consumer**: `fence(Acquire)` after reading tail, `fence(Release)` before advancing head

On x86 these compile to no instructions (strong memory model), but they're required for correctness on ARM and other weakly-ordered architectures.

## When to Use This

**Use nexus-queue when:**
- You have exactly one producer and one consumer
- You need the lowest possible latency
- You're building trading systems, audio pipelines, or real-time applications

**Consider alternatives when:**
- Multiple producers → use MPSC queues
- Multiple consumers → use MPMC queues
- You need async/await → use `tokio::sync::mpsc`

## License

MIT OR Apache-2.0
