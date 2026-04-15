# Token Bucket

A virtual bucket that holds up to `burst` tokens and refills at
a constant rate. Each request consumes tokens. If the bucket
can't cover the cost, the request is rejected.

## API

```rust
use nexus_rate::local::TokenBucket;
use std::time::{Duration, Instant};

let mut limiter = TokenBucket::builder()
    .rate(100)                          // 100 tokens...
    .period(Duration::from_secs(1))     // ...per second
    .burst(200)                         // bucket capacity (max tokens)
    .build()
    .unwrap();

let now = Instant::now();
assert!(limiter.try_acquire(1, now));
```

Builder fields:

| Method | Required | Meaning |
|---|---|---|
| `.rate(n)` | yes | Tokens added per period |
| `.period(dur)` | yes | Refill period |
| `.burst(n)` | yes | Bucket capacity (max tokens) |
| `.now(Instant)` | no | Initial time base |

Unlike GCRA, `burst` is **required** for token bucket — it's
the literal maximum number of tokens the bucket can hold.

## How it works (Folly-style lazy)

The naive token bucket implementation runs a background timer
that adds tokens to the bucket at the refill rate. That's
wasteful — you don't need to add tokens if nobody's asking.

This implementation tracks a single `zero_time` timestamp: the
moment when the bucket would have been empty if drained at the
refill rate. On `try_acquire`:

1. Compute `available_tokens = (now - zero_time) * rate /
   period`, clamped to `burst`.
2. If `available_tokens >= cost`, the request succeeds. Advance
   `zero_time` by `cost / rate` worth of time.
3. Otherwise, reject. `zero_time` is unchanged.

This is algebraically equivalent to the classic bucket
simulation, but it's O(1) with no background work. `rate /
period` is precomputed at build time as `nanos_per_token` to
avoid divisions on the hot path.

## Token bucket vs GCRA

Mathematically, these two algorithms are the same — they enforce
the same rate and the same burst shape. They differ in:

| | GCRA | Token Bucket |
|---|---|---|
| State | 1 u64 (TAT) | 1 u64 (zero_time) |
| Configuration | rate, period, burst *allowance* | rate, period, burst *capacity* |
| Mental model | "next allowed arrival time" | "tokens in a bucket" |
| `burst=0` meaning | No burst, steady rate only | Bucket holds 0 tokens (can't fire at all) |

GCRA's `burst` is additive ("N extra requests beyond steady
state"); Token Bucket's `burst` is absolute ("bucket holds at
most N tokens"). For steady-state rate X with capacity Y, you'd
configure:

- GCRA: `.rate(X).burst(Y - 1)` (because steady state gives you 1)
- Token Bucket: `.rate(X).burst(Y)`

Both give the same runtime behavior. Pick whichever mental
model matches how you think about your rate limit.

## Performance

`local::TokenBucket::try_acquire`: **~8-12 cycles p50**.
Slightly more work than GCRA because of the bucket-cap clamp,
but still negligible.

`sync::TokenBucket::try_acquire`: ~25-50 cycles p50 under light
contention.

## Release

```rust
# use nexus_rate::local::TokenBucket;
# use std::time::{Duration, Instant};
# let mut limiter = TokenBucket::builder().rate(10).period(Duration::from_secs(1)).burst(20).build().unwrap();
let now = Instant::now();
limiter.try_acquire(1, now);
limiter.release(1, now);    // refund a token
```

`release` adds tokens back to the bucket, capped at `burst`. It
will never let you exceed the configured bucket capacity. See
[release.md](./release.md).

## Reconfigure / reset

- `reconfigure(rate, period, burst)`: change limits at runtime.
- `reset(now)`: full bucket from `now`.

## When to use

- You think about rate limits as "the bucket holds N tokens".
- The rate limit spec literally talks about a bucket (Kraken's
  API docs).
- You want an explicit maximum burst capacity, separated from
  the steady-state rate.

Otherwise, use GCRA — it's slightly faster and the math is more
compact.
