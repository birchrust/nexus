# Getting Started

nexus-rt is an event-driven runtime built on explicit poll loops, not
async/await. You own the loop. The framework provides typed resource
storage, handler dispatch, and driver integration — all monomorphized
to zero-cost abstractions.

## Dependencies

```toml
[dependencies]
nexus-rt = { version = "0.7", features = ["timer"] }
nexus-timer = "1.3"
```

The `timer` feature enables the timer driver. Other optional features:
- `smartptr` — inline handler storage (`FlatVirtual`, `FlexVirtual`)
- `mio` — mio integration for IO drivers

## Core Concepts

```
WorldBuilder          →  World
  register resources      resource storage (typed, O(1) access)
  install drivers         poll handles (timer, clock, IO)
  build                   immutable after build

Handlers              →  dispatch via World
  fn(Res<A>, ResMut<B>)   parameter resolution at build time
  named functions only     closures don't work (HRTB limitation)

Drivers               →  poll loop integration
  Installer → Poller      install at setup, poll at runtime
  Timer, Clock, IO        each driver owns its resource
```

## Minimal Example

A complete application with a timer driver and clock:

```rust
use nexus_rt::clock::{RealtimeClockInstaller, Clock};
use nexus_rt::timer::{TimerInstaller, TimerPoller, TimerWheel};
use nexus_rt::{WorldBuilder, World, Res, ResMut, IntoHandler, Handler};
use nexus_timer::WheelBuilder;
use std::time::{Duration, Instant};

// ================================================================
// 1. Define your resources (application state)
// ================================================================

struct MessageCount(u64);

// ================================================================
// 2. Define your handlers (named functions, not closures)
// ================================================================

fn on_timeout(mut count: ResMut<MessageCount>, clock: Res<Clock>) {
    count.0 += 1;
    println!(
        "timer fired! count={}, clock={}ns",
        count.0,
        clock.unix_nanos()
    );
}

// ================================================================
// 3. Setup: build the World with resources and drivers
// ================================================================

fn main() {
    let mut wb = WorldBuilder::new();

    // Register application state
    wb.register(MessageCount(0));

    // Install clock driver (registers Clock resource, returns poller)
    let mut clock = wb.install_driver(RealtimeClockInstaller::default());

    // Install timer driver (registers TimerWheel resource, returns poller)
    let wheel = WheelBuilder::default().unbounded(64).build(Instant::now());
    let mut timer = wb.install_driver(TimerInstaller::new(wheel));

    // Build the world — no more registration after this
    let mut world = wb.build();

    // Schedule a repeating timer
    let handler = on_timeout.into_handler(world.registry());
    world.resource_mut::<TimerWheel>().schedule_forget(
        Instant::now() + Duration::from_millis(100),
        Box::new(handler),
    );

    // ================================================================
    // 4. Poll loop: you own the loop
    // ================================================================

    for _ in 0..10 {
        let now = Instant::now();
        clock.sync(&mut world, now);     // sync clock resource
        timer.poll(&mut world, now);      // fire expired timers

        std::thread::sleep(Duration::from_millis(50));
    }
}
```

## What Just Happened?

1. **`WorldBuilder`** — collects resources and drivers at setup time.
   Once `build()` is called, the `World` is sealed. No more registration.

2. **Resources** — `MessageCount` and `Clock` live in the World. Handlers
   access them by type: `Res<Clock>` for read, `ResMut<MessageCount>` for write.

3. **Drivers** — `RealtimeClockInstaller` and `TimerInstaller` each
   register their resource (`Clock`, `TimerWheel`) and return a poller.
   The poller is used in the poll loop.

4. **Handler** — `on_timeout` is a plain function. Parameters are resolved
   at build time (`into_handler`). At dispatch time, the World provides
   the actual references — no HashMap lookup, just pointer dereference.

5. **Poll loop** — you call `sync()` and `poll()` explicitly. No hidden
   executor, no task queue, no async runtime. You decide what runs when.

## Why Named Functions?

Handlers must be named functions, not closures:

```rust
// ✓ Works — named function
fn my_handler(state: ResMut<MyState>, event: SomeEvent) { ... }

// ✗ Does NOT work — closure
let handler = |state: ResMut<MyState>, event: SomeEvent| { ... };
```

This is a Rust type system limitation: the parameter resolution mechanism
uses Higher-Ranked Trait Bounds (HRTBs) which interact poorly with closure
type inference. Named functions have concrete, known types that the compiler
can resolve unambiguously. See [chain-types.md](chain-types.md) for the
full explanation.

In practice, this isn't limiting. Handlers are typically standalone
functions in your codebase — the same pattern as Bevy's systems.

## Next Steps

- [World & Resources](world.md) — how the typed store works
- [Handlers](handlers.md) — parameter types, dispatch, arity
- [Pipelines & DAGs](pipelines.md) — composing processing chains
- [Drivers](drivers.md) — building your own installer/poller
- [Clock](clock.md) — realtime, test, and historical clocks
