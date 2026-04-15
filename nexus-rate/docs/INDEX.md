# nexus-rate docs

Rate limiting primitives: GCRA, Token Bucket, Sliding Window. All
O(1) per check, no allocation on the hot path, builder-configured
with `std::time::Duration` and `std::time::Instant`.

## Start here

- [overview.md](./overview.md) — Why rate limit, and how to pick an algorithm
- [gcra.md](./gcra.md) — Generic Cell Rate Algorithm (the default)
- [token-bucket.md](./token-bucket.md) — Folly-style lazy token bucket
- [sliding-window.md](./sliding-window.md) — Sub-windowed counter
- [local-vs-sync.md](./local-vs-sync.md) — `local` (`&mut self`) vs `sync` (`&self` + atomics)
- [release.md](./release.md) — Rebate semantics (exchange-fill credits)
- [patterns.md](./patterns.md) — Exchange order rate limits, per-client API throttling, weighted requests
- [algorithm-comparison.md](./algorithm-comparison.md) — Decision tree and side-by-side

## TL;DR

```rust
use nexus_rate::local::Gcra;
use std::time::{Duration, Instant};

let mut limiter = Gcra::builder()
    .rate(100)                          // 100 requests
    .period(Duration::from_secs(1))     // per second
    .burst(20)                          // plus 20 burst
    .build()
    .unwrap();

let now = Instant::now();
if limiter.try_acquire(1, now) {
    // allowed
}
```

All three limiters share the same `try_acquire(cost, now) -> bool`
API. All three support `release(cost, now)` for rebate/refund.

## Available combinations

| Algorithm | `local` (fastest) | `sync` (thread-safe) |
|---|---|---|
| GCRA | [`local::Gcra`] | [`sync::Gcra`] |
| Token Bucket | [`local::TokenBucket`] | [`sync::TokenBucket`] |
| Sliding Window | [`local::SlidingWindow`] | — |

Sliding Window has no sync variant because its bookkeeping
requires more atomics than it's worth. Use `local` + a mutex if
you really need shared sliding-window limiting.

## Related

- [`nexus-queue`](../../nexus-queue/) — where rate-limited messages flow
- [`nexus-stats`](../../nexus-stats/) — CoDel, event rate tracking, other queue-health metrics
