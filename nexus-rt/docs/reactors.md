# Reactors — Interest-Based Dispatch

Reactors are dynamic, interest-based handlers: a reactor subscribes to
one or more data sources, and the runtime dispatches it whenever any of
its sources is marked. Use reactors when handlers and sources are
many-to-many and the wiring isn't known at build time.

**Requires the `reactors` feature flag.**

---

## When to Use Reactors

### Use handlers when:
- The dispatch is direct (one event → one handler)
- Wiring is known at compile time
- You want zero-cost monomorphized dispatch

### Use reactors when:
- One source feeds many handlers (1-to-N) AND the set of handlers changes at runtime
- One handler reacts to many sources (N-to-1) AND the set of sources changes
- Handlers register/deregister dynamically (per-session, per-instrument)
- You need per-instance dispatch with metadata (which symbol fired, which session)

A typical case: a market data feed has thousands of instruments, each
with subscribers that come and go. Handlers can't be wired statically —
new symbols are added throughout the day, subscribers connect and drop.

---

## Core Concepts

```
DataSource     ← represents a thing that can be "marked" (e.g., a symbol)
SourceRegistry ← maps domain keys (symbol_id, session_id) to DataSources
Reactor        ← a handler that subscribes to one or more sources
ReactorNotify  ← the dispatch hub, holds sources and reactors
Token          ← opaque handle to a registered reactor
```

The flow:
1. At setup, register data sources you'll mark
2. Register reactors that subscribe to those sources
3. At runtime, mark a source when its data changes
4. Poll `ReactorNotify` — it returns tokens for reactors whose sources were marked
5. Dispatch each token's reactor

---

## Minimal Example

```rust
use nexus_rt::{
    DataSource, IntoReactor, Reactor, ReactorNotify, Res, ResMut, SourceRegistry,
    WorldBuilder,
};

#[derive(nexus_rt::Resource, Default)]
struct UpdateCount(u64);

// A reactor function — like a handler, but with `&mut C` context first
fn on_quote_update(
    _ctx: &mut (),
    mut count: ResMut<UpdateCount>,
) {
    count.0 += 1;
}

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register(UpdateCount(0));

    // ReactorNotify is a resource — register it with capacities
    wb.register(ReactorNotify::new(
        16,  // source capacity
        16,  // reactor capacity
    ));

    let mut world = wb.build();

    // Set up a data source for "AAPL"
    let aapl_source = {
        let notify = world.resource_mut::<ReactorNotify>();
        notify.register_source()
    };

    // Register a reactor that subscribes to AAPL
    let registry = world.registry().clone();
    let token = {
        let notify = world.resource_mut::<ReactorNotify>();
        let reactor = on_quote_update.into_reactor((), &registry);
        let token = notify.create_reactor();
        let mut registration = notify.insert_reactor(token, Box::new(reactor));
        registration.subscribe(aapl_source);
        token
    };

    // Mark the source — simulating a quote arriving for AAPL
    world.resource_mut::<ReactorNotify>().mark(aapl_source);

    // Poll: returns tokens of reactors whose sources were marked
    let to_dispatch = world.resource_mut::<ReactorNotify>().poll(&mut world);
    // ... then dispatch each token via your runtime's reactor execution

    let _ = (token, to_dispatch);
}
```

The actual dispatch loop (executing tokens) lives in the runtime layer
that uses reactors — typically your application's poll loop.

---

## SourceRegistry: Mapping Domain Keys to Sources

`DataSource` is an opaque integer. To find a source by a domain-specific
key (symbol name, session ID), use `SourceRegistry`:

```rust
use nexus_rt::SourceRegistry;

#[derive(Hash, Eq, PartialEq)]
struct SymbolKey(u32);

let mut wb = WorldBuilder::new();
wb.register(SourceRegistry::default());
wb.register(ReactorNotify::new(1024, 1024));
let mut world = wb.build();

// At setup: create a source per symbol and store the mapping
{
    let notify = world.resource_mut::<ReactorNotify>();
    let aapl_source = notify.register_source();
    let msft_source = notify.register_source();

    let registry = world.resource_mut::<SourceRegistry>();
    registry.insert(SymbolKey(1), aapl_source);
    registry.insert(SymbolKey(2), msft_source);
}

// At runtime: look up by symbol, mark
let registry = world.resource::<SourceRegistry>();
if let Some(source) = registry.get(&SymbolKey(1)) {
    world.resource_mut::<ReactorNotify>().mark(source);
}
```

`SourceRegistry::insert` accepts any `Hash + Eq + 'static` key, so you
can use whatever your domain naturally uses.

---

## Per-Instance Reactors with Context

Reactors typically need per-instance state (which symbol they handle, a
buffer for accumulated data). Pass that as the `ctx` argument:

```rust
struct SymbolReactor {
    symbol_id: u32,
    last_mid: f64,
    quote_count: u64,
}

fn handle_symbol_quote(
    ctx: &mut SymbolReactor,
    book: ResMut<OrderBook>,
) {
    ctx.quote_count += 1;
    let mid = book.mid_for_symbol(ctx.symbol_id);
    if mid != ctx.last_mid {
        ctx.last_mid = mid;
        // ... react to mid change
    }
}

// Create per-symbol reactors
for symbol_id in 0..1000 {
    let reactor = handle_symbol_quote.into_reactor(
        SymbolReactor {
            symbol_id,
            last_mid: 0.0,
            quote_count: 0,
        },
        world.registry(),
    );

    let notify = world.resource_mut::<ReactorNotify>();
    let token = notify.create_reactor();
    let source = notify.register_source();
    notify.insert_reactor(token, Box::new(reactor)).subscribe(source);

    world.resource_mut::<SourceRegistry>()
        .insert(SymbolKey(symbol_id), source);
}
```

