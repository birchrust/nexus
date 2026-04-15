# World & Resources

The `World` is a typed singleton store. Each type gets one slot. Handlers
access resources by type -- `Res<T>` for shared read, `ResMut<T>` for
exclusive write.

## WorldBuilder

All registration happens at setup time through `WorldBuilder`. Once
`build()` is called, the set of resources is sealed.

```rust
use nexus_rt::{WorldBuilder, Resource};

#[derive(Resource, Default)]
struct GameState { score: u64 }

#[derive(Resource)]
struct Config { difficulty: u8 }

let mut wb = WorldBuilder::new();

// Register resources by type
wb.register(GameState::default());
wb.register(Config { difficulty: 3 });

// Install drivers (they register their own resources)
// let timer = wb.install_driver(TimerInstaller::new(wheel));
// let clock = wb.install_driver(RealtimeClockInstaller::default());

// Seal -- no more registration
let mut world = wb.build();
```

### WorldBuilder API

| Method | What it does |
|--------|-------------|
| `register(value)` | Register a resource, return `ResourceId`. Panics if already registered. |
| `register_default::<T>()` | Register with `T::default()`. |
| `ensure(value)` | Register if not present, return existing ID if present. |
| `ensure_default::<T>()` | Same as `ensure`, using `T::default()`. |
| `install_driver(installer)` | Consume an `Installer`, register its resources, return poller. |
| `install_plugin(plugin)` | Consume a `Plugin`, let it register resources. |
| `registry()` | Get `&Registry` for handler construction. |
| `contains::<T>()` | Check if a type is registered. |
| `len()` / `is_empty()` | Number of registered resources. |
| `build()` | Freeze into `World`. |

### Why build-time registration?

Resources are stored as individually heap-allocated `ResourceCell<T>` values
with direct `NonNull<u8>` pointers. `ResourceId` is assigned at registration
time and is stable for the World's lifetime. O(1) access, single deref.

If resources could be added at runtime, existing ResourceIds might be
invalidated. The build-time seal prevents this.

### register vs ensure

`register` panics on duplicates -- use it when double-registration is a
bug that should fail fast.

`ensure` is idempotent -- use it when multiple plugins or drivers may
independently need the same resource type:

```rust
use nexus_rt::{WorldBuilder, Resource};

#[derive(Resource, Default)]
struct SharedConfig { debug: bool }

let mut wb = WorldBuilder::new();

// First call registers it
wb.ensure(SharedConfig::default());

// Second call is a no-op -- returns existing ID
wb.ensure(SharedConfig { debug: true }); // value is DROPPED (existing kept)
```

## World

### Safe access (cold path -- HashMap lookup)

```rust
use nexus_rt::{WorldBuilder, Resource};

#[derive(Resource)]
struct Counter(u64);

let mut wb = WorldBuilder::new();
wb.register(Counter(0));
let mut world = wb.build();

// Read
let val = world.resource::<Counter>().0;

// Write
world.resource_mut::<Counter>().0 += 1;
assert_eq!(world.resource::<Counter>().0, 1);
```

### In handlers (hot path -- pre-resolved pointer)

```rust
use nexus_rt::{WorldBuilder, Res, ResMut, IntoHandler, Handler, Resource};

#[derive(Resource)]
struct Config { threshold: u64 }

#[derive(Resource)]
struct State { count: u64 }

fn my_handler(
    config: Res<Config>,         // shared read -- single deref
    mut state: ResMut<State>,    // exclusive write -- single deref
    _event: (),
) {
    if state.count < config.threshold {
        state.count += 1;
    }
}

let mut wb = WorldBuilder::new();
wb.register(Config { threshold: 100 });
wb.register(State { count: 0 });
let mut world = wb.build();

let mut handler = my_handler.into_handler(world.registry());
handler.run(&mut world, ());
assert_eq!(world.resource::<State>().count, 1);
```

### One-shot startup

`run_startup` runs a system once with full Param resolution. Useful for
post-build initialization:

```rust
use nexus_rt::{WorldBuilder, ResMut, Resource};

#[derive(Resource)]
struct Counter(u64);

fn startup(mut counter: ResMut<Counter>) {
    counter.0 = 42;
}

let mut wb = WorldBuilder::new();
wb.register(Counter(0));
let mut world = wb.build();

world.run_startup(startup);
assert_eq!(world.resource::<Counter>().0, 42);
```

### Sequence numbers

The World maintains a monotonic sequence number. Drivers advance it via
`next_sequence()`. Handlers read it via `Seq` or advance it via `SeqMut`.

```rust
use nexus_rt::{WorldBuilder, Sequence};

let mut wb = WorldBuilder::new();
let mut world = wb.build();

assert_eq!(world.current_sequence(), Sequence::ZERO);
let seq = world.next_sequence();
assert_eq!(seq, Sequence::new(1));
assert_eq!(world.current_sequence(), Sequence::new(1));
```

### Shutdown

The World owns a cooperative shutdown flag shared with `ShutdownHandle`:

```rust
use nexus_rt::WorldBuilder;

let mut world = WorldBuilder::new().build();
let handle = world.shutdown_handle();

assert!(!handle.is_shutdown());

// Handlers trigger via Shutdown param
// Event loop checks via handle
world.run(|_world| {
    // your poll loop here
    handle.shutdown(); // or triggered by a handler
});

assert!(handle.is_shutdown());
```

