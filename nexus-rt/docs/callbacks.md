# Callbacks -- Handlers with Owned Context

Callbacks are handlers that own per-instance state beyond what's in the
World. Each callback instance has its own private context, accessible
both inside and outside dispatch.

## When to Use Callback vs Handler

Use `IntoHandler` when all state lives in the World.

Use `IntoCallback` when each handler instance needs its own private state:

- **Per-timer context** -- each timer carries its own order ID, retry count,
  or deadline metadata
- **Per-connection state** -- each socket handler carries its own codec state,
  read buffer, or session context
- **Protocol state machines** -- each instance tracks its own position in a
  handshake or reconnection sequence

## Creating a Callback

The function signature puts the context first, then resources, then the event:

```rust
use nexus_rt::{WorldBuilder, ResMut, IntoCallback, Handler, Resource};

#[derive(Resource)]
struct TotalBytes(u64);

struct ConnectionCtx {
    connection_id: u64,
    bytes_received: u64,
}

fn on_data(
    ctx: &mut ConnectionCtx,           // owned context -- first param
    mut total: ResMut<TotalBytes>,     // world resource
    event: Vec<u8>,                    // event -- last param
) {
    ctx.bytes_received += event.len() as u64;
    total.0 += event.len() as u64;
}

let mut wb = WorldBuilder::new();
wb.register(TotalBytes(0));
let mut world = wb.build();

let mut cb = on_data.into_callback(
    ConnectionCtx { connection_id: 42, bytes_received: 0 },
    world.registry(),
);

cb.run(&mut world, vec![1, 2, 3]);
cb.run(&mut world, vec![4, 5]);

// Context is pub -- directly accessible
assert_eq!(cb.ctx.connection_id, 42);
assert_eq!(cb.ctx.bytes_received, 5);
assert_eq!(world.resource::<TotalBytes>().0, 5);
```

## Accessing Context Outside Dispatch

The `ctx` field on `Callback` is `pub`. Drivers can read or mutate it
between dispatches -- for example, to update a deadline or check a counter:

```rust
use nexus_rt::{WorldBuilder, IntoCallback, Handler};

struct RetryCtx {
    attempts: u32,
    max_retries: u32,
}

fn on_retry(ctx: &mut RetryCtx, _event: ()) {
    ctx.attempts += 1;
}

let mut world = WorldBuilder::new().build();

let mut cb = on_retry.into_callback(
    RetryCtx { attempts: 0, max_retries: 3 },
    world.registry(),
);

cb.run(&mut world, ());
cb.run(&mut world, ());

// Check context between dispatches
if cb.ctx.attempts >= cb.ctx.max_retries {
    // stop retrying
}
assert_eq!(cb.ctx.attempts, 2);

// Mutate context between dispatches
cb.ctx.max_retries = 5;
```

## Multiple Callbacks with Different Contexts

Each callback instance owns its own context. This is the primary use case --
per-connection, per-instrument, or per-order handlers:

```rust
use nexus_rt::{WorldBuilder, ResMut, IntoCallback, Handler, Resource};

#[derive(Resource)]
struct OrderState { total_filled: u64 }

struct OrderCtx {
    order_id: u64,
    filled_qty: u64,
}

fn on_fill(ctx: &mut OrderCtx, mut state: ResMut<OrderState>, fill_qty: u64) {
    ctx.filled_qty += fill_qty;
    state.total_filled += fill_qty;
}

let mut wb = WorldBuilder::new();
wb.register(OrderState { total_filled: 0 });
let mut world = wb.build();
let reg = world.registry();

// Each callback has its own OrderCtx
let mut order_a = on_fill.into_callback(
    OrderCtx { order_id: 100, filled_qty: 0 },
    reg,
);
let mut order_b = on_fill.into_callback(
    OrderCtx { order_id: 200, filled_qty: 0 },
    reg,
);

order_a.run(&mut world, 50u64);
order_b.run(&mut world, 30u64);
order_a.run(&mut world, 20u64);

assert_eq!(order_a.ctx.filled_qty, 70);
assert_eq!(order_b.ctx.filled_qty, 30);
assert_eq!(world.resource::<OrderState>().total_filled, 100);
```

## Callbacks with Local State

Callbacks support `Local<T>` in addition to their owned context. This is
useful when you want both shared-nothing handler state (Local) AND per-instance
metadata (context):