Each reactor instance owns its `ctx`. The function signature is
`fn(ctx: &mut C, params...)`. No Local<T> needed — the ctx IS the state.

---

## Subscribing to Multiple Sources

A single reactor can subscribe to several sources. It fires when any of
them is marked:

```rust
let token = notify.create_reactor();
let mut reg = notify.insert_reactor(token, Box::new(my_reactor));
reg.subscribe(source_a);
reg.subscribe(source_b);
reg.subscribe(source_c);
// my_reactor fires if any of A, B, or C is marked
```

The `poll()` deduplicates — even if multiple subscribed sources are
marked in the same cycle, the reactor token only appears once in the
returned list.

---

## Deregistration with DeferredRemovals

You can't deregister a reactor mid-frame (it might be in the dispatch
list). Use `DeferredRemovals` to queue removal until the end of the
cycle:

```rust
use nexus_rt::DeferredRemovals;

let mut wb = WorldBuilder::new();
wb.register(ReactorNotify::new(1024, 1024));
wb.register(DeferredRemovals::default());
// ... build, register reactors

// Inside a reactor, you decide to remove yourself or another reactor
fn cleanup_reactor(
    ctx: &mut MyContext,
    mut removals: ResMut<DeferredRemovals>,
) {
    if ctx.should_terminate {
        removals.deregister(ctx.my_token);
    }
}

// At the end of the poll cycle:
{
    let mut removals = std::mem::take(&mut *world.resource_mut::<DeferredRemovals>());
    let notify = world.resource_mut::<ReactorNotify>();
    removals.process(notify);
}
```

This pattern prevents the "remove myself while iterating" bug.

---

## PipelineReactor: Wrapping Pipelines as Reactors

If you want a pipeline to react to source marking, wrap it:

```rust
use nexus_rt::{PipelineBuilder, PipelineReactor};

let pipeline = PipelineBuilder::<MarketTick>::new()
    .then(parse, world.registry())
    .then(validate, world.registry())
    .dispatch(book_updater);

let reactor = PipelineReactor::new(pipeline);
let token = notify.create_reactor();
notify.insert_reactor(token, Box::new(reactor)).subscribe(source);
```

The pipeline runs every time the source is marked. Useful when the
reaction is itself a multi-stage process.

---

## Complete Example: Market Data Feed with Per-Instrument Reactors

```rust
use nexus_rt::{
    DataSource, IntoReactor, ReactorNotify, Res, ResMut,
    SourceRegistry, WorldBuilder,
};

#[derive(Clone, Copy)]
struct Quote {
    symbol_id: u32,
    bid: f64,
    ask: f64,
}

#[derive(nexus_rt::Resource)]
struct QuoteBook {
    quotes: std::collections::HashMap<u32, Quote>,
}

#[derive(Hash, Eq, PartialEq)]
struct SymbolKey(u32);

// Per-instrument reactor state
struct InstrumentReactor {
    symbol_id: u32,
    last_mid: f64,
}

// The reactor function: ctx first, then params
fn on_instrument_update(
    ctx: &mut InstrumentReactor,
    book: Res<QuoteBook>,
) {
    if let Some(quote) = book.quotes.get(&ctx.symbol_id) {
        let mid = (quote.bid + quote.ask) / 2.0;
        if (mid - ctx.last_mid).abs() > 0.01 {
            ctx.last_mid = mid;
            // ... act on price change (notify strategies, update vols, etc.)
        }
    }
}

fn setup_feed(world: &mut nexus_rt::World, symbols: &[u32]) {
    let registry_clone = world.registry().clone();

    for &symbol_id in symbols {
        // Allocate a data source for this symbol
        let source = world.resource_mut::<ReactorNotify>().register_source();

        // Map the symbol to its source
        world.resource_mut::<SourceRegistry>()
            .insert(SymbolKey(symbol_id), source);

        // Create the per-instrument reactor
        let reactor = on_instrument_update.into_reactor(
            InstrumentReactor { symbol_id, last_mid: 0.0 },
            &registry_clone,
        );

        let notify = world.resource_mut::<ReactorNotify>();
        let token = notify.create_reactor();
        notify.insert_reactor(token, Box::new(reactor)).subscribe(source);
    }
}

// On every quote arrival from the feed:
fn on_quote_arrived(world: &mut nexus_rt::World, quote: Quote) {
    // Update the book
    world.resource_mut::<QuoteBook>().quotes.insert(quote.symbol_id, quote);

    // Mark the source for this symbol — this wakes the reactor
    let source = world.resource::<SourceRegistry>()
        .get(&SymbolKey(quote.symbol_id));

    if let Some(source) = source {
        world.resource_mut::<ReactorNotify>().mark(source);
    }
}
```

---

## Performance

- `mark()` is O(1) — just sets a bit
- `poll()` returns tokens for marked reactors via bitvector scan
- Per-reactor dispatch is one indirect call (`Box<dyn Reactor>`)
- Dedup is automatic — a reactor subscribed to N marked sources fires once

For hot paths (millions of marks per second), reactors add ~10-30 cycles
per mark vs direct handler dispatch. The tradeoff is dynamic wiring —
you get to register and deregister at runtime.

---

## See Also

- [handlers.md](handlers.md) — Static dispatch (faster, less flexible)
- [callbacks.md](callbacks.md) — Per-instance state without reactors
- [pipelines.md](pipelines.md) — Combine with PipelineReactor
