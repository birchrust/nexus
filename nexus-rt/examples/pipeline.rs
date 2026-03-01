//! Pre-resolved pipeline composition using SystemParam stages.
//!
//! Pipeline stages are named functions whose SystemParam dependencies are
//! resolved at build time. Arity-0 stages (no SystemParams) also accept
//! closures.
//!
//! IntoStage-based methods (`.stage()`, `.map()`, `.and_then()`, `.catch()`)
//! resolve params from the registry. Closure-based methods (`.filter()`,
//! `.inspect()`, `.on_none()`, etc.) take `&mut World` for cold-path use.
//!
//! Two dispatch modes:
//! - `run()` — direct call, no boxing, works with borrowed inputs
//! - `build()` — box into `Pipeline<In>`, implements `Handler<In>`
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example pipeline
//! ```

use nexus_rt::{Handler, PipelineStart, ResMut, WorldBuilder};

struct PriceCache {
    latest: f64,
    updates: u64,
}

impl PriceCache {
    fn new() -> Self {
        Self {
            latest: 0.0,
            updates: 0,
        }
    }
}

struct MarketTick {
    symbol: &'static str,
    price: f64,
}

#[derive(Debug)]
enum ProcessError {
    InvalidPrice,
    UnknownSymbol,
}

// --- Stage functions: SystemParams first, stage input last ---

fn validate(tick: MarketTick) -> Result<MarketTick, ProcessError> {
    if tick.price <= 0.0 {
        Err(ProcessError::InvalidPrice)
    } else {
        Ok(tick)
    }
}

fn check_known(tick: MarketTick) -> Result<MarketTick, ProcessError> {
    if tick.symbol == "BTC" || tick.symbol == "ETH" {
        Ok(tick)
    } else {
        Err(ProcessError::UnknownSymbol)
    }
}

fn count_error(mut errors: ResMut<u64>, err: ProcessError) {
    println!("  [catch] {err:?}");
    *errors += 1;
}

fn store_price(mut cache: ResMut<PriceCache>, tick: MarketTick) {
    println!("  [ok] {} @ {:.2}", tick.symbol, tick.price);
    cache.latest = tick.price;
    cache.updates += 1;
}

fn accumulate(mut total: ResMut<u64>, x: u32) {
    *total += u64::from(x);
}

fn main() {
    // --- Bare value pipeline: arity-0 closure stages ---

    println!("=== Bare Value Pipeline ===\n");

    let mut world = WorldBuilder::new().build();
    let r = world.registry_mut();

    let mut bare_pipeline = PipelineStart::<u32>::new()
        .stage(|x: u32| x * 2, r)
        .stage(|x: u32| x + 1, r);

    println!("  5 → {}", bare_pipeline.run(&mut world, 5));
    println!("  10 → {}", bare_pipeline.run(&mut world, 10));

    // --- Option pipeline: filter + inspect (cold path), map (hot path) ---

    println!("\n=== Option Pipeline ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(PriceCache::new());
    let mut world = wb.build();
    let r = world.registry_mut();

    let mut option_pipeline = PipelineStart::<MarketTick>::new()
        .stage(
            |tick: MarketTick| -> Option<MarketTick> {
                if tick.price > 0.0 { Some(tick) } else { None }
            },
            r,
        )
        .filter(|_w, tick| tick.symbol == "BTC")
        .inspect(|_w, tick| {
            println!("  [inspect] {} @ {:.2}", tick.symbol, tick.price);
        })
        .map(store_price, r);

    let ticks = [
        MarketTick {
            symbol: "BTC",
            price: 50_000.0,
        },
        MarketTick {
            symbol: "ETH",
            price: 3_000.0,
        }, // filtered: not BTC
        MarketTick {
            symbol: "BTC",
            price: -1.0,
        }, // filtered: negative
        MarketTick {
            symbol: "BTC",
            price: 51_000.0,
        },
    ];

    for tick in ticks {
        option_pipeline.run(&mut world, tick);
    }

    let cache = world.resource::<PriceCache>();
    println!(
        "\n  PriceCache: latest={:.2}, updates={}",
        cache.latest, cache.updates
    );
    assert_eq!(cache.updates, 2);
    assert_eq!(cache.latest, 51_000.0);

    // --- Result pipeline: validate → check → catch → store ---

    println!("\n=== Result Pipeline with catch ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(PriceCache::new());
    wb.register::<u64>(0); // error counter
    let mut world = wb.build();
    let r = world.registry_mut();

    let mut result_pipeline = PipelineStart::<MarketTick>::new()
        .stage(validate, r)
        .and_then(check_known, r)
        .catch(count_error, r)
        .map(store_price, r);

    let ticks = [
        MarketTick {
            symbol: "BTC",
            price: 52_000.0,
        },
        MarketTick {
            symbol: "XYZ",
            price: 100.0,
        }, // unknown symbol → catch
        MarketTick {
            symbol: "ETH",
            price: -5.0,
        }, // invalid price → catch
        MarketTick {
            symbol: "ETH",
            price: 3_500.0,
        },
    ];

    for tick in ticks {
        result_pipeline.run(&mut world, tick);
    }

    let errors = *world.resource::<u64>();
    println!("\n  Errors: {errors}");
    assert_eq!(errors, 2);

    // --- Build into Handler ---

    println!("\n=== Pipeline as Handler ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();
    let r = world.registry_mut();

    let mut pipeline = PipelineStart::<u32>::new().stage(accumulate, r).build();

    pipeline.run(&mut world, 10);
    pipeline.run(&mut world, 20);
    pipeline.run(&mut world, 30);

    let total = *world.resource::<u64>();
    println!("  Total: {total}");
    assert_eq!(total, 60);

    println!("\nDone.");
}