### run() convenience

`World::run()` loops until shutdown:

```rust
use nexus_rt::WorldBuilder;

let mut world = WorldBuilder::new().build();
let handle = world.shutdown_handle();

// Immediately shut down for this example
handle.shutdown();

world.run(|_world| {
    // poll drivers here
});
```

## Registry

`Registry` maps types to `ResourceId` pointers. It is shared between
`WorldBuilder` and `World`, and is passed to `into_handler` so handlers
can resolve their parameter state.

```rust
use nexus_rt::{WorldBuilder, Resource};

#[derive(Resource)]
struct Foo(u32);

let mut wb = WorldBuilder::new();
wb.register(Foo(42));

let reg = wb.registry();

// Check if registered
assert!(reg.contains::<Foo>());

// Get the ResourceId (panics if not registered)
let id = reg.id::<Foo>();

// Try to get (returns None if not registered)
let maybe_id = reg.try_id::<Foo>();
assert!(maybe_id.is_some());
```

### Access conflict checking

The registry validates that handlers don't have conflicting resource access
at build time:

```rust
use nexus_rt::{WorldBuilder, Res, ResMut, IntoHandler, Resource};

#[derive(Resource)]
struct Data(u64);

// This would panic at build time:
// fn bad(a: Res<Data>, b: ResMut<Data>, _e: ()) {}
// let _ = bad.into_handler(registry);
// -> "conflicting access: resource borrowed by ResMut<Data> conflicts with Res<Data>"
```

## ResourceId

`ResourceId` is a pre-resolved handle to a specific resource slot. It's
a raw pointer internally -- dereferencing it is O(1), no lookup.

Drivers use ResourceIds to avoid the type-based lookup on every poll:

```rust
use nexus_rt::{WorldBuilder, Resource};

#[derive(Resource)]
struct Clock(u64);

let mut wb = WorldBuilder::new();
let clock_id = wb.register(Clock(0));  // returns ResourceId

let mut world = wb.build();

// At poll time -- O(1) access via unsafe get_mut
unsafe {
    let clock = world.get_mut::<Clock>(clock_id);
    clock.0 = 12345;
}
assert_eq!(world.resource::<Clock>().0, 12345);
```

The `unsafe` is required because the caller must guarantee the ResourceId
is valid and the type matches. Drivers get this guarantee from the
registration during `install()`.

## Resource Trait and #[derive(Resource)]

Types stored in the World must implement the `Resource` marker trait
(`Send + 'static`). Use the derive macro:

```rust
use nexus_rt::Resource;

#[derive(Resource)]
struct OrderBook {
    bids: Vec<(f64, f64)>,
    asks: Vec<(f64, f64)>,
}
```

Without the marker trait, two modules could independently register `u64`
and silently collide. The `Resource` bound forces a newtype, making
collisions a compile error.

## new_resource! Macro

For newtype wrappers around a single inner type, `new_resource!` generates
the struct with `Resource`, `Deref`, `DerefMut`, and `From` impls:

```rust
use nexus_rt::new_resource;

new_resource!(
    /// Total bytes received across all connections.
    #[derive(Debug, Default)]
    pub TotalBytes(u64)
);

let mut counter = TotalBytes::from(0u64);
*counter += 100;
assert_eq!(*counter, 100);
```

This is equivalent to:

```rust
use nexus_rt::Resource;

#[derive(Resource, Debug, Default)]
pub struct TotalBytes(pub u64);

impl std::ops::Deref for TotalBytes { /* ... */ }
impl std::ops::DerefMut for TotalBytes { /* ... */ }
impl From<u64> for TotalBytes { /* ... */ }
```

## Resource Rules

1. **One per type** -- `register::<T>(value)` stores one `T`. Registering
   the same type twice panics. If you need multiple instances of the
   same type, use a newtype wrapper.

2. **`Send + 'static`** -- resources must be `Send` (can cross thread
   boundaries at setup) and `'static` (no borrows).

3. **No runtime addition** -- all resources must be registered before
   `build()`. This is enforced by the type system -- `World` doesn't
   expose a `register` method.

4. **Mutable access is exclusive** -- `ResMut<T>` gives `&mut T`.
   Two handlers accessing `ResMut<T>` on the same type would conflict.
   The framework checks for conflicts at build time.

## Plugins

Plugins are composable units of resource registration. Any
`FnOnce(&mut WorldBuilder)` implements `Plugin`:

```rust
use nexus_rt::{WorldBuilder, Plugin, Resource};

#[derive(Resource)]
struct MetricsConfig { enabled: bool }

#[derive(Resource)]
struct MetricsState { count: u64 }

struct MetricsPlugin { enabled: bool }

impl Plugin for MetricsPlugin {
    fn build(self, wb: &mut WorldBuilder) {
        wb.register(MetricsConfig { enabled: self.enabled });
        wb.register(MetricsState { count: 0 });
    }
}

let mut wb = WorldBuilder::new();
wb.install_plugin(MetricsPlugin { enabled: true });

// Or inline with a closure:
wb.install_plugin(|wb: &mut WorldBuilder| {
    // register additional resources
});

let world = wb.build();
assert!(world.resource::<MetricsConfig>().enabled);
```
