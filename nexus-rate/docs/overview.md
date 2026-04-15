# Overview

Rate limiting enforces an upper bound on how fast something can
happen. In trading systems, rate limits show up in several places:

- **Exchange order rate limits.** CME: N new orders / second /
  client. Binance: X weight / minute. Kraken: token bucket with
  refill rate per API tier. Breaching these gets you banned.
- **Client rate limits.** Protecting your own system from clients
  that send too many requests.
- **Internal backpressure.** Keeping a slow downstream from being
  overwhelmed by a fast upstream.
- **Self-throttling.** Don't hammer a REST API just because you
  can — respect their documented rate.

All three algorithms in `nexus-rate` answer the same question
with the same API:

```rust
fn try_acquire(cost: u64, now: Instant) -> bool;
```

`true` means "allowed, proceed"; `false` means "rate limited, try
again later". They differ in **how** they decide, and **what
shape of burst** they allow.

## The three algorithms in one paragraph

- **GCRA** (Generic Cell Rate Algorithm): a single `u64`
  timestamp representing the theoretical next arrival time.
  Fastest and most compact. Smooth rate with configurable burst.
- **Token Bucket**: a virtual bucket that fills at a rate and
  drains on acquire. Lazy computation — no background refill.
  Same smooth rate and burst shape as GCRA, slightly different
  accounting.
- **Sliding Window**: an array of sub-window counters that
  rotate as time advances. Enforces a hard count within a rolling
  window, but allows a 2x spike at the window boundary.

## Decision: which algorithm?

See [algorithm-comparison.md](./algorithm-comparison.md) for the
full decision tree. Quick version:

| You want | Use |
|---|---|
| Smooth rate matching an exchange spec exactly | **GCRA** |
| Smooth rate, intuitive "burst capacity" mental model | **Token Bucket** |
| Exact count within a rolling window, no exceptions | **Sliding Window** |
| Fastest possible hot path | **GCRA** (~5-10 cycles) |

If you're not sure, use **GCRA**. It's the reference algorithm
from ATM network flow control, it has no pathological edge cases,
and it's the fastest.

## Local vs sync

Every algorithm has a `local` variant (`&mut self`, no atomics)
and most have a `sync` variant (`&self` + atomics, safe to share
across threads).

- **`local`**: ~5-10 cycles per `try_acquire`. Single-threaded
  rate limiters, one per strategy or one per exchange session.
- **`sync`**: ~20-50 cycles per `try_acquire` under light
  contention. One shared limiter across worker threads.

Prefer `local` when you can — shared state is the main source
of tail latency in rate limiting.

See [local-vs-sync.md](./local-vs-sync.md).

## Builder pattern

All limiters use a builder:

```rust
use nexus_rate::local::Gcra;
use std::time::Duration;

let limiter = Gcra::builder()
    .rate(100)
    .period(Duration::from_secs(1))
    .burst(20)               // optional, default 0
    // .now(custom_instant)  // optional, default Instant::now()
    .build()
    .unwrap();                // ConfigError on missing fields
# let _ = limiter;
```

`build()` returns `Result<_, ConfigError>`. Errors are detected
once, at configuration time, not on the hot path. There is no
runtime error path for `try_acquire` — it either returns `true`
or `false`.

## `cost` and weighted requests

Every call takes a `cost: u64` parameter. For standard requests,
pass `1`. For weighted requests (e.g., Binance's `weight: 50`
endpoints), pass the appropriate weight.

```rust
# use nexus_rate::local::Gcra;
# use std::time::{Duration, Instant};
# let mut limiter = Gcra::builder().rate(1200).period(Duration::from_secs(60)).build().unwrap();
let now = Instant::now();
limiter.try_acquire(1, now);    // light endpoint
limiter.try_acquire(5, now);    // medium endpoint
limiter.try_acquire(40, now);   // heavy endpoint
```

One limiter, many endpoint weights. This matches how most
exchange rate limits are actually documented.

## `now` parameter

All calls take an explicit `now: Instant`. Why not use
`Instant::now()` internally?

1. **Determinism in tests.** You can replay a sequence of
   timestamps deterministically.
2. **Clock consolidation.** If you have multiple rate limiters
   checked in sequence (e.g., per-client + global), you want
   them all to see the same `now` snapshot. Capture it once.
3. **Avoids the vDSO call on the hot path.** `Instant::now` on
   Linux is usually a vDSO, but it's still a function call. If
   you already have a timestamp from elsewhere in your pipeline,
   reuse it.

Capture `let now = Instant::now()` at the top of your hot path
and pass the same value to all limiters and statistics in that
iteration.

## Where to start

- Reading code: [gcra.md](./gcra.md) is the shortest and covers
  the core concepts.
- Picking one: [algorithm-comparison.md](./algorithm-comparison.md)
  has the decision tree.
- Real patterns: [patterns.md](./patterns.md) walks through
  three concrete trading use cases.
