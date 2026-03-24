# Drivers

Drivers integrate external systems (timers, clocks, IO) into the poll loop.
They follow the **installer/poller** pattern: consumed at setup, return a
handle for runtime polling.

## The Pattern

```
Setup time:                          Runtime:

┌──────────────┐                    ┌──────────────┐
│  Installer   │                    │   Poller     │
│              │  install()         │              │
│  config,     │ ──────────────►    │  ResourceId, │
│  resources   │  registers         │  state       │
│              │  resources in      │              │
│  (consumed)  │  WorldBuilder      │  poll/sync   │
└──────────────┘                    └──────────────┘
```

The installer carries configuration and registers resources into the
WorldBuilder. It's consumed by `install()`, which returns the poller.
The poller holds pre-resolved `ResourceId`s and polls/syncs on each
loop iteration.

## Installer Trait

```rust
pub trait Installer {
    type Poller;

    /// Consume the installer, register resources, return the poller.
    fn install(self, world: &mut WorldBuilder) -> Self::Poller;
}
```

`install()` takes `self` by value — the installer is consumed. The
returned `Poller` is the runtime handle.

## Built-In Drivers

### Timer Driver

```rust
use nexus_rt::timer::{TimerInstaller, TimerPoller, TimerWheel};
use nexus_timer::WheelBuilder;

// Setup
let wheel = WheelBuilder::default().unbounded(64).build(Instant::now());
let mut timer: TimerPoller = wb.install_driver(TimerInstaller::new(wheel));

// Poll loop
timer.poll(&mut world, now);  // fires expired timers, dispatches handlers
```

The timer driver registers a `TimerWheel` resource. Handlers can schedule
timers via `ResMut<TimerWheel>`.

### Clock Driver

```rust
use nexus_rt::clock::{RealtimeClockInstaller, Clock};

// Setup
let mut clock = wb.install_driver(RealtimeClockInstaller::default());

// Poll loop
clock.sync(&mut world, now);  // writes Clock resource
```

Three variants: `RealtimeClockInstaller` (production), `TestClockInstaller`
(testing), `HistoricalClockInstaller` (replay). See [clock.md](clock.md).

## Building Your Own Driver

A driver for a hypothetical market data feed:

```rust
use nexus_rt::{Installer, Resource, ResourceId, WorldBuilder, World};

// The resource handlers will access
#[derive(Resource)]
pub struct MarketData {
    pub last_price: f64,
    pub sequence: u64,
}

// Installer — carries connection config
pub struct MarketDataInstaller {
    feed_url: String,
}

impl MarketDataInstaller {
    pub fn new(feed_url: String) -> Self {
        Self { feed_url }
    }
}

// Poller — holds ResourceId + connection state
pub struct MarketDataPoller {
    data_id: ResourceId,
    // connection: FeedConnection,  // your IO handle
}

impl Installer for MarketDataInstaller {
    type Poller = MarketDataPoller;

    fn install(self, world: &mut WorldBuilder) -> MarketDataPoller {
        let data_id = world.register(MarketData {
            last_price: 0.0,
            sequence: 0,
        });

        // Connect to feed, set up IO...

        MarketDataPoller {
            data_id,
            // connection: connect(self.feed_url),
        }
    }
}

impl MarketDataPoller {
    pub fn poll(&mut self, world: &mut World) {
        // Read from connection, update resource
        let data = unsafe { world.get_mut::<MarketData>(self.data_id) };
        // data.last_price = ...;
        // data.sequence += 1;
    }
}
```

Usage:
```rust
let mut wb = WorldBuilder::new();
let mut md = wb.install_driver(MarketDataInstaller::new("wss://...".into()));
let mut world = wb.build();

loop {
    let now = Instant::now();
    clock.sync(&mut world, now);
    md.poll(&mut world);           // updates MarketData resource
    timer.poll(&mut world, now);
}
```

Handlers access the market data naturally:
```rust
fn on_timer(data: Res<MarketData>, clock: Res<Clock>) {
    println!("price={} seq={} at {}", data.last_price, data.sequence, clock.unix_nanos());
}
```

## Registering Handlers in Drivers (Rust 2024)

If your driver accepts handlers via a method that takes `&Registry` and
stores an `impl Handler<E>`, the Rust 2024 `+ use<...>` capture rule
applies to any factory function the user passes handlers through.

For example, if a user builds a pipeline in a factory function and
registers it with your driver:

```rust
fn on_message<C: Config>(
    reg: &Registry,
) -> impl for<'a> Handler<Msg<'a>> + use<C> {
    PipelineBuilder::<Msg<'_>>::new()
        .then(decode::<C>, reg)
        .dispatch(process::<C>.into_handler(reg))
        .build()
}

// Usage:
let handler = on_message::<MyConfig>(wb.registry());
driver.register(handler);   // works — registry borrow ended
let world = wb.build();      // works — no outstanding borrows
```

Without `+ use<C>` on `on_message`, the `wb.registry()` borrow would
be held, and `wb.build()` would fail. Document this in your driver's
handler registration examples.

See [Handlers — Returning from Functions](handlers.md#returning-handlers-from-functions-rust-2024).

## Design Principles

1. **Installer is consumed** — configuration doesn't persist. The poller
   is the only runtime artifact.

2. **Pre-resolved ResourceIds** — the poller resolves once at install,
   dereferences at O(1) on every poll.

3. **Poller owns its poll signature** — `sync(&mut self, &mut World, Instant)`
   for the clock, `poll(&mut self, &mut World, Instant)` for the timer,
   `poll(&mut self, &mut World)` for market data. No forced common signature.

4. **Drivers don't know about each other** — each driver registers its
   resource and polls independently. The poll loop is the only place
   where ordering is decided.
