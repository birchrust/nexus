# Bounded vs unbounded

The timer wheel is generic over its storage backend. Two types are
pre-built:

- **`Wheel<T>`** — backed by `nexus_slab::unbounded::Slab`, grows by adding
  new chunks.
- **`BoundedWheel<T>`** — backed by `nexus_slab::bounded::Slab`, fixed
  capacity, returns `Full<T>` on exhaustion.

Both share the exact same `schedule`/`cancel`/`poll` API for the common
case — bounded just adds `try_schedule` / `try_schedule_forget` for
graceful exhaustion handling.

## When to use bounded

- You know the peak timer count at design time (you sized the order
  router for 100k orders, each has one TTL timer, so the wheel needs
  100k slots).
- You want exhaustion to be an *explicit* failure mode rather than an
  implicit allocation.
- You want stable, predictable memory use — no growth events under load.

```rust
use std::time::Instant;
use nexus_timer::BoundedWheel;

// Capacity planning: peak orders × safety factor
const PEAK_ORDERS: usize = 100_000;
const SAFETY: usize = 2;

let wheel: BoundedWheel<OrderId> =
    BoundedWheel::bounded(PEAK_ORDERS * SAFETY, Instant::now());
# #[derive(Clone, Copy)] struct OrderId(u64);
```

## When to use unbounded

- You genuinely don't know how many timers you'll have (e.g. you're
  building a general-purpose event loop).
- You want zero allocation on the hot path after warm-up, but are OK with
  occasional growth events early in the process lifetime.
- Amortized O(1) is good enough.

The unbounded slab grows by allocating *new chunks*, not by reallocating
the existing storage. Once a chunk is allocated it stays put — the
addresses of existing entries never move. This is why it's safe for a
wheel holding raw pointers to entries.

```rust
use std::time::Instant;
use nexus_timer::Wheel;

// chunk_capacity = how many entries per chunk
// Pick big enough that you rarely grow under normal load
let wheel: Wheel<u64> = Wheel::unbounded(4096, Instant::now());
```

## Growth behavior

An unbounded slab with `chunk_capacity = 4096` will:

1. Allocate its first chunk on first `alloc`.
2. Allocate a second chunk when the first fills.
3. Continue adding chunks as needed.

No chunk is ever reallocated or moved. Growth is a one-time cost amortized
over 4096 entries — negligible in steady state.

## Fallible scheduling on bounded

```rust
use std::time::{Duration, Instant};
use nexus_timer::{BoundedWheel, Full};

let now = Instant::now();
let mut wheel: BoundedWheel<u64> = BoundedWheel::bounded(8, now);

// Fill it up
for i in 0..8 {
    wheel.try_schedule_forget(now + Duration::from_millis(10), i).unwrap();
}

// 9th insert fails — you get your value back
match wheel.try_schedule_forget(now + Duration::from_millis(10), 42) {
    Ok(()) => unreachable!(),
    Err(Full(value)) => assert_eq!(value, 42),
}
```

The `Full<T>` error carries the value you tried to insert, so you don't
lose it on exhaustion.

## Memory footprint

Each wheel entry is:

- One `WheelEntry<T>` in the slab — header (~32 bytes of metadata: deadline,
  refcount, DLL pointers) plus your `T`.
- No separate allocation per timer — everything is inside the slab chunk.

For `T = u64`, each timer is ~40 bytes. A wheel holding 100k timers uses
~4 MB of slab storage plus the level slot arrays (which are tiny — 64
slots × 7 levels × pointer pair ≈ 7 KB).
