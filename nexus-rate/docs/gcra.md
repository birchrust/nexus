# GCRA — Generic Cell Rate Algorithm

The reference rate limiting algorithm. Originally from ATM
network flow control, now used everywhere that cares about
smooth, predictable rate enforcement.

## API

```rust
use nexus_rate::local::Gcra;
use std::time::{Duration, Instant};

let mut limiter = Gcra::builder()
    .rate(100)                           // 100 requests...
    .period(Duration::from_secs(1))      // ...per second
    .burst(20)                            // plus up to 20 burst
    .build()
    .unwrap();

let now = Instant::now();
assert!(limiter.try_acquire(1, now));    // first call: allowed
```

Builder fields:

| Method | Required | Meaning |
|---|---|---|
| `.rate(n)` | yes | Requests allowed per period |
| `.period(dur)` | yes | Length of the rate window |
| `.burst(n)` | no (default 0) | Additional burst allowance |
| `.now(Instant)` | no (default `Instant::now()`) | Initial time base |

`build()` returns `Result<Gcra, ConfigError>`:
- `ConfigError::Missing("rate")` / `Missing("period")` — required field not set
- `ConfigError::Invalid(...)` — rate or period is zero, or period overflows u64 nanos

## How it works

GCRA tracks a single number: the **Theoretical Arrival Time**
(TAT) — the earliest time the next request could legally
arrive. On each `try_acquire`:

1. Clamp TAT up to `now` (lazy catch-up for idle periods).
2. Compute `new_tat = tat + cost * emission_interval`.
3. If `new_tat - now <= tau` (burst budget), accept and commit
   `tat = new_tat`. Otherwise reject and leave `tat` unchanged.

Where:
- `emission_interval = period / rate` — time per unit cost
- `tau = emission_interval * (burst + 1)` — burst budget in time

That's the whole algorithm. No bucket, no array, no background
thread. One `u64` of state.

## Semantics

- **Steady state**: the limiter allows `rate` requests per
  `period`. Exactly.
- **Burst**: after an idle period, up to `burst + 1` requests
  can fire back-to-back. Each one advances `tat`; when `tat -
  now > tau`, further requests are rejected until time
  advances.
- **Recovery**: the burst budget refills smoothly as time
  advances. No step function, no bucket-refill schedule.

Example: `.rate(10).period(1s).burst(5)`. You can fire up to 6
requests immediately, then one every 100 ms. If you wait 500
ms, you get 5 immediate slots back.

## Performance

`local::Gcra::try_acquire`: **~5-10 cycles p50**. A few integer
operations and one branch. No memory traffic beyond the
limiter's own state.

`sync::Gcra::try_acquire`: ~20-50 cycles p50 under light
contention (single CAS loop on a `u64`).

## `release` — rebate semantics

```rust
# use nexus_rate::local::Gcra;
# use std::time::{Duration, Instant};
# let mut limiter = Gcra::builder().rate(10).period(Duration::from_secs(1)).build().unwrap();
let now = Instant::now();
limiter.try_acquire(1, now);    // consume capacity
// ... later, exchange rejected the order or filled a partial ...
limiter.release(1, now);        // give back the capacity
```

`release` shifts TAT backward by `cost * emission_interval`,
but never earlier than `now`. This means you can't stockpile
credits by calling `release` during an idle period — the floor
is always "available right now".

Use case: exchange order rate limits that rebate on reject/fill.
See [release.md](./release.md).

## `time_until_allowed`

```rust
# use nexus_rate::local::Gcra;
# use std::time::{Duration, Instant};
# let mut limiter = Gcra::builder().rate(10).period(Duration::from_secs(1)).build().unwrap();
let now = Instant::now();
let wait = limiter.time_until_allowed(1, now);
// wait is Duration::ZERO if allowed now, else the duration to wait.
```

Useful for building backoff loops: if you get `false` from
`try_acquire`, call `time_until_allowed` to know how long to
sleep before retrying.

## `reconfigure` and `reset`

- `reconfigure(rate, period, burst)`: change rate at runtime.
  Takes effect on the next `try_acquire`. Fails with
  `ConfigError` on invalid values.
- `reset(now)`: clear state, full burst available from `now`.

Both are useful when rate limits change mid-session (e.g.,
upgrading your Binance API tier, or recovering from a pause).

## When NOT to use GCRA

- You need **exact count** within a rolling window with no
  burst allowance. Use [Sliding Window](./sliding-window.md).
- You find the burst + emission_interval math confusing. Use
  [Token Bucket](./token-bucket.md) — same result, more
  intuitive configuration.

Otherwise, GCRA is always the right starting point.
