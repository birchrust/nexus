# nexus-rate

Fixed-memory, zero-allocation rate limiting for real-time systems.

Three algorithms, two threading models, weighted requests. Every check is
O(1) with single-digit cycle overhead. `no_std` compatible.

## Quick Start

```rust
use nexus_rate::local::Gcra;

// 100 requests per second, burst of 10
let mut limiter = Gcra::builder()
    .rate(100)
    .period(1_000_000_000)  // 1 second in nanoseconds
    .burst(10)
    .build()
    .unwrap();

// On each request:
if limiter.try_acquire(1, now_ns) {
    process_request();
} else {
    reject_or_backoff();
}
```

## Algorithms

| Algorithm | What It Does | State | Allowed (p50) |
|-----------|-------------|-------|--------------|
| **GCRA** | Virtual scheduling. One multiply with precomputed interval. No division. | 8 bytes | 2 cycles |
| **TokenBucket** | Lazy token computation. Burst-tolerant. | 8 bytes + config | 2 cycles |
| **SlidingWindow** | Exact count over rolling time window. | N×8 bytes | 4 cycles |

All three share the same primary API: `try_acquire(cost, now) -> bool`.

### When to Use Which

- **GCRA** — simplest, fastest. Steady-rate limiting with burst tolerance.
  No multiplication on the check path.
- **TokenBucket** — same capability as GCRA but uses the "available tokens"
  mental model. Has `available(now)` query.
- **SlidingWindow** — when you need an exact event count over a time window.
  Use this to mirror exchange rate limit logic (e.g., "1200 orders per minute").

## Threading Models

```rust
// Single-threaded — &mut self, no atomics
use nexus_rate::local::Gcra;
let mut limiter = Gcra::builder()
    .rate(100).period(1_000_000_000).burst(10)
    .build().unwrap();
limiter.try_acquire(1, now);  // &mut self

// Multi-threaded — &self, CAS loop on atomics
use nexus_rate::sync::Gcra;
let limiter = Gcra::builder()
    .rate(100).period(1_000_000_000).burst(10)
    .build().unwrap();
limiter.try_acquire(1, now);  // &self — safe to share via Arc
```

| Module | `try_acquire` | Sync | Cost |
|--------|--------------|------|------|
| `local` | `&mut self` | Single-threaded | 2-4 cycles |
| `sync` | `&self` | Thread-safe (CAS) | 21-31 cycles |

## Weighted Requests

Some systems weight operations differently. For example, an exchange
might count cancel=1, new_order=2, amend=3:

```rust
// cost parameter controls the weight
limiter.try_acquire(1, now);  // cancel — weight 1
limiter.try_acquire(2, now);  // new order — weight 2
limiter.try_acquire(3, now);  // amend — weight 3
```

Applies uniformly: GCRA advances TAT by `cost × emission_interval`,
TokenBucket consumes `cost` tokens, SlidingWindow adds `cost` to the count.

## Runtime Reconfiguration

Rate limits change — exchange adjusts limits, admin command, config reload:

```rust
// Change rate/burst at runtime without rebuilding
limiter.reconfigure(200, 1_000_000_000, 20);
```

No allocation, no state reset. Takes effect on the next `try_acquire`.

## Multi-Rate Composition

Exchanges often enforce multiple limits (e.g., 10/s AND 1200/min).
Compose by checking multiple limiters:

```rust
let mut per_second = Gcra::builder()
    .rate(10).period(1_000_000_000).burst(3)
    .build().unwrap();
let mut per_minute = Gcra::builder()
    .rate(1200).period(60_000_000_000).burst(50)
    .build().unwrap();

// Both must allow:
if per_second.try_acquire(1, now) && per_minute.try_acquire(1, now) {
    send_order();
}
```

## API Summary

| Method | What |
|--------|------|
| `try_acquire(cost, now) -> bool` | Can I proceed? |
| `time_until_allowed(cost, now) -> u64` | How long to wait? (GCRA) |
| `available(now) -> u64` | Tokens remaining (TokenBucket) |
| `count() -> u64` | Current window count (SlidingWindow) |
| `remaining() -> u64` | Capacity left (SlidingWindow) |
| `reconfigure(...)` | Change limits at runtime |
| `reset(...)` | Clear state |

## Performance

All measurements in CPU cycles (`rdtsc`), batch of 64 checks, pinned core.

| Type | Allowed (p50) | Rejected (p50) |
|------|--------------|----------------|
| `local::Gcra` | 2 | 0 |
| `sync::Gcra` | 21 | 2 |
| `local::TokenBucket` | 2 | 0 |
| `sync::TokenBucket` | 31 | 4 |
| `local::SlidingWindow` | 4 | 2 |

Rejected paths are 0-2 cycles — the branch predictor perfectly predicts
rejection, so no wasted computation on the fast-reject path.

```bash
cargo build --release --example perf_rate -p nexus-rate
taskset -c 0 ./target/release/examples/perf_rate
```

## Features

| Feature | Default | What |
|---------|---------|------|
| `std` | yes | Implies `alloc`. `Error` trait on `ConfigError`. |
| `alloc` | no | Enables `SlidingWindow` (heap-allocated ring buffer). |

GCRA and TokenBucket work without any features (`no_std`, no `alloc`).
SlidingWindow requires `alloc`.

## Hot Path Internals

GCRA and TokenBucket precompute `nanos_per_token` at construction and reconfiguration time, avoiding u128 divisions on the hot path. Token computation uses ceil-division to guarantee that fractional tokens are never silently lost -- `try_acquire(1, now)` always consumes at least one token's worth of time.

## Timestamps

All timestamps are `u64`. The caller defines what the units mean —
nanoseconds, rdtsc cycles, milliseconds, etc. The algorithms don't
read clocks internally, making them deterministic and testable.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
