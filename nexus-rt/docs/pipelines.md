# Pipelines & DAGs

Pipelines and DAGs compose processing steps into chains. Each step is
a named function resolved at build time. The entire chain monomorphizes
to zero-cost — no vtable dispatch, no allocation per event.

## Pipeline — Linear Chain

A pipeline processes an event through a sequence of steps:

```
Input → Step 1 → Step 2 → Step 3 → Output
```

```rust
use nexus_rt::{Pipeline, Res, ResMut};

fn validate(order: Order, config: Res<Config>) -> Result<Order, String> {
    if order.qty > config.max_qty {
        return Err("qty too large".into());
    }
    Ok(order)
}

fn enrich(order: Order, clock: Res<Clock>) -> Order {
    Order { timestamp: clock.unix_nanos(), ..order }
}

fn submit(order: Order, mut state: ResMut<OrderState>) {
    state.pending.push(order);
}

// Build the pipeline
let pipeline = PipelineBuilder::<Order>::new()
    .then(validate, registry)
    .then(enrich, registry)
    .then(submit, registry)
    .build();
```

Each step is a named function. Input flows left to right. The chain type
is fully known at compile time — LLVM inlines everything.

## DAG — Directed Acyclic Graph

A DAG allows branching and merging:

```
        ┌→ Step A ─┐
Input ──┤           ├──→ Merge → Output
        └→ Step B ─┘
```

DAGs support conditional routing, fan-out, and convergence. Built using
the same combinator API as pipelines.

## Combinators

Both pipelines and DAGs support these composition methods:

| Combinator | What It Does |
|------------|-------------|
| `.then(step, reg)` | Chain: output of prev → input of next |
| `.guard(pred, reg)` | Gate: only continue if predicate returns true |
| `.filter(pred, reg)` | Keep only items matching predicate |
| `.tap(fn, reg)` | Side effect: observe without modifying |
| `.inspect(fn, reg)` | Like tap, but for `Result` — observe `Ok` values |
| `.inspect_err(fn, reg)` | Observe `Err` values |
| `.catch(fn, reg)` | Handle errors — convert `Result<T, E>` to `T` |

### Three Resolution Tiers

Every combinator accepts three kinds of functions:

1. **Named function with parameters** — accesses World resources:
   ```rust
   fn check(order: &Order, config: Res<Config>) -> bool { ... }
   pipeline.guard(check, registry)
   ```

2. **Arity-0 closure** — no resource access, just the value:
   ```rust
   pipeline.guard(|order: &Order| order.qty > 0, registry)
   ```

3. **Opaque closure** — raw World access:
   ```rust
   pipeline.guard(|world: &mut World, order: &Order| { ... }, registry)
   ```

Named functions get pre-resolved ResourceIds (fastest). Arity-0 closures
have no resource lookup. Opaque closures do HashMap lookup per call.

## Pipeline Output

Pipelines produce a value. To collect or process the output:

```rust
let result = pipeline.run(&mut world, input_order);
```

Or register the pipeline as a handler for a specific event type.

## Performance

The chain is a nested struct type:

```
ThenNode<ThenNode<GuardNode<Start, Pred>, Step1>, Step2>
```

LLVM sees through all layers. The compiled code is equivalent to
writing the steps inline — verified by the
[codegen audit](codegen-audit.md) (243 audit functions).

No allocation per dispatch. No vtable lookup. One function call
that inlines to the sequence of steps.

## Returning Pipelines from Functions (Rust 2024)

Pipeline factory functions are the most common place to hit Rust 2024's
lifetime capture rules. When a function takes `&Registry` and returns
`impl Handler<E>`, the return type captures the registry borrow by
default — blocking subsequent `WorldBuilder` calls.

Add `+ use<...>` listing only the type parameters the pipeline holds:

```rust
fn on_order<C: Config>(
    reg: &Registry,
) -> impl Handler<Order> + use<C> {
    PipelineBuilder::<Order>::new()
        .then(validate::<C>, reg)
        .guard(check_risk::<C>, reg)
        .dispatch(submit::<C>.into_handler(reg))
        .build()
}
```

The `&Registry` is consumed during `.then()` / `.build()` — the built
pipeline holds pre-resolved `ResourceId`s, not a reference to the
registry. Without `+ use<C>`, the compiler assumes the pipeline borrows
`reg` and the following will fail:

```rust
let reg = wb.registry();
let pipeline = on_order::<MyConfig>(reg);   // borrows reg
let timer = wb.install_driver(timer);       // ERROR: wb still borrowed
```

With `+ use<C>`, the borrow ends when `on_order` returns.

If there are no type parameters, use `+ use<>`.

## Tips

- **Steps should be small** — each step does one thing. Compose many
  small steps rather than one large handler.
- **Guard early** — put validation guards at the front of the pipeline
  to avoid unnecessary work.
- **Use `tap` for logging** — observe without disrupting the chain.
- **Named functions for hot paths** — closures work for simple predicates,
  but named functions with `Res<T>` parameters are the fast path.
