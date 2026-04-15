# Cookbook: Writing a Strategy Handler on nexus-rt

**Goal:** build a simple mean-reverting strategy on `nexus-rt`. The
strategy takes market data events, computes a signal with streaming
stats, makes a decision with fixed-point prices, and emits order
intents on a channel. Then we show how to test it.

**Crates used:**
`nexus-rt` (handlers, World, Pipeline), `nexus-stats` (EMA + z-score),
`nexus-decimal` (price arithmetic), `nexus-collections` (order book
via `RbTree` — optional), `nexus-id` (client order IDs),
`nexus-queue` (outbound order intents).

Read `nexus-rt/docs/INDEX.md` alongside this for the runtime
internals. This cookbook is the "how do the pieces fit together"
view; the crate docs are the deep dive.

---

## 1. Mental model

`nexus-rt` is a **dispatch primitive**, not a framework. You build
your own loop. The runtime gives you:

- A `World` that owns typed resources (pre-resolved, one `NonNull<u8>`
  deref to fetch).
- `Handler<E>` — `fn(resources..., &E)` — a typed event handler.
- `Res<T>` / `ResMut<T>` / `Local<T>` — parameter wrappers for
  handler arguments.
- `Pipeline` / `DAG` — composition of handlers into flows.
- Drivers that pre-resolve handlers at setup time and poll them in
  your main loop.

No async, no work-stealing, no hidden executor. The event loop is a
plain `while` in your `main`.

---

## 2. Resources

Strategy state lives in the `World` as resources. Anything the
handler reads or writes is a resource.

```rust
use nexus_stats::smoothing::EmaF64;
use nexus_stats::statistics::WelfordF64;
use nexus_decimal::Decimal;
use nexus_ascii::AsciiString16;
use nexus_rt::Resource;

// fixed: nexus-decimal has no `d8` module. Users declare their own aliases.
pub type D64 = Decimal<i64, 8>;

// fixed: nexus-rt requires `#[derive(Resource)]` on anything stored in the World.

/// Streaming fair value estimate (fast EMA of midprice).
#[derive(Resource)]
pub struct FairValue {
    pub ema: EmaF64,
}

/// Price dispersion used to size signal (z-score of residual).
#[derive(Resource)]
pub struct Dispersion {
    pub stats: WelfordF64,
}

/// Strategy config — static, read only from handlers.
#[derive(Resource)]
pub struct Config {
    pub symbol: AsciiString16,
    pub entry_z: f64,
    pub exit_z: f64,
    pub max_position: D64,
}

// fixed: there is no `nexus_id::Snowflake64` id type — the typed id is
// `SnowflakeId64<TS, WK, SQ>`. Pick a layout in your application.
pub type StrategyIdLayout = nexus_id::Snowflake64<42, 6, 16>;
pub type StrategyId = nexus_id::SnowflakeId64<42, 6, 16>;

/// Mutable position + pending-intent tracking.
#[derive(Resource)]
pub struct PositionBook {
    pub size: D64,
    pub avg_price: D64,
    pub last_intent_id: Option<StrategyId>,
}
```

A couple of principles:

- **One struct, one ownership unit.** `FairValue` wraps the EMA so
  that `ResMut<FairValue>` stamps change-detection correctly.
- **Config is separate from state.** It's `Res<Config>`, never
  `ResMut<Config>`. Static separation = no accidental mutation.
- **`nexus-decimal`** (`D64` = `Decimal<i64, 8>`) gives you exact
  price arithmetic. No float drift in position tracking.

---

## 3. The handler

A handler in `nexus-rt` is a plain function. The runtime injects
resources as parameters. The event is always the **last** parameter.

```rust
use nexus_rt::{Res, ResMut, Local, Resource};
use nexus_queue::spsc;

/// A quote tick coming in from the market data gateway.
pub struct QuoteTick {
    pub symbol: AsciiString16,
    pub bid: D64,
    pub ask: D64,
    pub ts_ns: u64,
}

/// An order intent emitted by the strategy.
#[derive(Clone, Copy)]
pub struct OrderIntent {
    pub id: StrategyId,
    pub symbol: AsciiString16,
    pub side: Side,
    pub price: D64,
    pub qty: D64,
}

#[derive(Clone, Copy)]
pub enum Side { Buy, Sell }

