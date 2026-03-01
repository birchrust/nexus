//! Mock runtime example — Driver + Plugin + explicit poll loop.
//!
//! Demonstrates the complete lifecycle of a nexus-rt runtime:
//!
//! 1. Define plugins (register resources)
//! 2. Install drivers via `WorldBuilder::install_driver`
//! 3. Build the world
//! 4. Runtime loop: poll drivers, advance tick
//!
//! The driver owns the full event lifecycle internally: receive events,
//! process via pipeline, update state. The executor loop just calls
//! `driver.poll()` — it doesn't know about event types.
//!
//! # Features demonstrated
//!
//! - `Driver` / `Plugin` — composable installation and registration
//! - `Pipeline` — pre-resolved processing chain inside the driver
//! - `Res<T>` / `ResMut<T>` — resource access via pre-resolved IDs
//! - `Local<T>` — per-system state (tick counter on check_signals)
//! - `Option<Res<T>>` — optional dependencies (risk limits)
//! - Change detection: tick = event sequence number
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example mock_runtime
//! ```

use std::collections::HashMap;

use nexus_rt::{Driver, IntoSystem, Local, Plugin, Res, ResMut, System, World, WorldBuilder};

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

struct SignalBuffer {
    signals: Vec<&'static str>,
}

impl SignalBuffer {
    fn new() -> Self {
        Self {
            signals: Vec::new(),
        }
    }
}

/// Optional resource — may or may not be registered.
struct RiskLimits {
    max_signals_per_tick: u64,
}

// =============================================================================
// Event type — fed into the driver
// =============================================================================

struct MarketTick {
    symbol: &'static str,
    price: f64,
}

// =============================================================================
// Pipeline stage functions
// =============================================================================

/// Compares tick price against cached price. Emits symbol into SignalBuffer
/// if delta exceeds threshold.
fn check_signals(
    cache: Res<PriceCache>,
    mut signals: ResMut<SignalBuffer>,
    mut tick_count: Local<u64>,
    tick: MarketTick,
) {
    *tick_count += 1;
    if let Some(&prev) = cache.prices.get(tick.symbol) {
        let delta = (tick.price - prev).abs();
        if delta > 50.0 {
            println!(
                "  [check_signals] tick #{}: {} moved {:.2} — signal",
                *tick_count, tick.symbol, delta
            );
            signals.signals.push(tick.symbol);
        }
    }
}

/// Updates the price cache with the latest price.
fn update_price(mut cache: ResMut<PriceCache>, tick: MarketTick) {
    println!("  [update_price] {} = {:.2}", tick.symbol, tick.price);
    cache.prices.insert(tick.symbol, tick.price);
}

// =============================================================================
// Plugin — resource registration
// =============================================================================

struct TradingPlugin {
    initial_prices: Vec<(&'static str, f64)>,
    risk_cap: u64,
}

impl Plugin for TradingPlugin {
    fn build(self, world: &mut WorldBuilder) {
        let mut cache = PriceCache::new();
        for (sym, price) in self.initial_prices {
            cache.prices.insert(sym, price);
        }
        world.register(cache);
        world.register(OrderCount(0));
        world.register(SignalBuffer::new());
        world.register(RiskLimits {
            max_signals_per_tick: self.risk_cap,
        });
    }
}

// =============================================================================
// Driver — market data event source
// =============================================================================

struct MarketDataInstaller;

/// Handle returned by driver installation. Owns the processing pipeline
/// and pre-resolved system for dispatch.
struct MarketDataHandle {
    /// Pipeline: MarketTick → check_signals → update_price
    check: Box<dyn System<MarketTick>>,
    update: Box<dyn System<MarketTick>>,
}

impl Driver for MarketDataInstaller {
    type Handle = MarketDataHandle;

    fn install(self, world: &mut WorldBuilder) -> MarketDataHandle {
        let r = world.registry_mut();
        MarketDataHandle {
            check: Box::new(check_signals.into_system(r)),
            update: Box::new(update_price.into_system(r)),
        }
    }
}

impl MarketDataHandle {
    /// Poll: process a batch of market ticks.
    ///
    /// For each tick: check signals (against previous cache), then update
    /// cache. Order matters — signal detection uses the old cache.
    fn poll(&mut self, world: &mut World, ticks: &[MarketTick]) {
        for tick in ticks {
            // Re-create MarketTick since we can't move out of a slice ref.
            let t = MarketTick {
                symbol: tick.symbol,
                price: tick.price,
            };
            self.check.run(world, t);

            let t = MarketTick {
                symbol: tick.symbol,
                price: tick.price,
            };
            self.update.run(world, t);
        }
    }
}

// =============================================================================
// Trade counting — runs after market data processing
// =============================================================================

fn count_trades(world: &mut World) {
    let cap = world.resource::<RiskLimits>().max_signals_per_tick;

    // Drain signals, count trades.
    let signals: Vec<&'static str> = world
        .resource_mut::<SignalBuffer>()
        .signals
        .drain(..)
        .collect();

    let count = world.resource_mut::<OrderCount>();
    let mut accepted = 0u64;
    for symbol in &signals {
        if accepted >= cap {
            println!(
                "  [count_trades] SKIPPED signal for {} (risk limit: {})",
                symbol, cap
            );
            continue;
        }
        count.0 += 1;
        accepted += 1;
        println!("  [count_trades] signal #{} for {}", count.0, symbol);
    }
}

// =============================================================================
// main — the executor loop
// =============================================================================

fn main() {
    // -- Build ----------------------------------------------------------------

    let mut wb = WorldBuilder::new();

    wb.install_plugin(TradingPlugin {
        initial_prices: vec![("BTC", 50_000.0), ("ETH", 3_000.0)],
        risk_cap: 2,
    });

    let mut md = wb.install_driver(MarketDataInstaller);

    let mut world = wb.build();

    // -- Tick 0 ---------------------------------------------------------------

    println!("=== Tick 0 ===\n");

    md.poll(
        &mut world,
        &[
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
        ],
    );
    count_trades(&mut world);
    world.advance_tick();

    // -- Tick 1 ---------------------------------------------------------------

    println!("\n=== Tick 1 ===\n");

    md.poll(
        &mut world,
        &[
            MarketTick {
                symbol: "BTC",
                price: 50_200.0,
            },
            MarketTick {
                symbol: "ETH",
                price: 3_015.0,
            },
            MarketTick {
                symbol: "BTC",
                price: 50_500.0,
            },
        ],
    );
    count_trades(&mut world);
    world.advance_tick();

    // -- Tick 2 (no events) ---------------------------------------------------

    println!("\n=== Tick 2 (no events) ===\n");

    md.poll(&mut world, &[]);
    count_trades(&mut world);
    world.advance_tick();

    println!("[dispatch] no events — nothing to process");

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
    //   BTC=49900: delta=200 > 50 → signal (vs updated 50100)
    //   count_trades: 2 signals, cap=2 → both accepted. OrderCount=2.
    //
    // Tick 1: check_signals compares against tick 0 cache (BTC=49900, ETH=3010)
    //   BTC=50200: delta=300 > 50 → signal (vs 49900)
    //   ETH=3015:  delta=5 < 50 → no signal
    //   BTC=50500: delta=300 > 50 → signal (vs updated 50200)
    //   count_trades: 2 signals, cap=2 → both accepted. OrderCount=4.
    //
    // Tick 2: no events → no signals. OrderCount unchanged.
    assert_eq!(count, 4, "expected 4 trade signals across all ticks");

    println!("\nDone.");
}
