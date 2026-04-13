# Architecture

nexus-rt is a dispatch framework, not a runtime. It provides the building
blocks — World, Handlers, Pipelines, Drivers — and the user composes their
own event loop. No async/await. No task scheduler. No implicit execution.

## Design Philosophy

**The user owns the loop.** nexus-rt provides typed resource storage
(`World`), zero-cost dispatch (`Handler`), and composable processing chains
(`Pipeline`, `DAG`). The user writes the poll loop, decides the event order,
and controls the tick rate. The framework gets out of the way.

**Zero-cost dispatch.** A 5-stage pipeline compiles to the same code as
5 inlined function calls. No vtable, no Box, no dynamic dispatch (unless
you opt into it with `Virtual<E>`). The `#[inline(always)]` chain through
`ChainCall::call` + `StepCall::call` ensures LLVM sees one flat function.

**Single writer.** `World` is `!Send + !Sync`. One thread owns all state.
`Res<T>` and `ResMut<T>` provide typed access with compile-time borrow
checking through the parameter resolution system.

## Component Map

```
┌────────────────────────────────────────────────────┐
│                   User Event Loop                   │
│                                                    │
│  loop {                                            │
│      driver_handle.poll(&mut world);               │
│      // dispatch events through pipelines/handlers │
│  }                                                 │
│                                                    │
├────────────────────────────────────────────────────┤
│                                                    │
│  ┌─────────┐  ┌───────────┐  ┌──────────────────┐ │
│  │  World  │  │  Handler  │  │    Pipeline      │ │
│  │         │  │           │  │                  │ │
│  │ TypeId  │  │ fn(Res<A>,│  │ Step → Step →    │ │
│  │   →     │  │   ResMut<B│  │  Guard → Map →   │ │
│  │ Resource│  │   event)  │  │   Tap → Output   │ │
│  │ Id      │  │           │  │                  │ │
│  └────┬────┘  └─────┬─────┘  └────────┬─────────┘ │
│       │             │                  │           │
│  ┌────┴─────────────┴──────────────────┴─────────┐ │
│  │           Parameter Resolution                 │ │
│  │  Res<T> → ResourceId → NonNull → &T            │ │
│  │  ResMut<T> → ResourceId → NonNull → &mut T     │ │
│  │  Resolved once at build time. Single deref     │ │
│  │  at dispatch time. ~1 cycle per resource.       │ │
│  └────────────────────────────────────────────────┘ │
│                                                    │
│  ┌──────────────┐  ┌─────────────┐                │
│  │   Driver     │  │   Plugin    │                │
│  │  (Installer) │  │  (Builder)  │                │
│  │              │  │             │                │
│  │ install(wb)  │  │ build(wb)   │                │
│  │  → Handle    │  │  → ()       │                │
│  └──────────────┘  └─────────────┘                │
└────────────────────────────────────────────────────┘
```

## World & Resources

The `World` is a type-erased singleton store. Each type `T` gets one slot,
accessed via `TypeId::of::<T>()` → `ResourceId` (a `NonNull<u8>` pointing
directly to `ResourceCell<T>` on the heap).

```
Register:
  world.insert(Config { ... })
    → Box::new(ResourceCell { value, changed_at })
    → Box::into_raw → ResourceId
    → HashMap<TypeId, ResourceId>

Access:
  world.resource::<Config>()    → &Config     (single deref)
  world.resource_mut::<Config>() → &mut Config (single deref, stamps tick)
```

**Change detection:** `ResourceCell<T>` is `#[repr(C)]` with `changed_at:
Cell<Sequence>` at offset 0. `ResMut<T>` stamps on `DerefMut`. `Res<T>` can
query `is_changed(since)`. No skip vectors — the ticks ARE the propagation.

**WorldBuilder:** Registration phase. Resources and drivers are installed
here. `build()` produces the final `World`. After build, no new resources
can be added (the HashMap is frozen).

## Handler System

A `Handler<E>` is anything that can process an event `E` given access to
World resources. The core trait:

```rust
pub trait Handler<E>: 'static {
    fn run(&mut self, world: &World, event: E);
}
```

**Named functions** are the primary way to create handlers:

```rust
fn on_quote(mut books: ResMut<Books>, config: Res<Config>, quote: Quote) {
    books.update(config, quote);
}

let handler = on_quote.into_handler(world.registry());
handler.run(&world, quote);
```

**Parameter resolution** happens at `into_handler` time — each `Res<T>` /
`ResMut<T>` resolves its `TypeId` to a `ResourceId` (one HashMap lookup).
At dispatch time, each parameter is a single pointer deref. ~1 cycle per
resource.

