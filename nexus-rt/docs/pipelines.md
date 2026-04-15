# Pipelines

Pipelines compose processing steps into typed chains. Each step is a
named function resolved at build time. The entire chain monomorphizes
to zero-cost -- no vtable dispatch, no allocation per event.

## Pipeline -- Linear Chain

A pipeline processes an event through a sequence of steps:

```
Input -> Step 1 -> Step 2 -> Step 3 -> Output
```

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, Res, ResMut, Handler, Resource};

#[derive(Resource)]
struct Config { max_qty: u64 }

#[derive(Resource)]
struct OrderLog { accepted: Vec<String> }

fn validate(config: Res<Config>, order: (String, u64)) -> Option<(String, u64)> {
    if order.1 > config.max_qty { return None; }
    Some(order)
}

fn log_accepted(mut log: ResMut<OrderLog>, order: (String, u64)) {
    log.accepted.push(order.0);
}

let mut wb = WorldBuilder::new();
wb.register(Config { max_qty: 1000 });
wb.register(OrderLog { accepted: vec![] });
let mut world = wb.build();
let reg = world.registry();

let mut pipeline = PipelineBuilder::<(String, u64)>::new()
    .then(validate, reg)       // (String, u64) -> Option<(String, u64)>
    .then(log_accepted, reg)   // Option propagation -- None skips
    .build();

pipeline.run(&mut world, ("BTC".into(), 100));
pipeline.run(&mut world, ("ETH".into(), 9999));  // rejected

assert_eq!(world.resource::<OrderLog>().accepted, vec!["BTC"]);
```

Each step is a named function. Input flows left to right. The chain type
is fully known at compile time -- LLVM inlines everything.

## Step Function Convention

Resources first, step input last, returns output:

```rust
use nexus_rt::{Res, ResMut, Resource};

#[derive(Resource)]
struct Config { threshold: f64 }
#[derive(Resource)]
struct Gateway { sent: Vec<u64> }

// Step: params first, input last, returns output
fn validate(config: Res<Config>, order_id: u64) -> Option<u64> {
    if (order_id as f64) > config.threshold { Some(order_id) } else { None }
}

