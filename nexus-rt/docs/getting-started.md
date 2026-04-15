# Getting Started

nexus-rt is an event-driven runtime built on explicit poll loops, not
async/await. You own the loop. The framework provides typed resource
storage, handler dispatch, and driver integration -- all monomorphized
to zero-cost abstractions.

## Dependencies

```toml
[dependencies]
nexus-rt = { version = "2.1", features = ["timer"] }
```

The `timer` feature enables the timer driver. Other optional features:
- `smartptr` -- inline handler storage (`FlatVirtual`, `FlexVirtual`)
- `mio` -- mio integration for IO drivers
- `reactors` -- interest-based dispatch for per-instrument/per-strategy handlers
- `signals` -- SIGINT/SIGTERM shutdown handling (Linux only)

## Core Concepts

```
WorldBuilder          ->  World
  register resources      resource storage (typed, O(1) access)
  install drivers         poll handles (timer, clock, IO)
  build                   immutable after build

Handlers              ->  dispatch via World
  fn(Res<A>, ResMut<B>)   parameter resolution at build time
  named functions only     closures don't work (HRTB limitation)

Drivers               ->  poll loop integration
  Installer -> Poller      install at setup, poll at runtime
  Timer, Clock, IO        each driver owns its resource
```

## Example 1: Timer-Driven Application

A complete application with a timer driver and clock:

```rust
use nexus_rt::clock::{RealtimeClockInstaller, Clock};
use nexus_rt::timer::{TimerInstaller, TimerPoller, TimerWheel, Wheel};
use nexus_rt::{WorldBuilder, World, Res, ResMut, IntoHandler, Handler, Resource};
use std::time::{Duration, Instant};

// ================================================================
// 1. Define your resources (application state)
// ================================================================

#[derive(Resource)]
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
    let wheel = Wheel::unbounded(64, Instant::now());
    let mut timer = wb.install_driver(TimerInstaller::new(wheel));

    // Build the world -- no more registration after this
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

## Example 2: Pipeline-Based Event Processing

Pipelines compose steps into typed processing chains. Each step is a
named function resolved at build time -- the entire chain monomorphizes
to a single inlined function.

```rust
use nexus_rt::{
    WorldBuilder, PipelineBuilder, Res, ResMut, Handler, Resource,
    IntoHandler,
};

// ================================================================
// Domain types
// ================================================================

#[derive(Clone)]
struct Order {
    symbol: String,
    qty: u64,
    price: f64,
}

#[derive(Resource)]
struct RiskConfig {
    max_qty: u64,
}

#[derive(Resource)]
struct OrderLog {
    accepted: Vec<Order>,
}

// ================================================================
// Pipeline steps -- each does one thing
// ================================================================

fn validate(config: Res<RiskConfig>, order: Order) -> Option<Order> {
    if order.qty == 0 || order.qty > config.max_qty {
        return None; // rejected -- pipeline stops here
    }
    Some(order)
}

fn log_order(mut log: ResMut<OrderLog>, order: Order) {
    log.accepted.push(order);
}

// ================================================================
// Setup and dispatch
// ================================================================

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register(RiskConfig { max_qty: 1000 });
    wb.register(OrderLog { accepted: vec![] });
    let mut world = wb.build();
    let reg = world.registry();

    // Build pipeline: Order -> validate -> log
    let mut pipeline = PipelineBuilder::<Order>::new()
        .then(validate, reg)        // Order -> Option<Order>
        .then(log_order, reg)       // Option<Order> -> () (None skips)
        .build();

    // Dispatch events
    pipeline.run(&mut world, Order {
        symbol: "BTC".into(),
        qty: 100,
        price: 50_000.0,
    });

    pipeline.run(&mut world, Order {
        symbol: "ETH".into(),
        qty: 9999,    // exceeds max_qty -- rejected by validate
        price: 3_000.0,
    });

    assert_eq!(world.resource::<OrderLog>().accepted.len(), 1);
    assert_eq!(world.resource::<OrderLog>().accepted[0].symbol, "BTC");
}
```

## What Just Happened?

1. **`WorldBuilder`** -- collects resources and drivers at setup time.
   Once `build()` is called, the `World` is sealed. No more registration.

2. **Resources** -- `RiskConfig` and `OrderLog` live in the World. Handlers
   access them by type: `Res<RiskConfig>` for read, `ResMut<OrderLog>` for write.

3. **Drivers** -- `RealtimeClockInstaller` and `TimerInstaller` each
   register their resource (`Clock`, `TimerWheel`) and return a poller.
   The poller is used in the poll loop.

4. **Handler** -- `on_timeout` is a plain function. Parameters are resolved
   at build time (`into_handler`). At dispatch time, the World provides
   the actual references -- no HashMap lookup, just pointer dereference.

5. **Pipeline** -- `PipelineBuilder` chains steps together. Each step is
   resolved once at build time. `Option` and `Result` provide flow control --
   `None` short-circuits the remaining steps.

6. **Poll loop** -- you call `sync()` and `poll()` explicitly. No hidden
   executor, no task queue, no async runtime. You decide what runs when.

## Why Named Functions?

Handlers must be named functions, not closures:

```rust
// Works -- named function
fn my_handler(state: ResMut<MyState>, event: SomeEvent) { /* ... */ }

// Does NOT work -- closure with resource params
let handler = |state: ResMut<MyState>, event: SomeEvent| { /* ... */ };
```

This is a Rust type system limitation: the parameter resolution mechanism
uses Higher-Ranked Trait Bounds (HRTBs) which interact poorly with closure
type inference. Named functions have concrete, known types that the compiler
can resolve unambiguously. See [chain-types.md](chain-types.md) for the
full explanation.

In practice, this isn't limiting. Handlers are typically standalone
functions in your codebase -- the same pattern as Bevy's systems.

**Exception:** Arity-0 closures (no resource parameters) DO work in
pipelines and handlers:

```rust
// Works -- no Res/ResMut parameters
let h = (|event: MyEvent| { println!("{event:?}"); }).into_handler(registry);

// Works -- arity-0 pipeline step
pipeline.guard(|order: &Order| order.qty > 0, registry);
```

## Plugins and Closures

`WorldBuilder::install_plugin` accepts any `impl Plugin`, including closures.
This is convenient for one-off resource registration:

```rust
wb.install_plugin(|wb: &mut WorldBuilder| {
    wb.register(MessageCount(0));
    wb.register(Config::default());
});
```

For reusable setup, define a struct implementing `Plugin`:

```rust
struct TradingPlugin { risk_cap: u64 }

impl Plugin for TradingPlugin {
    fn build(self, wb: &mut WorldBuilder) {
        wb.register(RiskConfig { cap: self.risk_cap });
    }
}

wb.install_plugin(TradingPlugin { risk_cap: 1000 });
```

## Next Steps

- [World & Resources](world.md) -- how the typed store works
- [Handlers](handlers.md) -- parameter types, dispatch, arity
- [Callbacks](callbacks.md) -- handlers with per-instance owned state
- [Pipelines](pipelines.md) -- composing processing chains
- [DAGs](dag.md) -- fan-out and merge data-flow graphs
- [Poll Loop](poll-loop.md) -- building your event loop
- [Drivers](drivers.md) -- building your own installer/poller
- [Clock](clock.md) -- realtime, test, and historical clocks
- [Templates](templates.md) -- resolve-once, stamp-many handler factories
- [Reactors](reactors.md) -- interest-based per-instance dispatch
- [Testing](testing-guide.md) -- testing handlers and pipelines