// fixed: no `Snowflake64Generator` type — the generator is
// `Snowflake64<TS, WK, SQ>` (see StrategyIdLayout above). Wrapped in a
// Resource newtype for the World.
#[derive(Resource)]
pub struct IdGen {
    pub gen: StrategyIdLayout,
    pub epoch: std::time::Instant,
}

/// The strategy handler. Resources as named arguments, event last.
///
/// fixed: `Handler::run` takes events by value, and nexus-queue SPSC
/// `Producer::push(&self, T)` already takes `&self`. No UnsafeCell shim
/// is required — put the Producer directly in the World as a Resource.
pub fn on_quote(
    cfg: Res<Config>,
    mut fair: ResMut<FairValue>,
    mut disp: ResMut<Dispersion>,
    mut pos: ResMut<PositionBook>,
    out: Res<OrderIntentTx>,
    mut ids: ResMut<IdGen>,
    tick: QuoteTick,
) {
    if tick.symbol != cfg.symbol { return; }

    // 1. Update fair value from midprice.
    let mid = (tick.bid.to_f64() + tick.ask.to_f64()) * 0.5;
    let _ = fair.ema.update(mid);
    let Some(f) = fair.ema.value() else { return; };

    // 2. Update residual dispersion.
    let residual = mid - f;
    let _ = disp.stats.update(residual);
    // fixed: Welford method is `std_dev()`, not `stddev()`.
    let Some(sigma) = disp.stats.std_dev() else { return; };
    if sigma <= 0.0 { return; }

    // 3. Z-score → signal.
    let z = residual / sigma;

    // 4. Decide.
    let neg_max = D64::ZERO.checked_sub(cfg.max_position).unwrap();
    let intent = if z > cfg.entry_z && pos.size > neg_max {
        Some(Side::Sell)   // mean revert: price too high, sell
    } else if z < -cfg.entry_z && pos.size < cfg.max_position {
        Some(Side::Buy)
    } else if z.abs() < cfg.exit_z && pos.size != D64::ZERO {
        Some(if pos.size > D64::ZERO { Side::Sell } else { Side::Buy })
    } else {
        None
    };

    // 5. Emit.
    if let Some(side) = intent {
        let px = match side {
            Side::Buy => tick.ask,
            Side::Sell => tick.bid,
        };
        let qty = cfg.max_position; // simple sizing
        // fixed: Snowflake64::next_id takes a `tick: u64`.
        let tick_ms =
            (std::time::Instant::now() - ids.epoch).as_millis() as u64;
        let id = ids.gen.next_id(tick_ms).expect("sequence exhausted");
        let intent = OrderIntent {
            id,
            symbol: cfg.symbol,
            side,
            price: px,
            qty,
        };
        // fixed: spsc::Producer::push is `&self` — no UnsafeCell needed.
        let _ = out.0.push(intent);
        pos.last_intent_id = Some(intent.id);
    }
}

/// Resource wrapper for the outbound SPSC producer.
#[derive(Resource)]
pub struct OrderIntentTx(pub spsc::Producer<OrderIntent>);
```

Notes on the shape:

- **Arguments are resolved once at build time, not per dispatch.**
  `IntoHandler` walks the signature and pre-resolves each `Res<T>`
  to a `ResourceId` (one `NonNull<u8>` deref). Per-dispatch cost is
  a handful of cycles per parameter.
- **`UnsafeCell` shim for the SPSC producer.** The producer needs
  `&mut self` but only one handler writes to it, and `nexus-rt` is
  single-threaded. You either wrap in `UnsafeCell` (as above) or
  use `ResMut<spsc::Producer<...>>`. The latter is more idiomatic;
  the former is what you write when the same producer is shared
  across multiple handlers that each produce intents.
- **`Local<T>`** is a per-handler state slot. Use it when the state
  doesn't need to be shared — scratch buffers, counters. It's
  stored in the `Callback`, not the `World`, so it's truly private.

---

## 4. Composing with Pipeline

A single handler is rarely the whole flow. `Pipeline` composes
handlers into a chain with combinators like `.then()`, `.guard()`,
`.tap()`, `.filter()`, `.map()`.

```rust
// fixed: entrypoint is `PipelineBuilder`, not `Pipeline`.
use nexus_rt::PipelineBuilder;