fn submit(mut gw: ResMut<Gateway>, order_id: u64) {
    gw.sent.push(order_id);
}
```

## Combinator Quick Reference

**Bare value `T`:**

| Combinator | Signature | What it does |
|------------|-----------|-------------|
| `.then(step, reg)` | `T -> U` | Transform value |
| `.tap(f, reg)` | `&T -> ()` | Side effect, value unchanged |
| `.guard(pred, reg)` | `&T -> bool` | `true` continues, `false` -> `None` |
| `.dispatch(handler)` | `T -> ()` | Terminal: dispatch to a Handler |
| `.route(pred, true_arm, false_arm, reg)` | `&T -> bool` | Binary routing |
| `.tee(dag_arm)` | `&T -> ()` | Side-channel via DAG arm |
| `.scan(init, f, reg)` | `(&mut Acc, T) -> Out` | Stateful accumulator |
| `.dedup()` | `T -> Option<T>` | Suppress unchanged values |

**`Option<T>`:**

| Combinator | Signature | What it does |
|------------|-----------|-------------|
| `.map(f, reg)` | `T -> U` | Transform inner value |
| `.and_then(f, reg)` | `T -> Option<U>` | Chain optionals |
| `.filter(pred, reg)` | `&T -> bool` | Keep if predicate holds |
| `.inspect(f, reg)` | `&T -> ()` | Observe `Some` values |
| `.on_none(f, reg)` | `() -> ()` | Side effect on None |
| `.ok_or(err)` | `-> Result<T, E>` | Convert to Result |
| `.ok_or_else(f, reg)` | `() -> E` | Convert to Result, lazy error |
| `.unwrap_or(default)` | `-> T` | Default value |
| `.unwrap_or_else(f, reg)` | `() -> T` | Default value, lazy |
| `.cloned()` | `Option<&T> -> Option<T>` | Clone inner reference |

**`Result<T, E>`:**

| Combinator | Signature | What it does |
|------------|-----------|-------------|
| `.map(f, reg)` | `T -> U` | Transform Ok value |
| `.and_then(f, reg)` | `T -> Result<U, E>` | Chain Results |
| `.catch(f, reg)` | `E -> ()` | Handle error, convert to `Option<T>` |
| `.map_err(f, reg)` | `E -> E2` | Transform error type |
| `.or_else(f, reg)` | `E -> Result<T, E2>` | Try recovery |
| `.inspect(f, reg)` | `&T -> ()` | Observe Ok values |
| `.inspect_err(f, reg)` | `&E -> ()` | Observe Err values |
| `.ok()` | `-> Option<T>` | Discard error |
| `.unwrap_or(default)` | `-> T` | Default on error |
| `.unwrap_or_else(f, reg)` | `E -> T` | Default from error |
| `.cloned()` | `Result<&T, E> -> Result<T, E>` | Clone inner ref |

**`bool`:**

| Combinator | What it does |
|------------|-------------|
| `.not()` | Logical negation |
| `.and(f, reg)` | Short-circuit AND with producer |
| `.or(f, reg)` | Short-circuit OR with producer |
| `.xor(f, reg)` | XOR with producer |

**Tuple `(A, B, ...)` (2-5 elements):**

| Combinator | What it does |
|------------|-------------|
| `.splat()` | Destructure into individual step arguments |

**Terminal:**

| Combinator | What it does |
|------------|-------------|
| `.build()` | Build into `Pipeline` (implements `Handler<E>`) |
| `.build_batch(cap)` | Build into `BatchPipeline<E>` with pre-allocated buffer |

## Three Resolution Tiers

Every combinator accepts three kinds of functions:

### 1. Named function with Param resources (fastest)

Pre-resolved ResourceIds -- single pointer deref per resource at dispatch:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, Res, Handler, Resource};

#[derive(Resource)]
struct Config { min_qty: u64 }

fn check(config: Res<Config>, order: &(String, u64)) -> bool {
    order.1 >= config.min_qty
}

let mut wb = WorldBuilder::new();
wb.register(Config { min_qty: 10 });
let mut world = wb.build();
let reg = world.registry();

let mut p = PipelineBuilder::<(String, u64)>::new()
    .guard(check, reg)
    .build();

p.run(&mut world, ("BTC".into(), 100));  // passes guard
p.run(&mut world, ("ETH".into(), 1));    // blocked by guard
```

### 2. Arity-0 closure (no resource access)

Simple predicates and transformations:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, Handler};

let world = WorldBuilder::new().build();
let reg = world.registry();

let mut p = PipelineBuilder::<u64>::new()
    .guard(|x: &u64| *x > 10, reg)
    .then(|x: u64| x * 2, reg)
    .build();
```

### 3. Opaque closure (raw `&mut World` access)

Escape hatch -- HashMap lookup per resource access:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, Handler, Resource};

#[derive(Resource)]
struct Counter(u64);

let mut wb = WorldBuilder::new();
wb.register(Counter(0));
let mut world = wb.build();
let reg = world.registry();

let mut p = PipelineBuilder::<u64>::new()
    .tap(|world: &mut nexus_rt::World, val: &u64| {
        world.resource_mut::<Counter>().0 += val;
    }, reg)
    .build();

p.run(&mut world, 5u64);
assert_eq!(world.resource::<Counter>().0, 5);
```

## .guard() -- Conditional Gate

Evaluates a predicate on `&T`. `true` continues the pipeline, `false`
short-circuits to `None`:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, Res, Handler, Resource};

#[derive(Resource)]
struct Limits { max_price: f64 }

fn price_check(limits: Res<Limits>, order: &(f64, u64)) -> bool {
    order.0 <= limits.max_price
}

let mut wb = WorldBuilder::new();
wb.register(Limits { max_price: 100.0 });
let mut world = wb.build();
let reg = world.registry();

