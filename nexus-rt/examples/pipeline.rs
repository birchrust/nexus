//! Pre-resolved pipeline composition using Param steps.
//!
//! Pipeline steps are named functions whose Param dependencies are
//! resolved at build time. Arity-0 steps (no Params) also accept
//! closures.
//!
//! All combinator methods resolve params from the registry. Named functions
//! get direct pointer access (single deref). Arity-0 closures work for
//! simple transforms. Raw `&mut World` closures are available as an escape
//! hatch via the `Opaque` marker.
//!
//! Two dispatch modes:
//! - `run()` — direct call, no boxing, works with borrowed inputs
//! - `build()` — into `Pipeline`, implements `Handler<In>`
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example pipeline
//! ```

use nexus_rt::{Handler, PipelineBuilder, ResMut, Resource, WorldBuilder, new_resource};

new_resource!(Accumulator(u64));

#[derive(Resource)]
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

// --- Step functions: Params first, step input last ---

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

#[allow(clippy::needless_pass_by_value)]
fn count_error(mut errors: ResMut<Accumulator>, err: ProcessError) {
    println!("  [catch] {err:?}");
    **errors += 1;
}

#[allow(clippy::needless_pass_by_value)]
fn store_price(mut cache: ResMut<PriceCache>, tick: MarketTick) {
    println!("  [ok] {} @ {:.2}", tick.symbol, tick.price);
    cache.latest = tick.price;
    cache.updates += 1;
}

#[allow(clippy::needless_pass_by_value)]
fn accumulate(mut total: ResMut<Accumulator>, x: u32) {
    **total += u64::from(x);
}

fn main() {
    // --- Bare value pipeline: arity-0 closure steps ---

    println!("=== Bare Value Pipeline ===\n");

    let mut world = WorldBuilder::new().build();
    let r = world.registry_mut();

    let mut bare_pipeline = PipelineBuilder::<u32>::new()
        .then(|x: u32| x * 2, r)
        .then(|x: u32| x + 1, r);

    println!("  5 → {}", bare_pipeline.run(&mut world, 5));
    println!("  10 → {}", bare_pipeline.run(&mut world, 10));

    // --- Option pipeline: filter + inspect (cold path), map (hot path) ---

    println!("\n=== Option Pipeline ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(PriceCache::new());
    let mut world = wb.build();
    let r = world.registry_mut();

    let mut option_pipeline = PipelineBuilder::<MarketTick>::new()
        .then(
            |tick: MarketTick| -> Option<MarketTick> {
                if tick.price > 0.0 { Some(tick) } else { None }
            },
            r,
        )
        .filter(|tick: &MarketTick| tick.symbol == "BTC", r)
        .inspect(
            |tick: &MarketTick| {
                println!("  [inspect] {} @ {:.2}", tick.symbol, tick.price);
            },
            r,
        )
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
    #[allow(clippy::float_cmp)]
    {
        assert_eq!(cache.latest, 51_000.0);
    }

    // --- Result pipeline: validate → check → catch → store ---

    println!("\n=== Result Pipeline with catch ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(PriceCache::new());
    wb.register(Accumulator(0)); // error counter
    let mut world = wb.build();
    let r = world.registry_mut();

    let mut result_pipeline = PipelineBuilder::<MarketTick>::new()
        .then(validate, r)
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

    let errors = world.resource::<Accumulator>().0;
    println!("\n  Errors: {errors}");
    assert_eq!(errors, 2);

    // --- Build into Handler ---

    println!("\n=== Pipeline as Handler ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(Accumulator(0));
    let mut world = wb.build();
    let r = world.registry_mut();

    let mut pipeline = PipelineBuilder::<u32>::new().then(accumulate, r).build();

    pipeline.run(&mut world, 10);
    pipeline.run(&mut world, 20);
    pipeline.run(&mut world, 30);

    let total = world.resource::<Accumulator>().0;
    println!("  Total: {total}");
    assert_eq!(total, 60);

    // --- Guard: wrap value in Option via predicate ---

    println!("\n=== Guard + Tap ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(Accumulator(0));
    let mut world = wb.build();
    let r = world.registry_mut();

    let mut guarded = PipelineBuilder::<u32>::new()
        .then(|x: u32| x, r)
        .guard(|x: &u32| *x > 5, r) // → Option<u32>
        .tap(|x: &Option<u32>| println!("  [tap] guard output: {x:?}"), r)
        .map(accumulate, r) // runs for Some only
        .unwrap_or(()); // discard None
    for v in [3u32, 7, 2, 10, 1] {
        guarded.run(&mut world, v);
    }
    let total = world.resource::<Accumulator>().0;
    println!("  Total (values > 5 only): {total}");
    assert_eq!(total, 17); // 7 + 10

    // --- Splat: destructure tuple output into individual arguments ---

    println!("\n=== Splat ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(Accumulator(0));
    let mut world = wb.build();
    let r = world.registry_mut();

    #[allow(clippy::items_after_statements)]
    fn split(x: u32) -> (u32, u32) {
        (x, x * 3)
    }
    #[allow(clippy::items_after_statements, clippy::needless_pass_by_value)]
    fn combine(mut out: ResMut<Accumulator>, a: u32, b: u32) {
        **out = a as u64 + b as u64;
    }

    let mut pipeline = PipelineBuilder::<u32>::new()
        .then(split, r)
        .splat()
        .then(combine, r)
        .build();

    pipeline.run(&mut world, 5);
    let result = world.resource::<Accumulator>().0;
    println!("  split(5) = (5, 15), combine = {result}");
    assert_eq!(result, 20);

    // --- Pipeline vs DAG note ---
    //
    // Pipeline is for linear chains (A → B → C).
    // DAG is for fan-out/merge (A → [B, C] → D).
    // Both produce Handler<E> via .build(). Use Pipeline for the common case;
    // reach for DAG when you need .fork().

    println!("\nDone.");
}
