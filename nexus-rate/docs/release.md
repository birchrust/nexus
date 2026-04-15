# Release / rebate

All three algorithms support `release(cost, now)`, which gives
back previously-consumed capacity. This exists for one specific
reason: **exchange rate limits that rebate on certain outcomes**.

## Why release exists

Many exchanges publish rate limits like:

> **Binance**: 1200 weight per minute. Rejected orders don't
> count against your weight budget.
>
> **CME**: 500 new orders per second. Fully-filled orders
> within the same second release their slot.
>
> **Kraken**: Decay-based rate counter. Successful orders
> increment; rejected orders don't.

If you consume capacity on submission and the exchange then
tells you "never mind, that didn't count", you want to refund
the capacity to your local limiter. Otherwise you're
systematically under the exchange's actual limit by however
often you get rebates.

`release` is the refund primitive.

## GCRA release

```rust
# use nexus_rate::local::Gcra;
# use std::time::{Duration, Instant};
# let mut gcra = Gcra::builder().rate(10).period(Duration::from_secs(1)).build().unwrap();
let now = Instant::now();
gcra.try_acquire(1, now);

// Later: exchange rejected the order.
gcra.release(1, now);
```

GCRA's `release` shifts TAT backward by `cost *
emission_interval`, but **never earlier than `now`**. You can't
stockpile credits:

- If you release during an active period, TAT moves backward
  and you effectively get the capacity back.
- If you release during a long idle period, TAT is already at
  `now`, so the release is a no-op — you already had the
  capacity.

This floor prevents you from "banking" releases during idle
time and then firing a super-burst later.

## Token Bucket release

```rust
# use nexus_rate::local::TokenBucket;
# use std::time::{Duration, Instant};
# let mut bucket = TokenBucket::builder().rate(10).period(Duration::from_secs(1)).burst(20).build().unwrap();
let now = Instant::now();
bucket.try_acquire(1, now);
bucket.release(1, now);
```

Token bucket's release adds tokens back to the bucket, capped
at `burst`. You can never exceed the bucket's configured
capacity. This is equivalent to GCRA's floor — you can't
stockpile.

## Sliding Window release

```rust
# use nexus_rate::local::SlidingWindow;
# use std::time::{Duration, Instant};
# let mut win = SlidingWindow::builder().window(Duration::from_secs(60)).sub_windows(10).limit(100).build().unwrap();
let now = Instant::now();
win.try_acquire(1, now);
win.release(1, now);
```

Sliding window's release decrements the current bucket's
counter. This is a literal count refund — the event "didn't
happen" from the window's perspective.

Note: this can produce slightly inconsistent history if release
lands in a later bucket than the original acquire (because time
has advanced). The algorithm handles this gracefully by
clamping to zero — you can't decrement below it.

## Pattern: exchange order with ACK/NACK

```rust
use nexus_rate::local::Gcra;
use std::time::{Duration, Instant};

enum OrderOutcome { Filled, Rejected, Cancelled }

let mut limiter = Gcra::builder()
    .rate(500).period(Duration::from_secs(1)).burst(50)
    .build().unwrap();

fn submit_order(limiter: &mut Gcra) -> bool {
    let now = Instant::now();
    if !limiter.try_acquire(1, now) {
        return false;  // rate limited, don't even try
    }
    // ... send to exchange ...
    true
}

fn on_outcome(limiter: &mut Gcra, outcome: OrderOutcome) {
    let now = Instant::now();
    match outcome {
        // Exchange explicitly says "this didn't count".
        OrderOutcome::Rejected => limiter.release(1, now),
        // Standard outcomes consume capacity.
        OrderOutcome::Filled | OrderOutcome::Cancelled => {}
    }
}
# let _ = (limiter, submit_order, on_outcome);
```

Read the exchange's rate limit documentation **carefully**.
Each exchange has different rules about what consumes capacity
and what rebates. Some rebates are unconditional; others only
apply to specific reject reasons. Match your `release` calls to
the exchange's actual accounting, not your intuition.

## Don't release speculatively

It's tempting to call `release` on timeout, on "the order
probably didn't go through", or on similar uncertain outcomes.
Don't:

- If the order **did** go through and you release anyway,
  you'll exceed the exchange's limit. Bans follow.
- If you're unsure whether to release, don't. The limiter will
  recover via normal time-based refill.

Only release when you have a definite signal from the exchange
that the request didn't count.

## Release as a soft recovery mechanism

If you get into a state where your local limiter thinks it's
near exhaustion but you know the exchange has more capacity
(e.g., because you've observed fills), you could technically
call `release` to resync. This works but is fragile — prefer
to build a new limiter with a fresh `reset(now)` or
`reconfigure` call, which is atomic and clean.