let mut p = PipelineBuilder::<(f64, u64)>::new()
    .guard(price_check, reg)  // (f64, u64) -> Option<(f64, u64)>
    .build();

// Returns Some for valid prices, None for violations
```

## .filter() -- Keep Matching Items

Works on `Option<T>` -- keeps `Some` values where the predicate holds,
replaces others with `None`:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, Handler};

let world = WorldBuilder::new().build();
let reg = world.registry();

let mut p = PipelineBuilder::<u64>::new()
    .guard(|x: &u64| *x > 0, reg)         // u64 -> Option<u64>
    .filter(|x: &u64| *x % 2 == 0, reg)   // keep only even
    .build();
```

## .tap() -- Side Effects

Observes the value by reference without consuming or changing it:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, ResMut, Handler, Resource};

#[derive(Resource)]
struct Log { entries: Vec<u64> }

fn log_value(mut log: ResMut<Log>, val: &u64) {
    log.entries.push(*val);
}

let mut wb = WorldBuilder::new();
wb.register(Log { entries: vec![] });
let mut world = wb.build();
let reg = world.registry();

let mut p = PipelineBuilder::<u64>::new()
    .tap(log_value, reg)       // observe, value continues unchanged
    .then(|x: u64| x * 2, reg)
    .build();

p.run(&mut world, 5u64);
assert_eq!(world.resource::<Log>().entries, vec![5]);
```

## .scan() -- Stateful Accumulator

Maintains state across invocations. The accumulator is per-pipeline-instance:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, ResMut, Handler, Resource};

#[derive(Resource)]
struct Out(f64);

fn running_avg(acc: &mut (f64, u64), value: f64) -> f64 {
    acc.0 += value;
    acc.1 += 1;
    acc.0 / acc.1 as f64
}

fn store(mut out: ResMut<Out>, avg: f64) {
    out.0 = avg;
}

let mut wb = WorldBuilder::new();
wb.register(Out(0.0));
let mut world = wb.build();
let reg = world.registry();

let mut p = PipelineBuilder::<f64>::new()
    .scan((0.0_f64, 0_u64), running_avg, reg)  // f64 -> f64 (running average)
    .then(store, reg)
    .build();

p.run(&mut world, 10.0);
p.run(&mut world, 20.0);
assert_eq!(world.resource::<Out>().0, 15.0);  // (10 + 20) / 2
```

## .route() -- Binary Routing

Evaluates a predicate and executes exactly one of two arms:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, ResMut, Handler, Resource};
use nexus_rt::dag::DagBuilder;

#[derive(Resource)]
struct FastPath(u64);
#[derive(Resource)]
struct SlowPath(u64);

fn store_fast(mut out: ResMut<FastPath>, val: &u64) { out.0 = *val; }
fn store_slow(mut out: ResMut<SlowPath>, val: &u64) { out.0 = *val; }

let mut wb = WorldBuilder::new();
wb.register(FastPath(0));
wb.register(SlowPath(0));
let mut world = wb.build();
let reg = world.registry();

// Build the two arms as DAG arms
let fast_arm = DagBuilder::<u64>::arm().then(store_fast, reg).build();
let slow_arm = DagBuilder::<u64>::arm().then(store_slow, reg).build();

let mut p = PipelineBuilder::<u64>::new()
    .route(|x: &u64| *x > 100, fast_arm, slow_arm, reg)
    .build();

p.run(&mut world, 200u64);  // fast path
assert_eq!(world.resource::<FastPath>().0, 200);

p.run(&mut world, 50u64);   // slow path
assert_eq!(world.resource::<SlowPath>().0, 50);
```

## .dedup() -- Suppress Unchanged Values

Remembers the last value and only passes through when it changes:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, ResMut, Handler, Resource};

#[derive(Resource)]
struct Updates(Vec<u64>);

fn store(mut u: ResMut<Updates>, val: u64) { u.0.push(val); }

let mut wb = WorldBuilder::new();
wb.register(Updates(vec![]));
let mut world = wb.build();
let reg = world.registry();

let mut p = PipelineBuilder::<u64>::new()
    .dedup()                   // u64 -> Option<u64>
    .then(store, reg)          // only fires on change
    .build();

p.run(&mut world, 1);
p.run(&mut world, 1);  // suppressed
p.run(&mut world, 2);
p.run(&mut world, 2);  // suppressed

assert_eq!(world.resource::<Updates>().0, vec![1, 2]);
```

