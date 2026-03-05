# nexus-rt

Single-threaded, event-driven runtime primitives with pre-resolved dispatch.

`nexus-rt` provides the building blocks for constructing runtimes where
user code runs as handlers dispatched over shared state. It is **not** an
async runtime — there is no task scheduler, no work stealing, no `Future`
polling. Your `main()` is the executor.

## Design

`nexus-rt` is heavily inspired by [Bevy ECS](https://bevyengine.org/).
Handlers as plain functions, `Param` for declarative dependency
injection, `Res<T>` / `ResMut<T>` wrappers, change detection via
sequence stamps, the `Plugin` trait for composable registration — these
are Bevy's ideas, and in many cases the implementation follows Bevy's
patterns closely (including the HRTB double-bound trick that makes
`IntoHandler` work). Credit where it's due: Bevy's system model is
an excellent piece of API design.

Where `nexus-rt` diverges is the target workload. Bevy is built for
simulation: many entities mutated per frame, parallel schedules,
component queries over archetypes. `nexus-rt` is built for event-driven
systems: singleton resources, sequential dispatch, and monotonic sequence
numbers instead of frame ticks. There are no entities, no components,
no archetypes — just a typed resource store where each event advances
a sequence counter and causality is tracked per-resource.

The result is a much smaller surface area tuned for low-latency event
processing rather than game-world state management.

## Architecture

```text
                        Build Time                          Dispatch Time
                   ┌──────────────────┐               ┌──────────────────────┐
                   │                  │               │                      │
                   │  WorldBuilder    │               │       World          │
                   │                  │               │                      │
                   │  ┌────────────┐  │    build()    │  ┌────────────────┐  │
                   │  │ Registry   │──┼──────────────►│  │ ResourceSlot[] │  │
                   │  │ TypeId→Idx │  │               │  │ ptr+changed_at │  │
                   │  └────────────┘  │               │  └───────┬────────┘  │
                   │                  │               │          │           │
                   │  install_plugin  │               │    get(id) ~3 cyc   │
                   │  install_driver  │               │          │           │
                   └──────────────────┘               └──────────┼───────────┘
                          │                                      │
                          │ returns Handle                       │
                          ▼                                      ▼
                   ┌──────────────────┐               ┌──────────────────────┐
                   │  Driver Handle   │               │  poll(&mut World)    │
                   │                  │               │                      │
                   │  Pre-resolved    │──────────────►│  1. next_sequence()  │
                   │  ResourceIds     │               │  2. get resources    │
                   │                  │               │  3. poll IO source   │
                   │  Owns pipeline   │               │  4. dispatch events  │
                   │  or handlers     │               │     via pipeline     │
                   └──────────────────┘               └──────────────────────┘
```

### Flow

1. **Build** — Register resources into `WorldBuilder`. Install plugins
   (fire-and-forget resource registration) and drivers (returns a handle).
2. **Freeze** — `builder.build()` produces an immutable `World`. All
   `ResourceId` values are dense indices, valid for the lifetime of the World.
3. **Poll loop** — Your code calls `driver.poll(&mut world)` in a loop.
   Each driver owns its event lifecycle internally: poll IO, decode events,
   dispatch through its pipeline, mutate world state.
4. **Sequence** — Each event gets a monotonic sequence number via
   `world.next_sequence()`. Change detection is causal: `changed_at` records
   which event caused the mutation.

### Dispatch tiers

| Tier | Purpose | Overhead |
|------|---------|----------|
| **Pipeline** | Pre-resolved step chains inside drivers. The workhorse. | ~2 cycles p50 |
| **Callback** | Dynamic per-instance context + pre-resolved params. | ~2 cycles p50 |
| **Handler** | `Box<dyn Handler<E>>` for type-erased dispatch. | ~2 cycles p50 |
| **Template** | Pre-resolved handler stamping for re-registration. | ~1 cycle p50 (generate) |
| **DAG** | Monomorphized fan-out / merge data-flow graphs. | ~1-3 cycles p50 |
| **FanOut / Broadcast** | Static or dynamic fan-out by reference. | ~2 cycles p50 |

All tiers resolve `Param` state at build time. Dispatch-time cost is
a bounds-checked index into a `Vec` — no hashing, no searching.

## Quick Start

```rust
use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler};

let mut builder = WorldBuilder::new();
builder.register::<u64>(0);
let mut world = builder.build();

fn tick(mut counter: ResMut<u64>, event: u32) {
    *counter += event as u64;
}

let mut handler = tick.into_handler(world.registry_mut());

handler.run(&mut world, 10u32);

assert_eq!(*world.resource::<u64>(), 10);
```

## Driver Model

Drivers are event sources. The `Driver` trait handles installation;
the returned handle is a concrete type with its own `poll()` signature.

```rust
use nexus_rt::{Driver, WorldBuilder, World, ResourceId};

struct TimerInstaller { resolution_ms: u64 }
struct TimerHandle { timers_id: ResourceId }

impl Driver for TimerInstaller {
    type Handle = TimerHandle;

    fn install(self, world: &mut WorldBuilder) -> TimerHandle {
        world.register(Vec::<u64>::new());
        // ... register other resources ...
        let timers_id = world.registry().id::<Vec<u64>>();
        TimerHandle { timers_id }
    }
}

// Handle defines its own poll signature — NOT a trait method.
impl TimerHandle {
    fn poll(&mut self, world: &mut World, now_ms: u64) {
        // get resources via pre-resolved IDs, fire expired timers
    }
}
```

The executor is your `main()`:

```rust
let mut wb = WorldBuilder::new();
wb.install_plugin(TradingPlugin { /* config */ });
let timer = wb.install_driver(TimerInstaller { resolution_ms: 100 });
let io = wb.install_driver(IoInstaller::new());
let mut world = wb.build();

loop {
    let now = std::time::Instant::now();
    timer.poll(&mut world, now);
    io.poll(&mut world);
}
```

## Features

### World — typed singleton store

Type-erased resource storage with dense `ResourceId` indexing. ~3 cycles
per dispatch-time access. Frozen after build — no inserts, no removes.

*(Bevy analogy: `World`, but singletons only — no entities, no components,
no archetypes.)*

### Res / ResMut — resource parameters

Declare resource dependencies in function signatures. `Res<T>` for shared
reads, `ResMut<T>` for exclusive writes. `ResMut` stamps `changed_at` on
`DerefMut` — the act of writing is the change signal.

*(Bevy analogy: `Res<T>` and `ResMut<T>`, same names and semantics.)*

```rust
fn process(config: Res<Config>, mut state: ResMut<State>, event: Event) {
    // config is read-only, state is read-write
}
```

### Optional resources

`Option<Res<T>>` and `Option<ResMut<T>>` resolve to `None` if the type
was not registered, rather than panicking at build time. Useful for
handlers that can operate with or without a particular resource.

```rust
fn maybe_log(logger: Option<Res<Logger>>, event: u32) {
    if let Some(log) = logger {
        log.info(event);
    }
}
```

### Param — build-time / dispatch-time resolution

The `Param` trait is the mechanism behind `Res<T>`, `ResMut<T>`,
`Local<T>`, and all other handler parameters. Two-phase resolution:

1. **Build time** — `Param::init(registry)` resolves opaque state (e.g.
   a `ResourceId`) and panics if the required type isn't registered.
2. **Dispatch time** — `Param::fetch(world, state)` uses the cached state
   to produce a reference in ~3 cycles.

*(Bevy analogy: `SystemParam`.)*

Built-in impls: `Res<T>`, `ResMut<T>`, `Option<Res<T>>`,
`Option<ResMut<T>>`, `Local<T>`, `RegistryRef`, `()`, and tuples up to
8 params.

### Handler / IntoHandler — fn-to-handler conversion

`IntoHandler` converts a plain `fn` into a `Handler` trait object.
Event `E` is always the last parameter; everything before it is resolved
as `Param` from a `Registry`.

*(Bevy analogy: `IntoSystem` / `System` trait.)*

```rust
fn tick(counter: Res<u64>, mut flag: ResMut<bool>, event: u32) {
    if event > 0 {
        *flag = true;
    }
}

let mut handler = tick.into_handler(registry);
handler.run(&mut world, 10u32);
```

Named functions only — closures do not work with `IntoHandler` due to
Rust's HRTB inference limitations with GATs (same limitation as Bevy).

### Pipeline — pre-resolved processing chains

Typed composition chains where each step is a named function with
`Param` dependencies resolved at build time.

```rust
let mut pipeline = PipelineStart::<Order>::new()
    .then(validate, registry)       // Order → Result<Order, Error>
    .and_then(enrich, registry)      // Order → Result<Order, Error>
    .catch(log_error, registry)      // Error → () (side effect)
    .map(submit, registry)           // Order → Receipt
    .build();                        // → Pipeline<Order, _> (concrete)

pipeline.run(&mut world, order);
```

Option and Result combinators (`.map()`, `.and_then()`, `.catch()`,
`.filter()`, `.unwrap_or()`, etc.) enable typed flow control without
runtime overhead. `Pipeline` implements `Handler<In>`, so it can be
boxed or stored alongside other handlers.

### Batch pipeline — per-item processing over a buffer

`build_batch(capacity)` produces a `BatchPipeline` that owns a
pre-allocated input buffer. Each item flows through the same chain
independently — errors are handled per-item, not per-batch.

```rust
let mut batch = PipelineStart::<Order>::new()
    .then(validate, registry)       // Order → Result<Order, Error>
    .catch(log_error, registry)      // handle error, continue batch
    .map(enrich, registry)           // runs for valid items only
    .then(submit, registry)
    .build_batch(1024);

// Driver fills input buffer
batch.input_mut().extend_from_slice(&orders);
batch.run(&mut world);  // drains buffer, no allocation
```

No intermediate buffers between steps. The compiler monomorphizes
the per-item chain identically to the single-event pipeline.

### DAG Pipeline — fan-out, merge, and data-flow graphs

`DagStart` builds a monomorphized data-flow graph where topology is
encoded in the type system. After monomorphization the entire DAG is
a single flat function — all values are stack locals, no arena, no
vtable dispatch.

```rust
use nexus_rt::{WorldBuilder, ResMut, Handler};
use nexus_rt::dag::DagStart;

let mut wb = WorldBuilder::new();
wb.register::<u64>(0);
let mut world = wb.build();
let reg = world.registry();

fn decode(raw: u32) -> u64 { raw as u64 * 2 }
fn add_one(val: &u64) -> u64 { *val + 1 }
fn mul3(val: &u64) -> u64 { *val * 3 }
fn merge_add(a: &u64, b: &u64) -> u64 { *a + *b }
fn store(mut out: ResMut<u64>, val: &u64) { *out = *val; }

let mut dag = DagStart::<u32>::new()
    .root(decode, reg)
    .fork()
    .arm(|a| a.then(add_one, reg))
    .arm(|b| b.then(mul3, reg))
    .merge(merge_add, reg)
    .then(store, reg)
    .build();

dag.run(&mut world, 5u32);
// root: 10, arm_a: 11, arm_b: 30, merge: 41
assert_eq!(*world.resource::<u64>(), 41);
```

Fan-out arms borrow the fork output by reference — no `Clone` needed.
Option and Result combinators (`.map()`, `.and_then()`, `.catch()`,
etc.) work on both the main chain and within arms. `Dag` implements
`Handler<E>`, so it can be boxed or stored alongside other handlers.

For linear chains without fan-out, prefer
[Pipeline](#pipeline--pre-resolved-processing-chains).

### FanOut / Broadcast — handler-level fan-out

`FanOut` dispatches the same event by reference to a fixed set of
handlers. Zero allocation, concrete types, monomorphizes to direct
calls. Macro-generated for arities 2-8.

`Broadcast` is the dynamic variant — stores `Vec<Box<dyn RefHandler<E>>>`
for runtime-determined handler counts.

```rust
use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler};
use nexus_rt::{fan_out, Broadcast, Cloned};

fn write_a(mut sink: ResMut<u64>, event: &u32) { *sink += *event as u64; }
fn write_b(mut sink: ResMut<i64>, event: &u32) { *sink += *event as i64; }

let mut builder = WorldBuilder::new();
builder.register::<u64>(0);
builder.register::<i64>(0);
let mut world = builder.build();

let h1 = write_a.into_handler(world.registry());
let h2 = write_b.into_handler(world.registry());
let mut fan = fan_out!(h1, h2);
fan.run(&mut world, 5u32);
assert_eq!(*world.resource::<u64>(), 5);
assert_eq!(*world.resource::<i64>(), 5);
```

Handlers inside combinators receive `&E`. Use `Cloned` or `Owned`
adapters for handlers that expect owned events.

For fan-out with merge (data flowing back together), use
[DagStart](#dag-pipeline--fan-out-merge-and-data-flow-graphs).

### Change detection

Each resource tracks a `changed_at` sequence number. Drivers call
`world.next_sequence()` before each event dispatch to advance the
sequence counter.

```rust
// Read side — check if a resource was modified this sequence
fn observer(val: Res<u64>, _event: ()) {
    if val.is_changed() {
        // val was written during the current sequence
    }
}

// Write side — ResMut stamps changed_at on DerefMut automatically
fn writer(mut val: ResMut<u64>, _event: ()) {
    *val = 42; // stamps changed_at = current_sequence
}

```

### Plugin — composable registration

Fire-and-forget resource registration units. Consumed by `WorldBuilder`.

*(Bevy analogy: `Plugin`.)*

```rust
struct MyPlugin { /* config */ }

impl Plugin for MyPlugin {
    fn build(self, world: &mut WorldBuilder) {
        world.register(MyState::new());
        world.register(MyConfig::default());
    }
}

wb.install_plugin(MyPlugin { /* ... */ });
```

### Local — per-handler state

`Local<T>` is state stored inside the handler instance, not in World.
Initialized with `Default::default()` at handler creation time. Each
handler instance gets its own independent copy — two handlers created
from the same function have separate `Local` values.

*(Bevy analogy: `Local<T>`.)*

```rust
fn count_events(mut count: Local<u64>, mut total: ResMut<u64>, _event: u32) {
    *count += 1;
    *total = *count;
}

let mut handler_a = count_events.into_handler(registry);
let mut handler_b = count_events.into_handler(registry);

handler_a.run(&mut world, 0);  // handler_a local=1
handler_b.run(&mut world, 0);  // handler_b local=1 (independent)
handler_a.run(&mut world, 0);  // handler_a local=2
```

### Callback — context-owning handlers

`Callback<C, F, Params>` is a handler with per-instance owned context.
Use it when each handler instance needs private state that isn't shared
via World — per-timer metadata, per-connection codec state, protocol
state machines.

Convention: `fn handler(ctx: &mut C, params..., event: E)` — context
first, `Param`-resolved resources in the middle, event last.

```rust
struct TimerCtx { order_id: u64, fires: u64 }

fn on_timeout(ctx: &mut TimerCtx, mut counter: ResMut<u64>, _event: ()) {
    ctx.fires += 1;
    *counter += ctx.order_id;
}

let mut cb = on_timeout.into_callback(
    TimerCtx { order_id: 42, fires: 0 },
    registry,
);
cb.run(&mut world, ());

// Context is pub — accessible outside dispatch
assert_eq!(cb.ctx.fires, 1);
```

### HandlerTemplate / CallbackTemplate — resolve once, stamp many

When handlers are created repeatedly on the hot path — IO readiness
re-registration, timer rescheduling, connection accept loops — each
`into_handler(registry)` call pays for HashMap lookups to resolve the
same `ResourceId` values every time.

Templates resolve parameters once, then `generate()` stamps out
handlers by copying pre-resolved state. ~1 cycle vs ~20-70 cycles
for `into_handler`.

A [`Blueprint`] declares the event and parameter types. The template
resolves them against the registry once:

```rust
use nexus_rt::{WorldBuilder, ResMut, Handler};
use nexus_rt::template::{Blueprint, HandlerTemplate, CallbackTemplate, CallbackBlueprint};

struct OnTick;
impl Blueprint for OnTick {
    type Event = u32;
    type Params = (ResMut<'static, u64>,);
}

fn tick(mut counter: ResMut<u64>, event: u32) {
    *counter += event as u64;
}

let mut builder = WorldBuilder::new();
builder.register::<u64>(0);
let mut world = builder.build();

let template = HandlerTemplate::<OnTick>::new(tick, world.registry());

// Stamp out handlers — no HashMap lookups, just Copy.
let mut h1 = template.generate();
let mut h2 = template.generate();

h1.run(&mut world, 10);
h2.run(&mut world, 5);
assert_eq!(*world.resource::<u64>(), 15);
```

For context-owning handlers, `CallbackTemplate` works the same way —
each `generate(ctx)` takes an owned context value:

```rust
struct TimerCtx { order_id: u64 }

struct OnTimeout;
impl Blueprint for OnTimeout {
    type Event = ();
    type Params = (ResMut<'static, u64>,);
}
impl CallbackBlueprint for OnTimeout {
    type Context = TimerCtx;
}

fn on_timeout(ctx: &mut TimerCtx, mut counter: ResMut<u64>, _event: ()) {
    *counter += ctx.order_id;
}

# let mut builder = WorldBuilder::new();
# builder.register::<u64>(0);
# let mut world = builder.build();
let cb_template = CallbackTemplate::<OnTimeout>::new(on_timeout, world.registry());
let mut cb = cb_template.generate(TimerCtx { order_id: 42 });
cb.run(&mut world, ());
assert_eq!(*world.resource::<u64>(), 42);
```

Convenience macros reduce Blueprint boilerplate:

```rust
use nexus_rt::handler_blueprint;
handler_blueprint!(OnTick, Event = u32, Params = (ResMut<'static, u64>,));
```

**Constraints:**
- `P::State: Copy` — excludes `Local<T>` with non-Copy state
  (incompatible with template stamping). All World-backed params
  (`Res`, `ResMut`, `Option` variants) have `State = ResourceId`
  which is `Copy`.
- Zero-sized callables only — named functions and captureless closures.
  Capturing closures and function pointers are rejected at compile time.

### RegistryRef — runtime handler creation

`RegistryRef` is a `Param` that provides read-only access to the
`Registry` during handler dispatch. Enables handlers to create new
handlers at runtime via `IntoHandler::into_handler` or
`IntoCallback::into_callback`.

```rust
fn spawner(reg: RegistryRef, _event: ()) {
    let handler = some_fn.into_handler(&reg);
    // store handler somewhere...
}
```

### Installer — event source installation

`Installer` is the install-time trait for event sources. The installer
registers its resources into `WorldBuilder` and returns a concrete
poller whose `poll()` method drives the event lifecycle. See the
[Driver Model](#driver-model) section for the full pattern.

### Timer driver (feature: `timer`)

Integrates `nexus_timer::Wheel` as a driver. `TimerInstaller` registers
the wheel into `WorldBuilder` and returns a `TimerPoller`.

- `TimerPoller::poll(world, now)` drains expired timers and fires handlers
- Handlers reschedule themselves via `ResMut<TimerWheel<S>>`
- `Periodic` helper for recurring timers
- Inline storage variants behind `smartptr` feature: `InlineTimerWheel`,
  `FlexTimerWheel`

### Mio driver (feature: `mio`)

Integrates `mio` as an IO driver. `MioInstaller` registers the
`MioDriver` (wrapping `mio::Poll` + handler slab) and returns a
`MioPoller`.

- `MioPoller::poll(world, timeout)` polls for readiness and fires handlers
- Move-out-fire pattern: handler is removed from slab, fired, and must
  re-insert itself to receive more events
- Stale tokens (already removed) are silently skipped
- Inline storage variants behind `smartptr` feature: `InlineMio`, `FlexMio`

### Virtual / FlatVirtual / FlexVirtual — storage aliases

Type aliases for type-erased handler storage:

```rust
use nexus_rt::Virtual;

// Heap-allocated (default)
let handler: Virtual<Event> = Box::new(my_handler.into_handler(registry));

// Behind "smartptr" feature — inline storage via nexus-smartptr
// use nexus_rt::FlatVirtual;
// let handler: FlatVirtual<Event> = flat!(my_handler.into_handler(registry));
```

`Virtual<E>` for heap-allocated. `FlatVirtual<E>` for fixed inline
(panics if handler doesn't fit). `FlexVirtual<E>` for inline with
heap fallback.

## When to Use What

| Situation | Use | Why |
|-----------|-----|-----|
| One-time setup, test harness | `IntoHandler` / `IntoCallback` | Simple, direct. Construction cost paid once. |
| Pipeline steps inside a driver | `Pipeline` / `BatchPipeline` | Zero-cost monomorphized chains, typed flow control. |
| IO re-registration (accept, echo) | `HandlerTemplate` / `CallbackTemplate` | Handler recreated every event — template eliminates per-event HashMap lookups. |
| Timer rescheduling | `HandlerTemplate` / `CallbackTemplate` | Same pattern — recurring handlers should not pay construction cost repeatedly. |
| Type-erased handler storage | `Box<dyn Handler<E>>` / `Virtual<E>` | When you need heterogeneous collections (driver slabs, timer wheels). |
| Per-instance private state | `Callback` (via `IntoCallback`) or `CallbackTemplate` | Context-owning handlers for connection state, timer metadata, etc. |
| Composable resource registration | `Plugin` | Fire-and-forget, consumed by `WorldBuilder`. |
| Fan-out with merge | `DagStart` → `Dag` | Monomorphized data-flow graph. Zero vtable, all stack locals. |
| Static fan-out (known count) | `FanOut` / `fan_out!` | Dispatch `&E` to N handlers. Zero allocation, concrete types. |
| Dynamic fan-out (runtime count) | `Broadcast` | `Vec<Box<dyn RefHandler>>`. One heap alloc per handler, zero clones. |

**Rule of thumb:** If a handler is created once, use `IntoHandler`. If
it's created repeatedly on every event (move-out-fire pattern), use a
template. For data that must fan out and merge back, use `DagStart`.
For fire-and-forget fan-out, use `FanOut` (static) or `Broadcast`
(dynamic).

## Performance

All measurements in CPU cycles, pinned to a single core with turbo
boost disabled.

### Dispatch (hot path)

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| Baseline hand-written fn | 2 | 3 | 4 |
| 3-stage pipeline (bare) | 2 | 2 | 4 |
| 3-stage pipeline (Res\<T\>) | 2 | 3 | 5 |
| Handler + Res\<T\> (read) | 2 | 4 | 5 |
| Handler + ResMut\<T\> (write) | 3 | 8 | 8 |
| Box\<dyn Handler\> | 2 | 9 | 9 |

Pipeline dispatch matches hand-written code — zero-cost abstraction
confirmed.

### Batch throughput

Total cycles for 100 items through the same pipeline chain.

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| Batch bare (100 items) | 130 | 264 | 534 |
| Linear bare (100 calls) | 196 | 512 | 528 |
| Batch Res\<T\> (100 items) | 390 | 466 | 612 |
| Linear Res\<T\> (100 calls) | 406 | 550 | 720 |

Batch dispatch amortizes to ~1.3 cycles/item for compute-heavy chains
(~1.5x faster than individual calls).

### Construction (cold path)

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| into_handler (1 param) | 21 | 30 | 79 |
| into_handler (4 params) | 45 | 86 | 147 |
| into_handler (8 params) | 93 | 156 | 221 |
| .then() (2 params) | 28 | 48 | 96 |

Construction cost is paid once at build time, never on the dispatch
hot path.

### Template generation (hot path handler creation)

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| generate (1 param) | 1 | 1 | 2 |
| generate (2 params) | 1 | 1 | 2 |
| generate (4 params) | 1 | 1 | 1 |
| generate (8 params) | 1 | 1 | 1 |
| generate callback (2 params) | 1 | 2 | 2 |
| generate callback (4 params) | 1 | 1 | 1 |

`generate()` copies pre-resolved `ResourceId` values — flat 1 cycle
at every arity. Compare with `into_handler` above: 24-70x faster for
handlers created on every event (IO re-registration, timer rescheduling).

### Running benchmarks

```bash
taskset -c 0 cargo run --release -p nexus-rt --example perf_pipeline
taskset -c 0 cargo run --release -p nexus-rt --example perf_construction
taskset -c 0 cargo run --release -p nexus-rt --example perf_template
taskset -c 0 cargo run --release -p nexus-rt --example mio_timer --features mio,timer
```

## Limitations

### Named functions only

`IntoHandler`, `IntoCallback`, and `IntoStep` (arity 1+) require named
`fn` items — closures do not work due to Rust's HRTB inference limitations
with GATs. This is the same limitation as Bevy's system registration.

Arity-0 pipeline steps (no `Param`) do accept closures:
```rust
// Works — arity-0 closure
pipeline.then(|x: u32| x * 2, registry);

// Does NOT work — arity-1 closure with Param
// pipeline.then(|config: Res<Config>, x: u32| x, registry);

// Works — named function
fn transform(config: Res<Config>, x: u32) -> u32 { x + *config as u32 }
pipeline.then(transform, registry);
```

### Single-threaded

`World` is `!Sync` by design. All dispatch is single-threaded, sequential.
This is intentional — for latency-sensitive event processing, eliminating
coordination overhead matters more than parallelism.

### Frozen after build

No resources can be added or removed after `WorldBuilder::build()`. All
registration happens at build time. This enables dense indexing and
eliminates runtime bookkeeping.

## Examples

- [`mock_runtime`](examples/mock_runtime.rs) — Complete driver model:
  plugin registration, driver installation, explicit poll loop
- [`pipeline`](examples/pipeline.rs) — Pipeline composition: bare value,
  Option, Result with catch, build into Handler
- [`local_state`](examples/local_state.rs) — Per-handler state with
  `Local<T>`, independent across handler instances
- [`optional_resources`](examples/optional_resources.rs) — Optional
  dependencies with `Option<Res<T>>` / `Option<ResMut<T>>`
- [`perf_pipeline`](examples/perf_pipeline.rs) — Dispatch latency
  benchmarks with codegen inspection probes
- [`perf_construction`](examples/perf_construction.rs) — Construction-time
  latency benchmarks at various arities
- [`perf_template`](examples/perf_template.rs) — Template generation
  vs `into_handler` construction benchmarks
- [`perf_fetch`](examples/perf_fetch.rs) — Fetch dispatch strategy
  benchmarks
- [`mio_timer`](examples/mio_timer.rs) — Echo server combining mio
  and timer drivers with template construction benchmarks

## License

See workspace root for license details.