pub fn build_pipeline(reg: &nexus_rt::Registry)
    -> impl nexus_rt::Handler<QuoteTick>
{
    PipelineBuilder::<QuoteTick>::new()
        // Drop ticks for symbols we don't trade.
        .guard(is_our_symbol, reg)
        // Update the fair value.
        .tap(update_fair_value, reg)
        // Compute signal and emit intent (or not).
        .then(on_quote, reg)
        .build()
}

// `.guard` / `.tap` use IntoRefStep — event is passed by reference.
fn is_our_symbol(cfg: Res<Config>, tick: &QuoteTick) -> bool {
    tick.symbol == cfg.symbol
}

fn update_fair_value(
    mut fair: ResMut<FairValue>,
    tick: &QuoteTick,
) {
    let mid = (tick.bid.to_f64() + tick.ask.to_f64()) * 0.5;
    let _ = fair.ema.update(mid);
}
```

Each stage is a named function (not a closure — closures don't
support the double-bound HRTB inference that `IntoHandler` relies
on). Opaque closures that take `&mut World` directly work but skip
the typed-resource machinery.

See `nexus-rt/docs/pipelines.md` for the full combinator list.

---

## 5. World setup

```rust
use nexus_rt::WorldBuilder;

fn build_world() -> (nexus_rt::World, spsc::Consumer<OrderIntent>) {
    let (intent_tx, intent_rx) = spsc::ring_buffer::<OrderIntent>(1024);

    // fixed: `WorldBuilder::register`, not `insert`.
    let mut wb = WorldBuilder::new();
    wb.register(Config {
        symbol: AsciiString16::try_from_str("BTC-USDT").unwrap(),
        entry_z: 2.0,
        exit_z: 0.5,
        max_position: D64::from_i64(1).unwrap(),
    });
    wb.register(FairValue {
        // fixed: `EmaF64::builder().halflife(...).build().unwrap()`.
        ema: EmaF64::builder().halflife(5.0).build().unwrap(),
    });
    wb.register(Dispersion { stats: WelfordF64::new() });
    wb.register(PositionBook {
        size: D64::ZERO,
        avg_price: D64::ZERO,
        last_intent_id: None,
    });
    wb.register(OrderIntentTx(intent_tx));
    wb.register(IdGen {
        gen: StrategyIdLayout::new(/* worker */ 1),
        epoch: std::time::Instant::now(),
    });

    (wb.build(), intent_rx)
}
```

All allocation happens here. Once `build()` returns, the `World`
owns every resource in a stable location and every handler can
pre-resolve its `ResourceId`s against it.

---

## 6. The main loop

```rust
fn main() {
    // fixed: build the pipeline from the WorldBuilder's registry
    // BEFORE calling `build()`, so IDs resolve against the same world.
    let (intent_tx, intent_rx) = spsc::ring_buffer::<OrderIntent>(1024);
    let mut wb = WorldBuilder::new();
    // ... register all resources as in build_world() ...
    # wb.register(OrderIntentTx(intent_tx));
    let handler_template = build_pipeline(wb.registry());
    let mut world = wb.build();
    let mut handler = handler_template;

    let mut md_source = connect_to_market_data(); // your source

    loop {
        // Poll market data — blocking or with timeout.
        let Some(tick) = md_source.next() else { continue };

        // fixed: Handler::run takes the event by value.
        nexus_rt::Handler::run(&mut handler, &mut world, tick);

        // Drain any emitted intents to the exchange connector.
        // fixed: spsc::Consumer::pop (not try_pop).
        while let Some(intent) = intent_rx.pop() {
            send_to_exchange(intent);
        }
    }
}
```

Not shown: actually driving the market data source (see the
[market-data-gateway cookbook](./cookbook-market-data-gateway.md))
and sending intents to an exchange (see the
[exchange-connection cookbook](./cookbook-exchange-connection.md)).

The point is that the **main loop is yours**. `nexus-rt` doesn't
own the poll — it's just giving you a well-typed dispatcher.

---

## 7. Order book as a Resource (optional)

If the strategy needs a local order book (limit levels, not just
top of book), `nexus-collections` gives you slab-backed sorted maps:

```rust
// fixed: module is `nexus_collections::rbtree`.
use nexus_collections::rbtree::RbTree;
use nexus_slab::bounded::BoundedSlab;