## .splat() -- Tuple Destructuring

When a step returns a tuple, the next step normally receives the whole
tuple. `.splat()` destructures it so the next step receives individual
arguments:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, Handler};

fn split(x: u64) -> (u64, u64) { (x / 2, x % 2) }
fn combine(a: u64, b: u64) -> u64 { a * 10 + b }

let world = WorldBuilder::new().build();
let reg = world.registry();

let mut p = PipelineBuilder::<u64>::new()
    .then(split, reg)      // u64 -> (u64, u64)
    .splat()               // destructure
    .then(combine, reg)    // (u64, u64) -> u64
    .build();
```

Supported for tuples of 2-5 elements.

## .view() / .end_view() -- Projected View Scopes

Opens a scope where steps operate on a read-only view constructed from
the event. Useful when you want to operate on a borrowed subset of a
large event:

```rust
use nexus_rt::{View, WorldBuilder, PipelineBuilder, Handler};

struct LargeEvent { price: f64, qty: u64, metadata: Vec<u8> }

// View is a lightweight projection
struct PriceView<'a> { price: &'a f64 }

struct AsPriceView;
unsafe impl View<LargeEvent> for AsPriceView {
    type ViewType<'a> = PriceView<'a>;
    type StaticViewType = PriceView<'static>;
    fn view(source: &LargeEvent) -> PriceView<'_> {
        PriceView { price: &source.price }
    }
}

fn check_price(view: &PriceView<'static>) -> bool {
    *view.price > 0.0
}

let world = WorldBuilder::new().build();
let reg = world.registry();

let mut p = PipelineBuilder::<LargeEvent>::new()
    .view::<AsPriceView>()
    .guard(check_price, reg)
    .end_view()  // back to LargeEvent
    .build();
```

## .dispatch() -- Terminal Handler Dispatch

Dispatches the pipeline output to a `Handler<T>` at the end of the chain:

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, ResMut, IntoHandler, Handler, Resource};

#[derive(Resource)]
struct Sink(Vec<u64>);

fn collect(mut sink: ResMut<Sink>, value: u64) { sink.0.push(value); }

let mut wb = WorldBuilder::new();
wb.register(Sink(vec![]));
let mut world = wb.build();
let reg = world.registry();

let handler = collect.into_handler(reg);

let mut p = PipelineBuilder::<u64>::new()
    .then(|x: u64| x * 2, reg)
    .dispatch(handler)
    .build();

p.run(&mut world, 5u64);
assert_eq!(world.resource::<Sink>().0, vec![10]);
```

## .build() vs .build_batch()

`.build()` returns a `Pipeline` that processes one event per `run()` call.

`.build_batch(capacity)` returns a `BatchPipeline` with a pre-allocated
input buffer. You push events into the buffer, then drain them all
through the pipeline. Errors on one item don't affect subsequent items.

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, ResMut, Handler, Resource};

#[derive(Resource)]
struct Sum(u64);

fn add(mut s: ResMut<Sum>, x: u64) { s.0 += x; }

let mut wb = WorldBuilder::new();
wb.register(Sum(0));
let mut world = wb.build();
let reg = world.registry();

let mut batch = PipelineBuilder::<u64>::new()
    .then(add, reg)
    .build_batch(64);  // pre-allocate buffer for 64 events

batch.push(1);
batch.push(2);
batch.push(3);
batch.drain(&mut world);  // processes all three

assert_eq!(world.resource::<Sum>().0, 6);
```

## Complete Example: Order Processing Pipeline

```rust
use nexus_rt::{WorldBuilder, PipelineBuilder, Res, ResMut, Handler, Resource};

