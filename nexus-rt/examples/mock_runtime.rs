//! Mock runtime example.
//!
//! Demonstrates how nexus-rt primitives compose into a realistic
//! single-threaded runtime. The runtime loop phases are explicit
//! in `main()` so you can see exactly what happens and in what order.
//!
//! # Runtime loop
//!
//! ```text
//! loop {
//!     // 1. Poll — pull raw events from data source into a local buffer
//!     // 2. Dispatch — drain buffer, fire each event into registered systems
//!     // 3. Post-dispatch — process accumulated signals, clear event buffers
//! }
//! ```
//!
//! All three phases are written out explicitly below — not hidden behind
//! a `tick()` method. This is intentional: the user should see the flow.
//!
//! # Architecture
//!
//! - **DataFeed** — simulated data source (VecDeque). In a real runtime
//!   this would be a mio poll + socket reads + protocol parsing.
//! - **Dispatcher** — holds registered system callbacks. Provides
//!   `fire_tick` and `fire_post` to run them against `&mut World`.
//! - **Domain resources** — PriceCache, OrderCount, Events<TradeSignal>
//!   live in World and are accessed by systems via `Res`/`ResMut`.
//!
//! # Features demonstrated
//!
//! - `Res<T>` / `ResMut<T>` — shared and exclusive resource access
//! - `Local<T>` — per-system state (tick counter on check_signals)
//! - `Option<Res<T>>` — optional dependencies (risk limits may not exist)
//! - `EventWriter<T>` / `EventReader<T>` — inter-system event buffers
//! - `register_default` — ergonomic resource registration
//! - `with_mut` — safe driver extraction pattern
//! - `Box<dyn System<E>>` — type-erased heterogeneous dispatch
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example mock_runtime
//! ```

use std::collections::{HashMap, VecDeque};

use nexus_rt::{
    EventReader, EventWriter, Events, IntoSystem, Local, Res, ResMut, System, WorldBuilder,
};

// =============================================================================
// Domain types (World resources)
// =============================================================================

struct PriceCache {
    prices: HashMap<&'static str, f64>,
}

impl PriceCache {
    fn new() -> Self {
        Self {
            prices: HashMap::new(),
        }
    }
}

struct OrderCount(u64);

/// Optional resource — may or may not be registered.
/// Systems that depend on it use `Option<Res<RiskLimits>>`.
struct RiskLimits {
    max_signals_per_tick: u64,
}

// =============================================================================
// Event types
// =============================================================================

#[derive(Clone)]
struct MarketTick {
    symbol: &'static str,
    price: f64,
}

struct TradeSignal {
    symbol: &'static str,
}

// =============================================================================
// Data feed — simulated data source
// =============================================================================

/// Simulated data source. In a real runtime this would be a mio poll
/// reading from sockets and parsing wire protocol into typed events.
struct DataFeed {
    queue: VecDeque<MarketTick>,
}

impl DataFeed {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    fn push(&mut self, tick: MarketTick) {
        self.queue.push_back(tick);
    }

    /// Drain all pending events into the provided buffer.
    fn drain_into(&mut self, buf: &mut Vec<MarketTick>) {
        buf.extend(self.queue.drain(..));
    }
}

// =============================================================================
// Dispatcher — holds registered system callbacks
// =============================================================================

/// Holds registered system callbacks. The dispatcher doesn't own the
/// loop — it just provides methods to fire events into systems.
struct Dispatcher {
    on_tick: Vec<Box<dyn System<MarketTick>>>,
    on_post: Vec<Box<dyn System<()>>>,
}

impl Dispatcher {
    fn new() -> Self {
        Self {
            on_tick: Vec::new(),
            on_post: Vec::new(),
        }
    }

    fn register_tick<P>(
        &mut self,
        sys: impl IntoSystem<MarketTick, P>,
        registry: &nexus_rt::Registry,
    ) {
        self.on_tick.push(Box::new(sys.into_system(registry)));
    }

