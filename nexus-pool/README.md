# nexus-pool

High-performance object pools for latency-sensitive applications.

[![Crates.io](https://img.shields.io/crates/v/nexus-pool.svg)](https://crates.io/crates/nexus-pool)
[![Documentation](https://docs.rs/nexus-pool/badge.svg)](https://docs.rs/nexus-pool)
[![License](https://img.shields.io/crates/l/nexus-pool.svg)](LICENSE)

## Features

- **Sub-100 cycle operations**: ~26 cycles for local pools, ~42-68 cycles for sync pools
- **Zero allocation on hot path**: Pre-allocate objects at startup
- **RAII guards**: Objects automatically return to pool on drop
- **Manual take/put**: Owned values without RAII when guard lifetime doesn't fit
- **Graceful shutdown**: Guards safely drop values if pool is gone

## Quick Start

```rust
use nexus_pool::local::BoundedPool;

// Create a pool of 100 pre-allocated buffers
let pool = BoundedPool::new(
    100,
    || Vec::<u8>::with_capacity(1024),  // Factory
    |v| v.clear(),                       // Reset on return
);

// Acquire and use
let mut buf = pool.try_acquire().expect("pool not empty");
buf.extend_from_slice(b"hello world");

// Automatically returns to pool when `buf` drops
```

## Pool Types

### `local::BoundedPool` / `local::Pool`

Single-threaded pools with zero synchronization overhead.

```rust
use nexus_pool::local::Pool;

// Growable pool - creates objects on demand
let pool = Pool::new(
    || Vec::<u8>::with_capacity(1024),
    |v| v.clear(),
);

// RAII: auto-returns to pool on drop
let buf = pool.acquire();

// Manual: caller controls lifetime
let mut buf = pool.take();
buf.extend_from_slice(b"hello");
pool.put(buf); // reset is called, value returns to pool
```

### `sync::Pool`

Thread-safe pool: one thread acquires, any thread can return.

```rust
use nexus_pool::sync::Pool;

let pool = Pool::new(1000, || Vec::new(), |v| v.clear());

let buf = pool.try_acquire().unwrap();

// Send to another thread - returns to pool when dropped
std::thread::spawn(move || {
    println!("{:?}", &*buf);
});
```

## Design Philosophy

**Predictability over generality.**

This crate intentionally does *not* provide MPMC (multi-producer multi-consumer) pools. Here's why:

1. **MPMC requires solving ABA**: Generation counters, hazard pointers, or epoch-based reclamation add overhead and complexity.

2. **MPMC is a design smell**: If multiple threads contend for the same pool, you've created a bottleneck. The pool that was supposed to reduce latency now adds it.

3. **Better alternatives exist**:
   - Per-thread pools (`local::Pool` per thread)
   - Sharded pools (hash thread ID to pool index)
   - Message passing (send buffers through channels)

If you truly need MPMC, use [`crossbeam::ArrayQueue`](https://docs.rs/crossbeam/latest/crossbeam/queue/struct.ArrayQueue.html).

## Performance

Measured on Intel Core i9 @ 3.1 GHz:

| Pool | Acquire p50 | Release p50 | Release p99 |
|------|-------------|-------------|-------------|
| `local::BoundedPool` | 26 cycles | 26 cycles | 58 cycles |
| `local::Pool` (reuse) | 26 cycles | 26 cycles | 58 cycles |
| `local::Pool` (factory) | 32 cycles | 26 cycles | 58 cycles |
| `sync::Pool` (same thread) | 42 cycles | 68 cycles | 74 cycles |
| `sync::Pool` (cross-thread) | 42 cycles | 68 cycles | 86 cycles |

Run the benchmarks yourself:

```bash
cargo run --example perf_local_pool --release
cargo run --example perf_sync_pool --release
```

## Use Cases

### Trading Systems

```rust
use nexus_pool::sync::Pool;

// Order entry thread owns the pool
let pool = Pool::new(
    10_000,
    || Order::default(),
    |o| o.reset(),
);

// Hot path: acquire order, fill, send to matching engine
let mut order = pool.try_acquire().expect("order pool exhausted");
order.symbol = symbol;
order.price = price;
order.quantity = qty;

// Send to matching engine thread
matching_engine_tx.send(order).unwrap();
// Order returns to pool when matching engine drops it
```

### Network Buffers

```rust
use nexus_pool::local::BoundedPool;

// Per-connection buffer pool
let buffers = BoundedPool::new(
    16,
    || Box::new([0u8; 65536]),
    |b| { /* optional: zero sensitive data */ },
);

loop {
    let mut buf = buffers.try_acquire()?;
    let n = socket.read(&mut buf[..])?;
    process(&buf[..n]);
    // buf returns to pool
}
```

## Minimum Supported Rust Version

Rust 1.85 or later.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