// Domain types
#[derive(Clone)]
struct Order { symbol: String, qty: u64, price: f64 }
struct ValidOrder { symbol: String, qty: u64, price: f64, timestamp: u64 }

// Resources
#[derive(Resource)]
struct RiskConfig { max_qty: u64, max_notional: f64 }

#[derive(Resource)]
struct Clock(u64);

#[derive(Resource)]
struct ExecutionLog { orders: Vec<String> }

// Steps
fn validate(config: Res<RiskConfig>, order: Order) -> Result<Order, String> {
    if order.qty > config.max_qty {
        return Err(format!("qty {} exceeds max {}", order.qty, config.max_qty));
    }
    let notional = order.qty as f64 * order.price;
    if notional > config.max_notional {
        return Err(format!("notional {notional} exceeds max {}", config.max_notional));
    }
    Ok(order)
}

fn enrich(clock: Res<Clock>, order: Order) -> ValidOrder {
    ValidOrder {
        symbol: order.symbol,
        qty: order.qty,
        price: order.price,
        timestamp: clock.0,
    }
}

fn execute(mut log: ResMut<ExecutionLog>, order: ValidOrder) {
    log.orders.push(format!("{}:{}@{}", order.symbol, order.qty, order.price));
}

fn log_rejection(mut log: ResMut<ExecutionLog>, err: String) {
    log.orders.push(format!("REJECTED: {err}"));
}

// Build
let mut wb = WorldBuilder::new();
wb.register(RiskConfig { max_qty: 1000, max_notional: 1_000_000.0 });
wb.register(Clock(1000));
wb.register(ExecutionLog { orders: vec![] });
let mut world = wb.build();
let reg = world.registry();

let mut pipeline = PipelineBuilder::<Order>::new()
    .then(validate, reg)            // Order -> Result<Order, String>
    .inspect_err(log_rejection, reg) // log errors without consuming
    .map(enrich, reg)               // Result<Order, _> -> Result<ValidOrder, _>
    .then(execute, reg)              // Result auto-propagation
    .build();

pipeline.run(&mut world, Order { symbol: "BTC".into(), qty: 100, price: 50_000.0 });
pipeline.run(&mut world, Order { symbol: "ETH".into(), qty: 9999, price: 3_000.0 });

let log = &world.resource::<ExecutionLog>().orders;
assert_eq!(log.len(), 2);
assert_eq!(log[0], "BTC:100@50000");
assert!(log[1].starts_with("REJECTED:"));
```

## Returning Pipelines from Functions (Rust 2024)

Pipeline factory functions are the most common place to hit Rust 2024's
lifetime capture rules. Add `+ use<...>` listing only the type parameters
the pipeline holds:

```rust
use nexus_rt::{Handler, PipelineBuilder, Res, ResMut, Resource};
use nexus_rt::world::Registry;

fn on_order(reg: &Registry) -> impl Handler<u64> + use<> {
    PipelineBuilder::<u64>::new()
        .then(|x: u64| x * 2, reg)
        .build()
}
```

Without `+ use<>`, the compiler assumes the pipeline borrows `reg`,
blocking subsequent builder calls.

## Performance

The chain is a nested struct type:

```
ThenNode<ThenNode<GuardNode<Start, Pred>, Step1>, Step2>
```

LLVM sees through all layers. The compiled code is equivalent to
writing the steps inline -- verified by the
[codegen audit](codegen-audit.md) (243 audit functions).

No allocation per dispatch. No vtable lookup. One function call
that inlines to the sequence of steps.

## Need Per-Instance State?

This document covers `PipelineBuilder` — pipelines composed from
`Handler`-style functions where state lives in the World.

If each step needs access to per-instance context (a session ID, a
retry counter, a connection handle), use `CtxPipelineBuilder` instead.
It's the parallel API for callbacks, threading `&mut C` through every
step. See [callbacks.md — Callback Pipelines](callbacks.md#callback-pipelines-ctxpipeline)
for the full guide.

Same combinator names, same builder pattern, same monomorphization —
just with a context parameter threaded through.