    fn register_post<P>(&mut self, sys: impl IntoSystem<(), P>, registry: &nexus_rt::Registry) {
        self.on_post.push(Box::new(sys.into_system(registry)));
    }

    /// Fire one event into all tick systems.
    fn fire_tick(&mut self, world: &mut nexus_rt::World, event: MarketTick) {
        for sys in &mut self.on_tick {
            sys.run(world, event.clone());
        }
    }

    /// Fire post-dispatch systems (once per tick, no event payload).
    fn fire_post(&mut self, world: &mut nexus_rt::World) {
        for sys in &mut self.on_post {
            sys.run(world, ());
        }
    }
}

// =============================================================================
// System callbacks — plain functions
// =============================================================================

/// Compares against previous price, emits signal if delta > threshold.
///
/// Uses `Local<u64>` to track how many ticks this system has processed
/// across all dispatch cycles — without polluting World.
fn check_signals(
    cache: Res<PriceCache>,
    mut signals: EventWriter<TradeSignal>,
    mut tick_count: Local<u64>,
    tick: MarketTick,
) {
    *tick_count += 1;
    if let Some(&prev) = cache.prices.get(tick.symbol) {
        let delta = (tick.price - prev).abs();
        if delta > 50.0 {
            println!(
                "  [check_signals] tick #{}: {} moved {:.2} — emitting signal",
                *tick_count, tick.symbol, delta
            );
            signals.send(TradeSignal {
                symbol: tick.symbol,
            });
        }
    }
}

/// Updates the price cache with the latest tick.
fn update_prices(mut cache: ResMut<PriceCache>, tick: MarketTick) {
    cache.prices.insert(tick.symbol, tick.price);
    println!("  [update_prices] {} = {:.2}", tick.symbol, tick.price);
}

/// Reads accumulated trade signals and increments the order count.
///
/// Uses `Option<Res<RiskLimits>>` — if risk limits are registered,
/// respects the per-tick cap. If not, signals are unlimited.
fn count_trades(
    signals: EventReader<TradeSignal>,
    mut count: ResMut<OrderCount>,
    limits: Option<Res<RiskLimits>>,
    _: (),
) {
    let cap = limits
        .as_ref()
        .map(|l| l.max_signals_per_tick)
        .unwrap_or(u64::MAX);

    let mut accepted = 0u64;
    for signal in signals.iter() {
        if accepted >= cap {
            println!(
                "  [count_trades] SKIPPED signal for {} (risk limit: {})",
                signal.symbol, cap
            );
            continue;
        }
        count.0 += 1;
        accepted += 1;
        println!("  [count_trades] signal #{} for {}", count.0, signal.symbol);
    }
}

// =============================================================================
// main — the runtime loop is here, not hidden in a method
// =============================================================================

