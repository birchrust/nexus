# `local` vs `sync`

Every algorithm in `nexus-rate` has two variants:

- **`local::*`**: `&mut self` methods, no atomics. Single-threaded.
- **`sync::*`**: `&self` methods backed by atomic CAS. Shareable
  across threads.

The APIs are identical except for the receiver: `local`
requires `&mut self`, `sync` requires `&self` (so can live
behind an `Arc`).

## Local

```rust
use nexus_rate::local::Gcra;
use std::time::{Duration, Instant};

let mut limiter = Gcra::builder()
    .rate(100).period(Duration::from_secs(1))
    .build().unwrap();

// Single thread only.
if limiter.try_acquire(1, Instant::now()) {
    // ...
}
```

- **Performance**: ~5-10 cycles per `try_acquire`. No atomics,
  no CAS, no memory barriers.
- **Thread model**: one thread owns the limiter. Period.
- **Use when**: you have one thread per rate limit (per-session,
  per-client, per-strategy).

## Sync

```rust
use nexus_rate::sync::Gcra;
use std::sync::Arc;
use std::time::{Duration, Instant};

let limiter = Arc::new(
    Gcra::builder()
        .rate(100).period(Duration::from_secs(1))
        .build().unwrap()
);

// Share across threads.
let a = Arc::clone(&limiter);
let thread = std::thread::spawn(move || {
    if a.try_acquire(1, Instant::now()) {
        // ...
    }
});
thread.join().unwrap();
```

- **Performance**: ~20-50 cycles per `try_acquire` under light
  contention. Heavy contention can spike the tail (CAS retries).
- **Thread model**: any thread can call `try_acquire`.
- **Use when**: you have a global rate limit shared by many
  workers.

## Heuristic: prefer local

Shared atomic state is almost always the wrong answer for rate
limiting. The common mistake is:

```text
One global `sync::Gcra` shared across 16 worker threads.
```

This creates a 16-way contention hotspot on a single `u64`.
Under load, CAS retries cause tail latency spikes exactly when
you can least afford them.

Better alternatives:

1. **Per-thread `local::Gcra` with divided rate.** 16 workers,
   each with a local limiter at `rate / 16`. No contention, no
   CAS.

2. **Per-client `local::Gcra`.** If each client gets its own
   thread (or hashes to a thread), each thread owns a map of
   per-client limiters. No sharing.

3. **Dedicated rate-limit thread.** One thread owns the limiter;
   other threads send requests to it via
   [`nexus-queue::mpsc`](../../nexus-queue/) and receive "go /
   no-go" back. Adds a queue hop but keeps the hot path clean.

Use `sync::*` only when the rate limit is genuinely global
(e.g., one shared API token for the whole process) and none of
the sharding alternatives apply.

## Sync implementation notes

Under the hood, `sync::Gcra` and `sync::TokenBucket` use a
single `AtomicU64` for state, updated via `compare_exchange_weak`
in a CAS loop. The loop typically exits in one iteration under
light contention. Under heavy contention, you'll see:

- More CAS retries → more cycles per operation.
- Cache line ping-pong between cores → worse tail latency.
- **Still lock-free**: no thread can permanently block others.

## When sync is fine

Low-contention cases are fine:

- 2-4 threads, infrequent limiter calls.
- Per-minute rate limits with short bursts.
- Administrative / control-plane throttling (not the hot path).

The issue is only when you're hammering a single shared limiter
from many threads at high rate. If you're calling `try_acquire`
millions of times per second across 16 threads, the contention
dominates.

## What's not available in sync

- **`sync::SlidingWindow`** doesn't exist. Implementing a
  lock-free sliding window is more pain than it's worth — if
  you need it, wrap `local::SlidingWindow` in a mutex.
- **`reconfigure`** on sync limiters is subtle: changing rate
  mid-flight is a non-atomic multi-field update. The current
  sync limiters don't expose `reconfigure` for this reason.
  Build a new limiter and swap the `Arc` if you need to change
  rates dynamically.
