# Timers and Time

All timer primitives share the runtime's timer wheel (mio-driven, hashed
wheel, O(1) insert/pop). `Instant` timestamps are captured once per poll
cycle and cached in TLS — read them with `event_time()` when you need a
consistent "now" without a syscall.

## `sleep` and `sleep_until`

```rust
pub fn sleep(duration: Duration) -> Sleep;
pub fn sleep_until(deadline: Instant) -> Sleep;
```

```rust
use nexus_async_rt::{Runtime, sleep};
use nexus_rt::WorldBuilder;
use std::time::Duration;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        sleep(Duration::from_millis(100)).await;
        println!("100ms elapsed");
    });
}
```

`Sleep` is a `Future<Output = ()>`. It's safe to drop before completion —
the timer is cancelled. If the future is polled with a new waker (task
migration between spawns, for instance), the waker slot updates atomically.

## `timeout` and `timeout_at`

Wraps a future with a deadline.

```rust
pub fn timeout<F: Future>(duration: Duration, future: F) -> Timeout<F>;
pub fn timeout_at<F: Future>(deadline: Instant, future: F) -> Timeout<F>;
```

`Timeout<F>` implements `Future<Output = Result<F::Output, Elapsed>>`.

```rust
use nexus_async_rt::{Runtime, timeout, Elapsed};
use nexus_rt::WorldBuilder;
use std::time::Duration;

async fn slow_http_call() -> String { todo!() }

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        match timeout(Duration::from_secs(2), slow_http_call()).await {
            Ok(body) => println!("got {} bytes", body.len()),
            Err(Elapsed) => eprintln!("http call timed out"),
        }
    });
}
```

Use `timeout_at` when you have an absolute deadline (e.g., session expiry
driven by the exchange protocol).

## `interval` and `interval_at`

Periodic ticks. Replaces manual `sleep` loops.

```rust
pub fn interval(period: Duration) -> Interval;
pub fn interval_at(start: Instant, period: Duration) -> Interval;
```

```rust
use nexus_async_rt::{Runtime, interval, MissedTickBehavior};
use nexus_rt::WorldBuilder;
use std::time::Duration;

async fn publish_heartbeat() {}

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let mut hb = interval(Duration::from_secs(30));
        hb.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            hb.tick().await;
            publish_heartbeat().await;
        }
    });
}
```

### `MissedTickBehavior`

When the task is blocked for longer than one period, the interval "misses"
ticks. You choose the recovery strategy:

- `Burst` (default): fire missed ticks back-to-back until caught up.
- `Delay`: shift the schedule forward — next tick is `now + period`.
- `Skip`: fire once now, then resume the original schedule, skipping any
  missed ticks.

For heartbeats, use `Delay` — you don't want a flurry of heartbeats after
a GC pause.

## `event_time` — Cached "now"

```rust
pub fn event_time() -> Instant;
```

Returns the `Instant` captured at the start of the current poll cycle. Free
— no syscall, no TSC read. Use it when you want every handler dispatched in
the same cycle to agree on "now" (important for deterministic replay).

```rust
use nexus_async_rt::{event_time, sleep, Runtime};
use nexus_rt::WorldBuilder;
use std::time::Duration;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let start = event_time();
        sleep(Duration::from_millis(50)).await;
        let end = event_time();
        println!("slept ~{} ms", (end - start).as_millis());
    });
}
```

## `after` and `after_delay`

Convenience: run a future, wait until a deadline, then return.

```rust
pub async fn after<F: Future>(deadline: Instant, future: F) -> F::Output;
pub async fn after_delay<F: Future>(duration: Duration, future: F) -> F::Output;
```

Unlike `timeout`, these **await the future first**, then sleep until the
deadline. Useful for rate-limiting a retry: "do the HTTP call, then make
sure we didn't finish faster than 1s".

## `yield_now`

```rust
pub fn yield_now() -> YieldNow;
```

Cooperative yield: returns `Pending` once, then `Ready(())`. The executor
picks up the next ready task before resuming yours. Use it inside CPU-bound
inner loops to avoid starving IO.

```rust
use nexus_async_rt::{yield_now, Runtime};
use nexus_rt::WorldBuilder;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        for chunk in 0..1000 {
            crunch(chunk);
            if chunk % 16 == 0 {
                yield_now().await;
            }
        }
    });
}

fn crunch(_: u32) {}
```

Most workloads don't need this — the event_interval knob on RuntimeBuilder
already forces periodic IO checks. Reach for `yield_now` only when you
have a specific starvation problem.

## Waker Update in `Sleep`

When a `Sleep` future is polled with a different waker than last time (task
moved between spawns, or wrapped in a combinator that re-polls), the timer
registration updates its waker slot atomically. You don't need to think
about this — it Just Works — but it's why `sleep` composes cleanly with
`select!`-style combinators.

## Example: Reconnect with Exponential Backoff

```rust
use nexus_async_rt::{sleep, Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;
use std::time::Duration;

async fn connect() -> std::io::Result<()> { Err(std::io::Error::other("nope")) }

async fn reconnect_loop() {
    let mut delay = Duration::from_millis(100);
    let cap = Duration::from_secs(30);
    loop {
        match connect().await {
            Ok(()) => {
                delay = Duration::from_millis(100);
                // run session...
            }
            Err(e) => {
                eprintln!("connect failed: {e}; retrying in {delay:?}");
                sleep(delay).await;
                delay = (delay * 2).min(cap);
            }
        }
    }
}

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        spawn_boxed(reconnect_loop()).await;
    });
}
```

## Example: Heartbeat With Timeout on Reads

```rust
use nexus_async_rt::{interval, timeout, Runtime};
use nexus_rt::WorldBuilder;
use std::time::Duration;

async fn send_ping() {}
async fn read_frame() -> std::io::Result<()> { Ok(()) }

async fn session() -> std::io::Result<()> {
    let mut hb = interval(Duration::from_secs(15));
    loop {
        // Read with a 30s timeout; also send heartbeats on schedule.
        tokio_like_select(&mut hb).await?;
    }
}

async fn tokio_like_select(hb: &mut nexus_async_rt::Interval) -> std::io::Result<()> {
    // Manual alternation — we don't have select! in nexus-async-rt.
    // In practice, run read and heartbeat in separate spawned tasks and
    // communicate via a channel.
    hb.tick().await;
    send_ping().await;
    timeout(Duration::from_secs(30), read_frame()).await
        .map_err(|_| std::io::Error::other("read timeout"))?
}

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async { let _ = session().await; });
}
```

For true concurrent reads + heartbeats, split into two spawned tasks joined
by a channel — see [patterns.md](patterns.md).

## See Also

- [Cancellation](cancellation.md) — combining timers with cancellation
- [Patterns](patterns.md) — heartbeat + read loop, retry with backoff
- [API Guide](API_GUIDE.md) — `event_interval` tuning