fn main() {
    // -- Build ----------------------------------------------------------------

    let mut builder = WorldBuilder::new();
    builder
        .register(PriceCache::new())
        .register(OrderCount(0))
        .register(RiskLimits {
            max_signals_per_tick: 2,
        })
        .register_default::<Events<TradeSignal>>();

    let mut dispatcher = Dispatcher::new();
    dispatcher.register_tick(check_signals, builder.registry());
    dispatcher.register_tick(update_prices, builder.registry());
    dispatcher.register_post(count_trades, builder.registry());

    builder.register(DataFeed::new()).register(dispatcher);
    let mut world = builder.build();

    // -- Load simulated data --------------------------------------------------

    world.with_mut::<DataFeed, _>(|feed, _| {
        for tick in [
            MarketTick {
                symbol: "BTC",
                price: 50_000.0,
            },
            MarketTick {
                symbol: "ETH",
                price: 3_000.0,
            },
            MarketTick {
                symbol: "BTC",
                price: 50_100.0,
            },
            MarketTick {
                symbol: "ETH",
                price: 3_010.0,
            },
            MarketTick {
                symbol: "BTC",
                price: 49_900.0,
            },
        ] {
            feed.push(tick);
        }
    });

    // -- Tick 1 ---------------------------------------------------------------
    //
    // The three phases are explicit here. In a real runtime you'd wrap this
    // in a `loop { ... }` with a mio poll at the top.

    println!("=== Tick 1 ===\n");

    // Phase 1: Poll — pull events from the data source into a local buffer.
    //
    // We yank DataFeed out of World via with_mut, drain it into a local Vec,
    // then drop the borrow. The buffer is ours — no World borrow held.
    let mut event_buf = Vec::new();
    world.with_mut::<DataFeed, _>(|feed, _| {
        feed.drain_into(&mut event_buf);
    });
    println!("[poll] {} events buffered", event_buf.len());

    // Phase 2: Dispatch — fire each event into registered systems.
    //
    // We yank the Dispatcher out of World so systems can access all other
    // resources via &mut World without aliasing the dispatcher itself.
    world.with_mut::<Dispatcher, _>(|dispatcher, world| {
        for tick in event_buf.drain(..) {
            dispatcher.fire_tick(world, tick);
        }
    });

    // Phase 3: Post-dispatch — run post-tick systems, then clear event buffers.
    //
    // Post-dispatch systems (like count_trades) read accumulated events
    // from the dispatch phase. After they run, we clear the buffers.
    world.with_mut::<Dispatcher, _>(|dispatcher, world| {
        dispatcher.fire_post(world);
    });
    world.resource_mut::<Events<TradeSignal>>().clear();

    // -- Tick 2 ---------------------------------------------------------------

    world.with_mut::<DataFeed, _>(|feed, _| {
        feed.push(MarketTick {
            symbol: "BTC",
            price: 50_200.0, // +300 from 49900 → signal
        });
        feed.push(MarketTick {
            symbol: "ETH",
            price: 3_015.0, // +5 from 3010 → no signal
        });
        feed.push(MarketTick {
            symbol: "BTC",
            price: 50_500.0, // +300 from 50200 → signal (will hit risk cap)
        });
    });

    println!("\n=== Tick 2 ===\n");

    // Same three phases — explicit and visible.
    world.with_mut::<DataFeed, _>(|feed, _| {
        feed.drain_into(&mut event_buf);
    });
    println!("[poll] {} events buffered", event_buf.len());

    world.with_mut::<Dispatcher, _>(|dispatcher, world| {
        for tick in event_buf.drain(..) {
            dispatcher.fire_tick(world, tick);
        }
    });

    world.with_mut::<Dispatcher, _>(|dispatcher, world| {
        dispatcher.fire_post(world);
    });
    world.resource_mut::<Events<TradeSignal>>().clear();

    // -- Results --------------------------------------------------------------

    println!("\n=== Results ===\n");

    {
        let cache = world.resource::<PriceCache>();
        println!("PriceCache:");
        for (sym, price) in &cache.prices {
            println!("  {sym} = {price:.2}");
        }
    }

    let count = world.resource::<OrderCount>().0;
    println!("OrderCount: {count}");
    // Tick 1: 2 BTC signals (50100 moved 100, 49900 moved 200)
    // Tick 2: 2 BTC signals but risk cap is 2 per tick
    //   - 50200 moved 300 → signal #3
    //   - 50500 moved 300 → signal #4, but cap=2 → SKIPPED
    // Wait, let's trace carefully:
    // Tick 1 check_signals: BTC 50100 vs prev 50000 → delta 100 > 50 → signal
    //                       ETH 3010 vs prev 3000 → delta 10 < 50 → no signal
    //                       BTC 49900 vs prev 50100 → delta 200 > 50 → signal
    // count_trades: 2 signals, cap 2 → both accepted. OrderCount = 2
    //
    // Tick 2 check_signals: BTC 50200 vs prev 49900 → delta 300 > 50 → signal
    //                       ETH 3015 vs prev 3010 → delta 5 < 50 → no signal
    //                       BTC 50500 vs prev 50200 → delta 300 > 50 → signal
    // count_trades: 2 signals, cap 2 → both accepted. OrderCount = 4
    assert_eq!(count, 4, "expected 4 trade signals");

    println!("\nDone.");
}