```rust
use nexus_rt::{WorldBuilder, ResMut, Local, IntoCallback, Handler, Resource};

#[derive(Resource)]
struct Log { entries: Vec<String> }

struct Ctx { name: String }

fn on_event(ctx: &mut Ctx, mut call_count: Local<u64>, mut log: ResMut<Log>, event: u32) {
    *call_count += 1;
    log.entries.push(format!("{}: event {} (call #{})", ctx.name, event, *call_count));
}

let mut wb = WorldBuilder::new();
wb.register(Log { entries: vec![] });
let mut world = wb.build();
let reg = world.registry();

let mut cb = on_event.into_callback(Ctx { name: "conn-1".into() }, reg);
cb.run(&mut world, 1u32);
cb.run(&mut world, 2u32);

assert_eq!(world.resource::<Log>().entries.len(), 2);
```

## Callbacks as Box<dyn Handler<E>>

Callbacks implement `Handler<E>`, so they can be stored in heterogeneous
collections alongside regular handlers:

```rust
use nexus_rt::{WorldBuilder, ResMut, IntoHandler, IntoCallback, Handler, Resource, Virtual};

#[derive(Resource)]
struct Counter(u64);

fn add(mut c: ResMut<Counter>, event: u64) { c.0 += event; }
fn mul(ctx: &mut u64, mut c: ResMut<Counter>, _event: u64) { c.0 *= *ctx; }

let mut wb = WorldBuilder::new();
wb.register(Counter(0));
let mut world = wb.build();
let reg = world.registry();

let h1: Virtual<u64> = Box::new(add.into_handler(reg));
let h2: Virtual<u64> = Box::new(mul.into_callback(3u64, reg));

let mut handlers: Vec<Virtual<u64>> = vec![h1, h2];
for h in &mut handlers {
    h.run(&mut world, 5u64);
}

// 0 + 5 = 5, then 5 * 3 = 15
assert_eq!(world.resource::<Counter>().0, 15);
```

**Note:** Boxing erases the concrete type, so you lose direct access to
`cb.ctx`. If you need context access, keep the concrete `Callback` type.

## CallbackTemplate: Stamping Many Callbacks

When you need many callbacks from the same function with different contexts,
use `CallbackTemplate` to resolve parameters once and stamp cheaply.
See [templates.md](templates.md) for the full guide.

```rust
use nexus_rt::{WorldBuilder, ResMut, IntoHandler, Handler, Resource};
use nexus_rt::template::{CallbackTemplate, CallbackBlueprint};

#[derive(Resource)]
struct SharedState { total: u64 }

struct ConnCtx { id: u64, bytes: u64 }

fn on_data(ctx: &mut ConnCtx, mut state: ResMut<SharedState>, event: u64) {
    ctx.bytes += event;
    state.total += event;
}

// Define a blueprint (binds the function signature)
nexus_rt::callback_blueprint!(
    OnDataBp,
    Context = ConnCtx,
    Event = u64,
    Params = (ResMut<'static, SharedState>,)
);

let mut wb = WorldBuilder::new();
wb.register(SharedState { total: 0 });
let mut world = wb.build();
let reg = world.registry();

// Resolve once
let template = CallbackTemplate::<OnDataBp>::new(on_data, reg);

// Stamp 100 callbacks -- no parameter resolution, just memcpy of state
let mut callbacks: Vec<_> = (0..100)
    .map(|id| template.generate(ConnCtx { id, bytes: 0 }))
    .collect();

callbacks[0].run(&mut world, 10u64);
callbacks[99].run(&mut world, 20u64);

assert_eq!(callbacks[0].ctx.bytes, 10);
assert_eq!(callbacks[99].ctx.bytes, 20);
assert_eq!(world.resource::<SharedState>().total, 30);
```

## Returning Callbacks from Functions (Rust 2024)

When a factory function takes `&Registry` and returns `impl Handler<E>`,
Rust 2024 captures the registry borrow. Use `+ use<...>` to exclude it:

```rust
use nexus_rt::{Handler, IntoCallback, ResMut, Resource};
use nexus_rt::world::Registry;

#[derive(Resource)]
struct State(u64);

fn on_event(ctx: &mut u64, mut s: ResMut<State>, _e: ()) {
    s.0 += *ctx;
}

fn build_callback(ctx: u64, reg: &Registry) -> impl Handler<()> + use<> {
    on_event.into_callback(ctx, reg)
}
```

## Named Functions Only

The same HRTB limitation as `IntoHandler` applies: closures with resource
parameters don't work. Use named `fn` items.

Arity-0 closures (context + event only, no resources) DO work:

```rust
use nexus_rt::{WorldBuilder, IntoCallback, Handler};

struct Ctx { count: u32 }

let mut world = WorldBuilder::new().build();

let mut cb = (|ctx: &mut Ctx, event: u32| {
    ctx.count += event;
}).into_callback(Ctx { count: 0 }, world.registry());

cb.run(&mut world, 5u32);
assert_eq!(cb.ctx.count, 5);
```
