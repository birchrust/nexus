//! Mock runtime example — Scheduler + Plugin + App.
//!
//! Demonstrates the complete lifecycle of a nexus-rt runtime:
//!
//! 1. Define plugins (register resources + systems with ordering)
//! 2. Build via `App::new().add_plugin(...).build()`
//! 3. Runtime loop:
//!    - Poll: push events into `Events<MarketTick>` via `resource_mut`
//!    - Dispatch: `world.with_mut::<Scheduler, _>(|s, w| s.dispatch(w))`
//!    - Clear event buffers, `advance_tick()`
//!
//! Systems are batch-oriented: they read all events for a tick via
//! `EventReader<T>`, not one event at a time.
//!
//! # Features demonstrated
//!
//! - `Scheduler` — toposorted dispatch with automatic skip propagation
//! - `Plugin` / `App` — composable system and resource registration
//! - `EventReader<T>` / `EventWriter<T>` — batch event processing
//! - `Res<T>` / `ResMut<T>` — shared and exclusive resource access
//! - `Local<T>` — per-system state (tick counter on check_signals)
//! - `Option<Res<T>>` — optional dependencies (risk limits)
//! - Skip propagation: on a tick with no new events, all systems skip
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example mock_runtime
//! ```

use std::collections::HashMap;

use nexus_rt::{
    App, EventReader, EventWriter, Events, Local, Plugin, Res, ResMut, Scheduler, SchedulerBuilder,
    WorldBuilder,
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

struct MarketTick {
    symbol: &'static str,
    price: f64,
}

struct TradeSignal {
    symbol: &'static str,
}

// =============================================================================
// Systems — batch-oriented via EventReader<T>
// =============================================================================

/// Compares each tick's price against the cached price from the previous
/// dispatch cycle. Emits a TradeSignal if the delta exceeds the threshold.
///
/// Uses `Local<u64>` to track total ticks processed across all cycles.
///
/// Ordering: runs BEFORE update_prices so signal detection compares
/// against the previous cycle's cache.
fn check_signals(
    ticks: EventReader<MarketTick>,
    cache: Res<PriceCache>,
    mut signals: EventWriter<TradeSignal>,
    mut tick_count: Local<u64>,
    _: (),
) {
    for tick in ticks.iter() {
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
}

/// Updates the price cache with the latest prices from this batch.
///
/// Runs AFTER check_signals so that signal detection compares against
/// the previous cycle's prices.
fn update_prices(ticks: EventReader<MarketTick>, mut cache: ResMut<PriceCache>, _: ()) {
    for tick in ticks.iter() {
        cache.prices.insert(tick.symbol, tick.price);
        println!("  [update_prices] {} = {:.2}", tick.symbol, tick.price);
    }
}

/// Reads accumulated trade signals and increments the order count.
///
/// Uses `Option<Res<RiskLimits>>` — respects per-tick cap if registered.
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
// Plugin — composable registration
// =============================================================================

struct TradingPlugin {
    initial_prices: Vec<(&'static str, f64)>,
    risk_cap: u64,
}

impl Plugin for TradingPlugin {
    fn build(&self, world: &mut WorldBuilder, scheduler: &mut SchedulerBuilder) {
        // Resources
        let mut cache = PriceCache::new();
        for &(sym, price) in &self.initial_prices {
            cache.prices.insert(sym, price);
        }
        world.register(cache);
        world.register(OrderCount(0));
        world.register(RiskLimits {
            max_signals_per_tick: self.risk_cap,
        });
        world.register_default::<Events<MarketTick>>();
        world.register_default::<Events<TradeSignal>>();

        // Systems + ordering
        let signals = scheduler.add_system(check_signals, world.registry());
        let prices = scheduler.add_system(update_prices, world.registry());
        let trades = scheduler.add_system(count_trades, world.registry());

        // check_signals before update_prices: signal detection uses old cache
        // check_signals before count_trades: signals must exist before counting
        scheduler.after(prices, signals);
        scheduler.after(trades, signals);
    }
}

// =============================================================================
// main — the runtime loop is here, not hidden in a method
// =============================================================================

fn main() {
    // -- Build via App + Plugin -----------------------------------------------

    let mut app = App::new();
    app.add_plugin(&TradingPlugin {
        initial_prices: vec![("BTC", 50_000.0), ("ETH", 3_000.0)],
        risk_cap: 2,
    });
    let mut world = app.build();

    // -- Tick 0 ---------------------------------------------------------------

    println!("=== Tick 0 ===\n");

    // Poll: push events into Events<MarketTick>.
    {
        let events = world.resource_mut::<Events<MarketTick>>();
        events.send(MarketTick {
            symbol: "BTC",
            price: 50_100.0,
        });
        events.send(MarketTick {
            symbol: "ETH",
            price: 3_010.0,
        });
        events.send(MarketTick {
            symbol: "BTC",
            price: 49_900.0,
        });
    }
    println!("[poll] 3 events");

    // Dispatch: Scheduler walks toposorted systems with skip propagation.
    world.with_mut::<Scheduler, _>(|scheduler, world| {
        scheduler.dispatch(world);
    });

    // Clear event buffers + advance tick.
    world.resource_mut::<Events<MarketTick>>().clear();
    world.resource_mut::<Events<TradeSignal>>().clear();
    world.advance_tick();

    // -- Tick 1 ---------------------------------------------------------------

    println!("\n=== Tick 1 ===\n");

    {
        let events = world.resource_mut::<Events<MarketTick>>();
        events.send(MarketTick {
            symbol: "BTC",
            price: 50_200.0,
        });
        events.send(MarketTick {
            symbol: "ETH",
            price: 3_015.0,
        });
        events.send(MarketTick {
            symbol: "BTC",
            price: 50_500.0,
        });
    }
    println!("[poll] 3 events");

    world.with_mut::<Scheduler, _>(|scheduler, world| {
        scheduler.dispatch(world);
    });

    world.resource_mut::<Events<MarketTick>>().clear();
    world.resource_mut::<Events<TradeSignal>>().clear();
    world.advance_tick();

    // -- Tick 2 (no events — demonstrates skip propagation) -------------------

    println!("\n=== Tick 2 (no events) ===\n");

    // No events pushed. All systems should be skipped.
    world.with_mut::<Scheduler, _>(|scheduler, world| {
        scheduler.dispatch(world);
    });
    println!("[dispatch] all systems skipped (no inputs changed)");

    world.resource_mut::<Events<MarketTick>>().clear();
    world.resource_mut::<Events<TradeSignal>>().clear();
    world.advance_tick();

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

    // Tick 0: check_signals compares against initial cache (BTC=50000, ETH=3000)
    //   BTC=50100: delta=100 > 50 → signal
    //   ETH=3010:  delta=10 < 50 → no signal
    //   BTC=49900: delta=100 > 50 → signal (vs initial 50000, not 50100)
    //   count_trades: 2 signals, cap=2 → both accepted. OrderCount=2.
    //
    // Tick 1: check_signals compares against tick 0 cache (BTC=49900, ETH=3010)
    //   BTC=50200: delta=300 > 50 → signal
    //   ETH=3015:  delta=5 < 50 → no signal
    //   BTC=50500: delta=600 > 50 → signal (vs 49900, not 50200)
    //   count_trades: 2 signals, cap=2 → both accepted. OrderCount=4.
    //
    // Tick 2: no events → all systems skipped. OrderCount unchanged.
    assert_eq!(count, 4, "expected 4 trade signals across all ticks");

    println!("\nDone.");
}