**The HRTB double-bound pattern:** GATs aren't injective, so the compiler
can't determine `P` from `P::Item<'w>` alone. The `IntoHandler` trait uses
two bounds: `FnMut(P, E)` for type inference + `FnMut(P::Item<'a>, E)` for
dispatch. This is the same pattern Bevy uses.

**Closures don't work** with `IntoHandler` — named functions only. Closure
type inference fails with the double-bound HRTB pattern. For closures, use
`Callback<C, F, P>` with explicit context, or arity-0 pipeline stages.

### Callback: Context-Owning Handlers

```rust
let callback = Callback::new(my_state, |ctx, res: Res<Config>, event| {
    ctx.process(res, event);
});
```

`Callback<C, F, P>` owns context `C` and resolves parameters `P` the same
way as `HandlerFn`. The `CtxFree<F>` wrapper handles context-free functions
through a coherence trick (avoids overlapping impls).

### Virtual Dispatch

For heterogeneous handler collections:

```rust
let handler: Virtual<Quote> = Box::new(on_quote.into_handler(reg));
handler.run(&world, quote); // vtable call
```

`FlatVirtual<E>` / `FlexVirtual<E>` provide inline storage via nexus-smartptr
(avoids the Box heap allocation).

## Pipelines & DAGs

Composable processing chains with type-safe combinators.

```rust
let pipeline = Pipeline::new(parse_order, reg)
    .guard(check_risk, reg)         // filter: pass or reject
    .then(enrich_order, reg)        // transform: Order → EnrichedOrder
    .tap(log_order, reg)            // side effect: doesn't change value
    .map_result(|r| r.ok())         // unwrap Result
    .build();

pipeline.run(&world, raw_bytes);
```

**Pipeline** is linear: each stage feeds into the next. **DAG** allows
branching (route to different sub-pipelines based on event type).

**Three resolution tiers per combinator:**
1. Named fn with Param: `pipeline.guard(check_risk, reg)`
2. Arity-0 closure: `pipeline.guard(|o: &Order| o.price > 0.0, reg)`
3. Opaque closure: `pipeline.guard(|w: &mut World, o: &Order| { ... }, reg)`

Each compiles to the same thing — the tier just determines how parameters
are resolved.

**Codegen:** Every chain node's `call()` has `#[inline(always)]`. A 5-stage
pipeline compiles to a single function with no internal calls. Verified via
`cargo-asm` (see `docs/codegen-audit.md`).

## Drivers

The installer/poller pattern for IO and timers:

```rust
// Define an installer:
struct MyDriver { /* config */ }
impl Installer for MyDriver {
    type Handle = MyHandle;
    fn install(self, wb: &mut WorldBuilder) -> Self::Handle {
        wb.insert(MyState::default());
        MyHandle { /* pre-resolved IDs */ }
    }
}

// Install during build:
let handle = wb.install_driver(MyDriver { ... });
let mut world = wb.build();

// Poll in the event loop:
loop {
    handle.poll(&mut world);
}
```

**Key design:** The installer is consumed at build time. The handle stores
pre-resolved `ResourceId`s — no HashMap lookup in the poll loop. The handle's
`poll()` method is concrete (not a trait method), so it monomorphizes with
zero overhead.

## Templates

For stamping out many handlers with the same function signature but different
per-instance state:

```rust
let template = HandlerTemplate::<MyBlueprint>::new(handler_fn, &world);
let handler_a = template.stamp(key_a);
let handler_b = template.stamp(key_b);
```

The blueprint carries `type Params: Param`, keeping parameter state fully
typed. Only the function type `F` (unnameable) is erased via fn pointers.
No byte buffers, no `*mut u8` casts. `P::State: Copy` is required so
stamp is a memcpy.

## Clock

Pluggable time source for testing and replay:

```rust
// Production:
let clock = Clock::realtime();

// Testing (manual control):
let clock = Clock::test();
clock.advance(Duration::from_secs(1));

// Historical replay:
let clock = Clock::historical(start_time);
clock.set(specific_timestamp);
```

The clock is a World resource. Drivers and handlers access it via
`Res<Clock>`. Swapping the clock implementation changes time behavior
for the entire system without modifying any handler code.

## File Map

| File | Role |
|------|------|
| `world.rs` | World, WorldBuilder, ResourceId, ResourceCell |
| `resource.rs` | Res<T>, ResMut<T>, Local<T>, Option wrappers |
| `system.rs` | SystemParam trait, Param derive support |
| `handler.rs` | Handler<E> trait, IntoHandler, OpaqueHandler |
| `callback.rs` | Callback<C, F, P>, CtxFree, HandlerFn |
| `pipeline.rs` | Pipeline builder, chain nodes, combinators |
| `dag.rs` | DAG builder, branching combinators |
| `combinator.rs` | IntoStep, IntoRefStep, IntoProducer traits |
| `driver.rs` | Installer trait, Plugin trait |
| `template.rs` | Blueprint, HandlerTemplate, CallbackTemplate |
| `view.rs` | View trait for lifetime-erased resource access |
| `clock.rs` | Clock resource, realtime/test/historical modes |
| `catch_unwind.rs` | CatchAssertUnwindSafe, panic counter |
| `adapt.rs` | Adapter types for handler composition |
| `tuples.rs` | SystemParam impls for tuples (arity 0-12) |
