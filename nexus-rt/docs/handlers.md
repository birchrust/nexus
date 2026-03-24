# Handlers

Handlers are the units of work in nexus-rt. They're plain Rust functions
whose parameters declare what resources they need. The framework resolves
parameters at build time and dispatches with zero overhead.

## Writing a Handler

```rust
fn process_order(
    mut state: ResMut<OrderBook>,   // exclusive write access
    config: Res<Config>,             // shared read access
    clock: Res<Clock>,               // shared read access
    event: OrderEvent,               // the event being processed
) {
    state.apply(event, config.max_depth);
    log::info!("processed at {}", clock.unix_nanos());
}
```

**Rules:**
- Must be a **named function** (not a closure — see below)
- Parameters are `Res<T>`, `ResMut<T>`, `Local<T>`, `Option<Res<T>>`, or the event type
- The event type (if present) must be the last parameter
- Up to 8 resource parameters are supported

## Building a Handler

Convert a function into a `Handler` using `into_handler`:

```rust
let registry = world.registry();
let handler = process_order.into_handler(registry);
```

`into_handler` resolves all parameter types against the registry at build
time, producing pre-resolved `ResourceId`s. At dispatch time, there's no
type lookup — just pointer dereferences.

## Dispatch

Handlers implement the `Handler<E>` trait:

```rust
pub trait Handler<E>: Send {
    fn run(&mut self, world: &mut World, event: E);
}
```

The framework calls `run()` with the World and the event. The handler's
internal state (resolved ResourceIds, Local storage) handles the rest.

## Parameter Types

| Parameter | Access | When to Use |
|-----------|--------|------------|
| `Res<T>` | `&T` (shared) | Read-only access to a resource |
| `ResMut<T>` | `&mut T` (exclusive) | Read-write access to a resource |
| `Local<T>` | `&mut T` (per-handler) | State private to this handler instance |
| `Option<Res<T>>` | `Option<&T>` | Resource that may not be registered |
| `Option<ResMut<T>>` | `Option<&mut T>` | Same, mutable |
| `#[derive(Param)]` struct | grouped fields | Bundle multiple params into one struct |
| Event type | by value | The event being processed |

## Dynamic Dispatch (Virtual)

For collections of handlers with different concrete types:

```rust
// Box<dyn Handler<E>> — heap allocated
let handler: Box<dyn Handler<MyEvent>> = Box::new(
    my_function.into_handler(registry)
);

// FlatVirtual<E> — inline storage, no heap (requires smartptr feature)
let handler: FlatVirtual<MyEvent> = FlatVirtual::new(
    my_function.into_handler(registry)
);
```

## Callbacks (Handler with Context)

For handlers that own state beyond what's in the World:

```rust
use nexus_rt::{Callback, Resource};

struct MyContext {
    connection_id: u64,
    buffer: Vec<u8>,
}

#[derive(Resource)]
struct SharedState {
    bytes_received: u64,
}

fn on_data(
    ctx: &mut MyContext,            // owned context
    state: ResMut<SharedState>,     // world resource
    event: DataEvent,
) {
    ctx.buffer.extend_from_slice(&event.data);
    state.bytes_received += event.data.len() as u64;
}

let callback = on_data.into_callback(
    MyContext { connection_id: 42, buffer: Vec::new() },
    registry,
);
```

The context is stored inside the callback — not in the World. Each
callback instance has its own context.

## Returning Handlers from Functions (Rust 2024)

When you write a factory function that takes `&Registry` and returns
`impl Handler<E>`, Rust 2024's default capture rules hold the registry
borrow in the return type — blocking subsequent `WorldBuilder` calls.

Add `+ use<...>` listing only the type parameters the return type holds:

```rust
fn build_handler<C: Config>(
    reg: &Registry,
) -> impl Handler<Order> + use<C> {
    process_order::<C>.into_handler(reg)
}
```

The `&Registry` is consumed during `into_handler` — the returned handler
holds pre-resolved `ResourceId`s, not a reference to the registry. The
`use<C>` annotation tells the compiler exactly that.

If there are no type parameters to capture, use an empty `use<>`:

```rust
fn build_handler(reg: &Registry) -> impl Handler<Order> + use<> {
    process_order.into_handler(reg)
}
```

This applies to all handler-producing patterns: `into_handler`,
`into_callback`, `PipelineBuilder::build()`, `DagBuilder::build()`,
and `template.generate()`.

## Why Named Functions?

```rust
// ✓ Works
fn my_handler(state: ResMut<MyState>, event: MyEvent) { ... }
let h = my_handler.into_handler(registry);

// ✗ Does not compile
let h = (|state: ResMut<MyState>, event: MyEvent| { ... }).into_handler(registry);
```

The parameter resolution uses Higher-Ranked Trait Bounds (HRTBs) with
a double-bound pattern: one bound for type inference, another for
dispatch. Closure types are unnameable and interact poorly with this
pattern — the compiler can't infer the parameter types through the
HRTB. Named functions have concrete types that resolve cleanly.

This is the same limitation Bevy has with its systems. In practice,
handlers are typically standalone functions — it's a natural code
organization pattern.

**Exception:** Arity-0 closures (no resource parameters) DO work:

```rust
// ✓ Works — no Res/ResMut parameters
let h = (|event: MyEvent| { println!("{event:?}"); }).into_handler(registry);
```

## Pre-built handlers as `IntoHandler`

Any type that already implements `Handler<E>` — including `Pipeline`,
`Dag`, `Callback`, and `TemplatedHandler` — satisfies `IntoHandler<E, Resolved>`
via a blanket impl. This means you can pass a built pipeline directly
to any API that expects `impl IntoHandler`:

```rust
let pipeline = PipelineBuilder::<Order>::new()
    .then(validate, reg)
    .dispatch(submit.into_handler(reg))
    .build();

// Pass directly — no wrapping needed.
// The `Resolved` marker type makes this work.
driver.register(pipeline, reg);
```

Users never need to name `Resolved` — it's inferred automatically.
