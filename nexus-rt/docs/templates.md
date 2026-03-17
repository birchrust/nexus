# Templates (Advanced)

Templates are handler factories. When you need many handlers that share
the same function but differ in per-instance context, templates let you
produce handlers without re-resolving parameters each time.

## The Problem

Without templates, creating N handlers from the same function requires
N calls to `into_handler`:

```rust
// Creating 100 handlers for 100 connections — each resolves parameters
for id in 0..100 {
    let handler = on_data.into_handler(registry);  // resolves every time
    // ... store handler ...
}
```

Each `into_handler` call walks the registry to resolve `ResourceId`s for
every parameter. For a function with 4 parameters, that's 4 lookups × 100
handlers = 400 lookups.

## The Solution

A template resolves once, generates many:

```rust
use nexus_rt::{HandlerTemplate, handler_blueprint};

// Define a blueprint (binds the function signature)
handler_blueprint!(OnDataBlueprint, Event = DataEvent, Params = (ResMut<'static, SharedState>,));

// Create the template — resolves parameters ONCE, pass the function
let template = HandlerTemplate::<OnDataBlueprint>::new(on_data, registry);

// Generate 100 handlers — no parameter resolution, just copying state
for id in 0..100 {
    let handler = template.generate();  // O(1), copies pre-resolved state
    // ... store handler ...
}
```

## With Context (Callbacks)

Templates work with callbacks that have per-instance owned context:

```rust
use nexus_rt::{CallbackTemplate, callback_blueprint};

struct ConnectionCtx {
    id: u64,
    buffer: Vec<u8>,
}

fn on_data(ctx: &mut ConnectionCtx, state: ResMut<SharedState>, event: DataEvent) {
    ctx.buffer.extend_from_slice(&event.data);
    state.total_bytes += event.data.len() as u64;
}

callback_blueprint!(OnDataCb, Context = ConnectionCtx, Event = DataEvent, Params = (ResMut<'static, SharedState>,));

let template = CallbackTemplate::<OnDataCb>::new(on_data, registry);

// Each generate gets its own context
for id in 0..100 {
    let handler = template.generate(ConnectionCtx {
        id,
        buffer: Vec::new(),
    });
    // handler owns its ConnectionCtx, shares parameter resolution
}
```

## Blueprint Macros

The `handler_blueprint!` and `callback_blueprint!` macros use keyword
syntax for the type parameters:

```rust
// Handler blueprint — no context
handler_blueprint!(MyBlueprint, Event = MyEvent, Params = (Res<'static, Config>, ResMut<'static, State>));

// Callback blueprint — with context
callback_blueprint!(MyCbBlueprint, Context = MyCtx, Event = MyEvent, Params = (ResMut<'static, State>,));
```

These generate a ZST struct implementing the `Blueprint` (or
`CallbackBlueprint`) trait with the specified associated types.

## When to Use Templates

- **Many handlers from one function** — connection handlers, per-instrument
  processors, per-subscription callbacks
- **Hot setup paths** — when creating handlers at runtime (e.g., new
  connection arrives, need a handler fast)
- **Memory efficiency** — parameter state (ResourceIds) is `Copy`, so
  generating is just a memcpy of a few words

## When NOT to Use Templates

- **One-off handlers** — `into_handler` is simpler and sufficient
- **Different functions** — templates bind to one function. Different
  functions need different templates.

## Design

Templates exploit the fact that parameter state (`P::State`) is `Copy`
for all standard parameter types. `ResourceId` is a pointer — it's `Copy`.
The template stores the pre-resolved state and copies it on each generate.

The function itself is a zero-sized type (ZST) — `size_of::<F>() == 0`.
Templates verify this at compile time with `const { assert!(size_of::<F>() == 0) }`.
Only named functions (not closures that capture state) are ZST.
