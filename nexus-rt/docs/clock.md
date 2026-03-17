# Clock Module

High-precision time for event-driven runtimes.

## Overview

`Clock` is a World resource that holds the current time — both a monotonic
`Instant` and UTC nanoseconds since Unix epoch. Handlers read it via
`Res<Clock>`. The poll loop syncs it once per iteration using a clock
poller.

```
┌──────────────────────────────┐
│         Poll Loop            │
│                              │
│  let now = Instant::now();   │ ← one vDSO call
│  clock_poller.sync(world,now)│ ← writes Clock resource
│  timer.poll(world, now);     │
│                              │
│  ┌────────────────────────┐  │
│  │      Handlers          │  │
│  │                        │  │
│  │  fn on_event(          │  │
│  │    clock: Res<Clock>,  │  │ ← reads cached time
│  │    event: SomeEvent,   │  │
│  │  ) {                   │  │
│  │    clock.unix_nanos(); │  │ ← no syscall, field read
│  │    clock.instant();    │  │
│  │  }                     │  │
│  └────────────────────────┘  │
└──────────────────────────────┘
```

Every handler in a poll iteration sees the same timestamp. This is
intentional — see [Why one timestamp per batch?](#why-one-timestamp-per-batch)

## Three Sync Sources

Each follows the installer/poller pattern (same as the timer driver):

| Installer | Poller | Use Case |
|-----------|--------|----------|
| `RealtimeClockInstaller` | `RealtimeClockPoller` | Production — calibrated UTC |
| `TestClockInstaller` | `TestClockPoller` | Testing — deterministic time |
| `HistoricalClockInstaller` | `HistoricalClockPoller` | Replay/backtest |

### RealtimeClock — Production

Calibrates a monotonic-to-UTC offset at startup using Agrona's bracketed
sampling technique. On each `sync()`, computes UTC nanos from the cached
offset — no syscall on the hot path.

```rust
use nexus_rt::clock::{RealtimeClockInstaller, Clock};
use nexus_rt::{WorldBuilder, Res};
use std::time::Instant;

let mut wb = WorldBuilder::new();
let mut clock_poller = wb.install_driver(RealtimeClockInstaller::default());
// ... register other resources, install other drivers ...
let mut world = wb.build();

loop {
    let now = Instant::now();          // one vDSO call
    clock_poller.sync(&mut world, now); // compute + cache UTC nanos
    timer_poller.poll(&mut world, now); // timer uses the instant
    // ... dispatch handlers ...
    // handlers read Res<Clock> — cached, no syscall
}
```

#### Configuration

```rust
let installer = RealtimeClockInstaller::builder()
    .threshold(Duration::from_nanos(150))    // calibration accuracy target
    .max_retries(30)                          // calibration attempts
    .resync_interval(Duration::from_secs(1800)) // recalibrate every 30min
    .build()
    .unwrap();
```

| Parameter | Default (release) | Default (debug) | What |
|-----------|------------------|-----------------|------|
| `threshold` | 250ns | 1μs | Stop retrying when bracket gap is this tight |
| `max_retries` | 20 | 20 | Max calibration attempts |
| `resync_interval` | 1 hour | 1 hour | How often to recalibrate for NTP drift |

Minimum threshold: 100ns (release), 1μs (debug). Values below the minimum
are clamped automatically.

#### Calibration

```
For each retry:
    before  = Instant::now()
    wall    = SystemTime::now()       ← the measurement we're calibrating
    after   = Instant::now()

    gap     = after - before          ← timing variance
    midpoint = before + gap/2         ← best estimate of when wall was read
    offset  = wall_nanos - midpoint_nanos

Keep the attempt with the smallest gap. Stop early if gap < threshold.
```

Inspired by Agrona's
[`OffsetEpochNanoClock`](https://github.com/real-logic/agrona) (Real Logic).

After calibration, check `is_accurate()`:
```rust
if !clock_poller.is_accurate() {
    log::warn!(
        "clock calibration gap {:?} exceeded threshold",
        clock_poller.calibration_gap()
    );
}
```

### TestClock — Deterministic Testing

Time does not advance automatically. You control it explicitly.

```rust
use nexus_rt::clock::{TestClockInstaller, Clock};
use nexus_rt::WorldBuilder;
use std::time::Duration;

let mut wb = WorldBuilder::new();
let mut clock = wb.install_driver(TestClockInstaller::new()); // starts at epoch
let mut world = wb.build();

// Time is at 0
clock.sync(&mut world);
assert_eq!(world.resource::<Clock>().unix_nanos(), 0);

// Advance 100ms
clock.advance(Duration::from_millis(100));
clock.sync(&mut world);
assert_eq!(world.resource::<Clock>().unix_nanos(), 100_000_000);

// Jump to a specific time
clock.set_nanos(1_710_000_000_000_000_000); // some UTC timestamp
clock.sync(&mut world);
assert_eq!(world.resource::<Clock>().unix_nanos(), 1_710_000_000_000_000_000);

// Reset
clock.reset();
clock.sync(&mut world);
assert_eq!(world.resource::<Clock>().unix_nanos(), 0);
```

Start at a specific UTC time:
```rust
let clock = wb.install_driver(
    TestClockInstaller::starting_at(1_710_000_000_000_000_000)
);
```

**Key property:** `Instant` advances in lockstep with nanos. If you
`advance(100ms)`, the `clock.instant()` also advances by exactly 100ms.
This makes timer wheel interactions deterministic in tests.

### HistoricalClock — Replay

Auto-advances by a fixed step on each `sync()`. For backtesting against
historical data.

```rust
use nexus_rt::clock::{HistoricalClockInstaller, Clock};
use nexus_rt::WorldBuilder;
use std::time::Duration;

let start = 1_710_000_000_000_000_000_i128;
let end = start + 3_600_000_000_000; // +1 hour

let installer = HistoricalClockInstaller::new(
    start,
    end,
    Duration::from_millis(1), // step 1ms per sync
).unwrap();

let mut wb = WorldBuilder::new();
let mut clock = wb.install_driver(installer);
let mut world = wb.build();

while !clock.is_exhausted() {
    clock.sync(&mut world);

    let nanos = world.resource::<Clock>().unix_nanos();
    // Replay events that occurred at this timestamp
    for event in replay_events_at(nanos) {
        // ... process ...
    }
}
```

The clock writes the current position THEN advances, so the first
`sync()` writes `start_nanos` and subsequent calls step forward.

## Reading Time in Handlers

Handlers access time via `Res<Clock>` — read-only, no syscall:

```rust
fn on_market_data(clock: Res<Clock>, event: MarketDataEvent) {
    let timestamp = clock.unix_nanos();  // UTC nanos since epoch
    let mono = clock.instant();           // monotonic Instant

    // Stamp the event
    let msg = Message {
        timestamp,
        payload: event.data,
    };
}
```

The `Clock` fields are private — handlers can only read through the
accessor methods. Only clock pollers (same module) can write.

## Why One Timestamp Per Batch?

Every handler in a single poll iteration sees the same `Clock` values.
This is the standard pattern in event-driven systems:

- **Aeron** uses `CachedEpochClock` — sampled once per duty cycle
- **Game engines** compute `deltaTime` once per frame
- **Event sourcing** systems stamp the batch, not individual events

The reasons:

1. **Consistency** — all events in a batch share the same ordering timestamp
2. **Performance** — one clock read per iteration, not per event
3. **Correctness** — reading the clock between events would make later
   events appear to happen later simply because earlier events took time
   to process. That's a processing artifact, not reality.

If you need per-event arrival timestamps (e.g., NIC receive timestamps),
those come from the data source, not from the poll loop clock.

## Choosing a Sync Source

| Situation | Sync Source |
|-----------|------------|
| Production trading system | `RealtimeClockInstaller` |
| Unit tests | `TestClockInstaller` — deterministic, manually controlled |
| Integration tests with real time | `RealtimeClockInstaller` |
| Backtesting / replay | `HistoricalClockInstaller` |
| Simulation with controlled time | `TestClockInstaller` with scripted `advance()` |

The poll loop is the only code that knows which sync source is in use.
Handlers always see `Res<Clock>` regardless.