pub struct LocalBook {
    pub bids: RbTree<D64, Level>,
    pub asks: RbTree<D64, Level>,
    pub storage: BoundedSlab<Level>,
}

pub struct Level {
    pub price: D64,
    pub qty: D64,
    pub orders: u32,
}
```

Reads are O(log n), inserts/removes are O(log n), iteration from
best is O(1) per step. For very shallow books a `Heap` on top of a
slab is faster. Pick based on access pattern.

---

## 8. Testing with a Test Harness

`nexus-rt` ships with a `TestHarness` that lets you dispatch events
against a pre-built world without a main loop. This is the fastest
way to test a strategy.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nexus_rt::testing::TestHarness;

    #[test]
    fn buys_when_price_deeply_below_fair() {
        // fixed: TestHarness is constructed from a WorldBuilder, and
        // `dispatch` takes `(handler, event)` — event by value.
        let (intent_tx, intent_rx) = spsc::ring_buffer::<OrderIntent>(1024);
        let mut wb = WorldBuilder::new();
        wb.register(Config {
            symbol: AsciiString16::try_from_str("BTC-USDT").unwrap(),
            entry_z: 2.0,
            exit_z: 0.5,
            max_position: D64::from_i64(1).unwrap(),
        });
        wb.register(FairValue {
            ema: EmaF64::builder().halflife(5.0).build().unwrap(),
        });
        wb.register(Dispersion { stats: WelfordF64::new() });
        wb.register(PositionBook {
            size: D64::ZERO, avg_price: D64::ZERO, last_intent_id: None,
        });
        wb.register(OrderIntentTx(intent_tx));
        wb.register(IdGen {
            gen: StrategyIdLayout::new(1),
            epoch: std::time::Instant::now(),
        });

        let mut handler = build_pipeline(wb.registry());
        let mut h = TestHarness::new(wb);

        let sym = AsciiString16::try_from_str("BTC-USDT").unwrap();
        // Warm up fair value around 100.
        for _ in 0..200 {
            h.dispatch(&mut handler, QuoteTick {
                symbol: sym,
                bid: D64::from_f64(99.9).unwrap(),
                ask: D64::from_f64(100.1).unwrap(),
                ts_ns: 0,
            });
        }

        // Now drop price dramatically.
        h.dispatch(&mut handler, QuoteTick {
            symbol: sym,
            bid: D64::from_f64(90.0).unwrap(),
            ask: D64::from_f64(90.1).unwrap(),
            ts_ns: 0,
        });

        let intent = intent_rx.pop().expect("expected a buy");
        assert!(matches!(intent.side, Side::Buy));
    }
}
```

The test harness is synchronous and deterministic. Strategy unit
tests should not involve tokio, threads, or real IO — if they do,
you're testing the executor, not the strategy.

---

## 9. Gotchas

- **Named functions only.** `.then(|cfg, tick| ...)` will fail
  to infer types. Define `fn on_quote(...)` and pass the name.
- **`Res<T>` is read-only, `ResMut<T>` stamps a change tick.** If
  another handler listens for changes via `Res::is_changed()`,
  they'll only see it if you took `ResMut`.
- **Change detection ≠ firing.** Event handlers fire on every event.
  Use `Res::is_changed()` inside the handler if you want "only
  when config changed" semantics.
- **Resources are singleton.** If you need per-symbol state, the
  resource is a `HashMap<Symbol, State>`, and the handler indexes
  into it.
- **No dynamic handler add/remove.** Register everything at build
  time. That's by design — it's what lets pre-resolution work.

---

## Further reading

- `nexus-rt/docs/handlers.md` — handler trait and `IntoHandler`
  inference
- `nexus-rt/docs/world.md` — resource lifecycle
- `nexus-rt/docs/pipelines.md` — full combinator catalog
- `nexus-rt/docs/dag.md` — DAGs for multi-input flows
- `nexus-rt/docs/templates.md` — stamped handler factories
- `nexus-rt/docs/testing-guide.md` — TestHarness in depth
- `nexus-stats/docs/` — streaming statistics
- `nexus-decimal/docs/` — fixed-point arithmetic
