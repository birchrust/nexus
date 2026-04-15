# Building Your Event Loop

nexus-rt does not have an implicit executor. You own the poll loop.
This is intentional -- for latency-sensitive systems, controlling
exactly what runs when is a feature, not a limitation.

## Why No Implicit Executor

- **Deterministic ordering** -- you decide which driver to poll first
- **No scheduler overhead** -- no work queue, no task switching
- **No hidden allocation** -- no future boxing, no waker machinery
- **Latency control** -- you choose sleep vs spin, poll timeout, etc.
- **Replay-friendly** -- deterministic loops can be replayed for debugging

## Minimal Poll Loop (Single Driver)

The simplest loop polls one driver:

```rust
use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler, Resource};
use nexus_rt::timer::{TimerInstaller, TimerPoller, TimerWheel, Wheel};
use std::time::{Duration, Instant};

#[derive(Resource)]
struct Counter(u64);

fn on_tick(mut c: ResMut<Counter>, _now: Instant) {
    c.0 += 1;
}

let mut wb = WorldBuilder::new();
wb.register(Counter(0));

let wheel = Wheel::unbounded(64, Instant::now());
let mut timer = wb.install_driver(TimerInstaller::new(wheel));

let mut world = wb.build();
let shutdown = world.shutdown_handle();

// Schedule a timer
let handler = on_tick.into_handler(world.registry());
world.resource_mut::<TimerWheel>()
    .schedule_forget(Instant::now() + Duration::from_millis(100), Box::new(handler));

// Poll loop
while !shutdown.is_shutdown() {
    let now = Instant::now();
    timer.poll(&mut world, now);
    std::thread::sleep(Duration::from_millis(10));
}
```

## Multi-Driver Loop

Real systems have multiple drivers (clock, timer, IO, reactor). Each
driver has its own poller with its own `poll()` signature:

```rust
use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler, Resource};
use nexus_rt::clock::{RealtimeClockInstaller, Clock};
use nexus_rt::timer::{TimerInstaller, TimerPoller, TimerWheel, Wheel};
use std::time::{Duration, Instant};

#[derive(Resource)]
struct AppState { ticks: u64 }

fn on_timeout(mut state: ResMut<AppState>, _now: Instant) {
    state.ticks += 1;
}

let mut wb = WorldBuilder::new();
wb.register(AppState { ticks: 0 });

let mut clock = wb.install_driver(RealtimeClockInstaller::default());
let wheel = Wheel::unbounded(64, Instant::now());
let mut timer = wb.install_driver(TimerInstaller::new(wheel));

let mut world = wb.build();
let shutdown = world.shutdown_handle();

// Schedule work
let handler = on_timeout.into_handler(world.registry());
world.resource_mut::<TimerWheel>()
    .schedule_forget(Instant::now() + Duration::from_millis(50), Box::new(handler));

// Multi-driver poll loop
while !shutdown.is_shutdown() {
    let now = Instant::now();

    // 1. Sync clock -- updates the Clock resource
    clock.sync(&mut world, now);

    // 2. Poll timers -- fires expired timers
    timer.poll(&mut world, now);

    // 3. (If using mio) Poll IO
    // mio.poll(&mut world, timeout).expect("mio poll");

    // 4. (If using reactors) Dispatch reactors
    // reactors.dispatch(&mut world);

    // Sleep or spin
    std::thread::sleep(Duration::from_millis(1));
}
```

## World::run() Convenience

`World::run()` wraps the shutdown check loop:

```rust
use nexus_rt::WorldBuilder;
use std::time::{Duration, Instant};

let mut world = WorldBuilder::new().build();
let shutdown = world.shutdown_handle();

// This is equivalent to while !shutdown.is_shutdown() { ... }
// The loop exits when a handler calls Shutdown::trigger()
// or the handle is shutdown externally

// Immediately shut down for this example
shutdown.shutdown();

world.run(|world| {
    // your poll body here
    let _now = Instant::now();
    // clock.sync(world, now);
    // timer.poll(world, now);
});
```

## Sleep vs Spin

**Sleep** (`thread::sleep`) -- saves CPU, adds wakeup latency (~1-15ms
depending on OS scheduler). Good for:
- Background services not on the hot path
- Development and testing
- Systems where microsecond latency doesn't matter

**Spin** (busy loop) -- uses 100% of one core, gives sub-microsecond
response. Good for:
- Market data processing
- Order entry paths
- Any hot path where latency matters

**Hybrid** -- sleep when idle, spin when active. Use the timer wheel's
next deadline to choose:

```rust
use std::time::{Duration, Instant};

# use nexus_rt::WorldBuilder;
# let mut world = WorldBuilder::new().build();
# let shutdown = world.shutdown_handle();
# shutdown.shutdown(); // immediately exit for doc example
// Inside your poll loop:
world.run(|world| {
    let now = Instant::now();
    // timer.poll(world, now);

    // Sleep until next deadline, or spin if work is imminent
    // let timeout = timer.next_deadline(world)
    //     .map(|d| d.saturating_duration_since(now));
    //
    // match timeout {
    //     Some(d) if d > Duration::from_micros(100) => {
    //         std::thread::sleep(d.min(Duration::from_millis(10)));
    //     }
    //     _ => {} // spin -- work is imminent or no timers
    // }
});
```

## Event Ordering

Poll order determines event processing order. General principles:

1. **Clock first** -- sync the clock before anything that reads it
2. **IO before timers** -- process incoming data before scheduling
   decisions that depend on it
3. **Timers before business logic** -- fire expired timers before
   running scheduled work
4. **Reactors last** -- reactors react to changes made by handlers
   in the same frame

```rust
// Recommended poll order:
// 1. clock.sync(world, now);           // update clock
// 2. mio.poll(world, timeout);         // process IO events
// 3. timer.poll(world, now);           // fire expired timers
// 4. reactors.dispatch(world);         // dispatch woken reactors
```

## Backpressure

When handlers are slow and events pile up:

- **Timer wheel** -- timers fire in order but all at once if behind.
  The wheel catches up in a single poll call.
- **mio/IO** -- the OS buffers events. If your handler is slow,
  events queue in the kernel. Monitor read buffer sizes.
- **Reactors** -- reactors are dedup'd per frame. If a data source
  is marked 100 times before dispatch, the reactor runs once.
- **BatchPipeline** -- processes events independently. One error
  doesn't block subsequent events.

If your poll loop is consistently slow:
1. Profile -- which handler/driver is slow?
2. Move slow work off the hot path (log buffer, async task)
3. Consider batching IO reads
4. Split the event loop (separate threads for separate concerns)

## Integration with nexus-async-rt

For mixed workloads, run nexus-rt's poll loop on a dedicated thread
alongside a tokio runtime on another:

- **nexus-rt thread** -- hot path: market data, order entry, timers
- **tokio thread** -- cold path: REST APIs, monitoring, reconnection

Communication between them via channels (nexus-channel or
crossbeam). The nexus-rt loop never awaits -- it polls channels
for inbound work.

## Signal Handling

With the `signals` feature, `ShutdownHandle::enable_signals` registers
SIGINT and SIGTERM handlers that flip the shutdown flag automatically:

```rust
use nexus_rt::WorldBuilder;

let mut world = WorldBuilder::new().build();
let shutdown = world.shutdown_handle();

// Register signal handlers (Linux only)
#[cfg(feature = "signals")]
shutdown.enable_signals().expect("signal registration");

// Now Ctrl+C will trigger shutdown
// world.run(|world| { ... });
```
