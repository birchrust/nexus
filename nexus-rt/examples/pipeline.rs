//! Per-event typed pipeline composition.
//!
//! Pipelines compose stages through closure chains. `pipe()` is the core
//! transform. When the output is `Option<T>` or `Result<T, E>`, additional
//! convenience methods (`map`, `filter`, `catch`, etc.) become available.
//!
//! Two dispatch modes:
//! - `run()` — direct call, no boxing, works with borrowed inputs
//! - `build()` — box into `Pipeline<In>`, implements `System<In>`
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example pipeline
//! ```

use nexus_rt::{Pipeline, PipelineStart, System, WorldBuilder};

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

    fn update(&mut self, price: f64) {
        self.latest = price;
        self.updates += 1;
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

fn main() {
    // --- Bare value pipeline: pipe transforms freely ---

    println!("=== Bare Value Pipeline ===\n");

    let mut world = WorldBuilder::new().build();

    let mut bare_pipeline = PipelineStart::<u32>::new()
        .pipe(|_w, x| x * 2)
        .pipe(|_w, x| x + 1);

    println!("  5 → {}", bare_pipeline.run(&mut world, 5));
    println!("  10 → {}", bare_pipeline.run(&mut world, 10));

    // --- Option pipeline: filter + inspect ---

    println!("\n=== Option Pipeline ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(PriceCache::new());
    let mut world = wb.build();

    let mut option_pipeline = PipelineStart::<MarketTick>::new()
        .pipe(|_w, tick| if tick.price > 0.0 { Some(tick) } else { None })
        .filter(|_w, tick| tick.symbol == "BTC")
        .inspect(|_w, tick| {
            println!("  [inspect] processing {} @ {:.2}", tick.symbol, tick.price);
        })
        .map(|w, tick| {
            w.resource_mut::<PriceCache>().update(tick.price);
        });

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

    // --- Result pipeline: error handling + catch ---

    println!("\n=== Result Pipeline with catch ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(PriceCache::new());
    wb.register::<u64>(0); // error counter
    let mut world = wb.build();

    let mut result_pipeline = PipelineStart::<MarketTick>::new()
        .pipe(|_w, tick| -> Result<MarketTick, ProcessError> {
            if tick.price <= 0.0 {
                Err(ProcessError::InvalidPrice)
            } else {
                Ok(tick)
            }
        })
        .and_then(|_w, tick| -> Result<MarketTick, ProcessError> {
            if tick.symbol == "BTC" || tick.symbol == "ETH" {
                Ok(tick)
            } else {
                Err(ProcessError::UnknownSymbol)
            }
        })
        .catch(|w, err| {
            println!("  [catch] {err:?}");
            *w.resource_mut::<u64>() += 1;
        })
        .map(|w, tick| {
            println!("  [ok] {} @ {:.2}", tick.symbol, tick.price);
            w.resource_mut::<PriceCache>().update(tick.price);
        });

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

    // --- Build into System, store in World ---

    println!("\n=== Pipeline as System ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);

    let pipeline = PipelineStart::<u32>::new()
        .pipe(|w, x| {
            *w.resource_mut::<u64>() += u64::from(x);
        })
        .build();

    wb.register::<Pipeline<u32>>(pipeline);
    let mut world = wb.build();

    world.with_mut::<Pipeline<u32>, _>(|pipeline, world| {
        pipeline.run(world, 10);
        pipeline.run(world, 20);
        pipeline.run(world, 30);
    });

    let total = *world.resource::<u64>();
    println!("  Total: {total}");
    assert_eq!(total, 60);

    println!("\nDone.");
}
