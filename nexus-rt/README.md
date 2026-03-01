# nexus-rt

Single-threaded, event-driven runtime primitives with pre-resolved dispatch.

`nexus-rt` provides the building blocks for constructing runtimes where
user code runs as handlers dispatched over shared state. It is **not** an
async runtime — there is no task scheduler, no work stealing, no `Future`
polling. Your `main()` is the executor.

## Design

`nexus-rt` is heavily inspired by [Bevy ECS](https://bevyengine.org/).
Handlers as plain functions, `SystemParam` for declarative dependency
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
| **Pipeline** | Pre-resolved stage chains inside drivers. The workhorse. | ~2 cycles p50 |
| **Callback** | Dynamic per-instance context + pre-resolved params. | ~2 cycles p50 |
| **Handler** | `Box<dyn Handler<E>>` for type-erased dispatch. | ~2 cycles p50 |

All tiers resolve `SystemParam` state at build time. Dispatch-time cost is
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

### Res / ResMut — system parameters

Declare resource dependencies in function signatures. `Res<T>` for shared
reads, `ResMut<T>` for exclusive writes. `ResMut` stamps `changed_at` on
`DerefMut` — the act of writing is the change signal.

```rust
fn process(config: Res<Config>, mut state: ResMut<State>, event: Event) {
    // config is read-only, state is read-write
}
```

### Pipeline — pre-resolved processing chains

Typed composition chains where each stage is a named function with
`SystemParam` dependencies resolved at build time.

```rust
let mut pipeline = PipelineStart::<Order>::new()
    .stage(validate, registry)       // Order → Result<Order, Error>
    .and_then(enrich, registry)      // Order → Result<Order, Error>
    .catch(log_error, registry)      // Error → () (side effect)
    .map(submit, registry)           // Order → Receipt
    .build();                        // → Pipeline<Order, _> (concrete)

pipeline.run(&mut world, order);
```

Option and Result combinators (`.map()`, `.and_then()`, `.catch()`,
`.filter()`, `.unwrap_or()`, etc.) enable typed flow control without
runtime overhead.

### Batch pipeline — per-item processing over a buffer

`build_batch(capacity)` produces a `BatchPipeline` that owns a
pre-allocated input buffer. Each item flows through the same chain
independently — errors are handled per-item, not per-batch.

```rust
let mut batch = PipelineStart::<Order>::new()
    .stage(validate, registry)       // Order → Result<Order, Error>
    .catch(log_error, registry)      // handle error, continue batch
    .map(enrich, registry)           // runs for valid items only
    .stage(submit, registry)
    .build_batch(1024);

// Driver fills input buffer
batch.input_mut().extend_from_slice(&orders);
batch.run(&mut world);  // drains buffer, no allocation
```

No intermediate buffers between stages. The compiler monomorphizes
the per-item chain identically to the single-event pipeline.

### Change detection

Each resource tracks a `changed_at` sequence number. `Res::is_changed()`
and `Handler::inputs_changed()` compare against the world's current
sequence. Drivers call `world.next_sequence()` before each event dispatch.

### Plugin — composable registration

Fire-and-forget resource registration units. Consumed by `WorldBuilder`:

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
Useful for counters, caches, or any per-handler accumulator.

### Callback — context-owning handlers

`Callback<C, F, Params>` is a handler with per-instance owned context.
Convention: `fn handler(ctx: &mut C, params..., event: E)`.

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
| inputs_changed (1 param) | 1 | 1 | 2 |
| inputs_changed (8 params) | 4 | 6 | 9 |

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
| .stage() (2 params) | 28 | 48 | 96 |

Construction cost is paid once at build time, never on the dispatch
hot path.

### Running benchmarks

```bash
taskset -c 0 cargo run --release -p nexus-rt --example perf_pipeline
taskset -c 0 cargo run --release -p nexus-rt --example perf_construction
```

## Limitations

### Named functions only

`IntoHandler`, `IntoCallback`, and `IntoStage` (arity 1+) require named
`fn` items — closures do not work due to Rust's HRTB inference limitations
with GATs. This is the same limitation as Bevy's system registration.

Arity-0 pipeline stages (no `SystemParam`) do accept closures:
```rust
// Works — arity-0 closure
pipeline.stage(|x: u32| x * 2, registry);

// Does NOT work — arity-1 closure with SystemParam
// pipeline.stage(|config: Res<Config>, x: u32| x, registry);

// Works — named function
fn transform(config: Res<Config>, x: u32) -> u32 { x + *config as u32 }
pipeline.stage(transform, registry);
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
- [`perf_fetch`](examples/perf_fetch.rs) — Fetch dispatch strategy
  benchmarks

## License

See workspace root for license details.
