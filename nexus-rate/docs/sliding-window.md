# Sliding Window

A rolling-window counter with configurable sub-window
granularity. Enforces `at most N events in any rolling window
of duration W`. No sync variant — see below.

## API

```rust
use nexus_rate::local::SlidingWindow;
use std::time::{Duration, Instant};

let mut limiter = SlidingWindow::builder()
    .window(Duration::from_secs(60))   // 60-second window
    .sub_windows(10)                    // 10 buckets of 6 seconds each
    .limit(1000)                        // max 1000 events in any 60s window
    .build()
    .unwrap();

let now = Instant::now();
assert!(limiter.try_acquire(1, now));
```

Builder fields:

| Method | Required | Meaning |
|---|---|---|
| `.window(dur)` | yes | Total rolling window duration |
| `.sub_windows(n)` | yes | Number of buckets (granularity) |
| `.limit(n)` | yes | Max events per window |
| `.now(Instant)` | no | Initial time base |

## How it works

The window is divided into `sub_windows` buckets, each
`window / sub_windows` long. Each bucket holds a count of
events that landed in it. On each `try_acquire`:

1. Compute which bucket `now` falls into.
2. If the current bucket is older than `window` (i.e., we've
   advanced past it), zero it and advance the "current bucket"
   pointer.
3. Zero any buckets we've skipped past.
4. Sum the counts across all buckets still in the window.
5. If `sum + cost <= limit`, accept and add `cost` to the
   current bucket. Otherwise reject.

## Granularity trade-off

More sub-windows = finer granularity = less burst at the window
boundary. Fewer sub-windows = coarser, cheaper to compute.

- **`sub_windows = 1`**: fixed window. Allows a 2x burst at the
  window boundary (fill window at the end, then refill the next
  window at the start). Usually not what you want.
- **`sub_windows = 10`**: 10% boundary burst. Common default.
- **`sub_windows = 60`**: 1/60 boundary burst. More accurate,
  slightly more expensive.

Worst-case burst is `ceil(limit * (1 + 1/sub_windows))` across
the boundary.

Rule of thumb: use 10 sub-windows unless you have a specific
reason to go higher.

## Why no `sync::SlidingWindow`?

Thread-safe sliding window requires either:

1. An atomic counter per sub-window, plus atomic rotation
   logic (hard to get right without a big mutex).
2. A mutex around the whole thing (defeats the point of
   lock-free rate limiting).

Neither is cheap enough to justify the complexity when GCRA and
token bucket already give you smooth rate limiting in the sync
case. If you truly need shared sliding-window semantics, wrap
`local::SlidingWindow` in a `Mutex` and live with the cost.

For most use cases where you *think* you need a sliding window,
you actually want GCRA — it gives smoother enforcement with
less boundary drama.

## Performance

`local::SlidingWindow::try_acquire`: ~15-30 cycles p50, scaling
linearly with `sub_windows` (the bucket sum). At 10 sub-windows,
you're summing 10 `u64` — effectively free.

## When to use

- The rate limit spec literally says "N per rolling N minutes"
  and you need exact enforcement within that window.
- You need to track the count directly (e.g., for reporting:
  "how many API calls in the last minute?").
- You don't care about slight burst at the window boundary.

Otherwise, use **GCRA** for smoother enforcement and a faster
hot path.

## Example: API call count reporting

```rust
use nexus_rate::local::SlidingWindow;
use std::time::{Duration, Instant};

let mut calls = SlidingWindow::builder()
    .window(Duration::from_secs(60))
    .sub_windows(10)
    .limit(u64::MAX)    // effectively no limit; we just want count
    .build()
    .unwrap();

let now = Instant::now();
calls.try_acquire(1, now);    // log a call
// ... continues ...
```

Using the sliding window as a counter (with `limit=u64::MAX`)
gives you a rolling event count without extra infrastructure.
For proper queue-health statistics, prefer
[`nexus-stats`](../../nexus-stats/) which has dedicated
`EventRate` and windowed metrics.
