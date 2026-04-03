//! Actor system usage examples.
//!
//! Demonstrates the two registration patterns:
//! 1. **Startup (main)** — two-phase: `alloc_actor` → `into_actor` → `insert`
//! 2. **Runtime (handler)** — one-shot via `RegistryRef` param
//!
//! Also shows: pipeline actors, self-removal, data source registry,
//! and the full mark → dispatch cycle.
//!
//! ```bash
//! cargo run -p nexus-rt --example actors
//! ```

use nexus_notify::Token;
use nexus_rt::{
    ActorNotify, ActorSystem, CtxPipelineBuilder, DeferredRemovals, IntoActor, PipelineActor, Res,
    ResMut, SourceRegistry, WorldBuilder,
};

// =============================================================================
// Resources (shared state in World)
// =============================================================================

nexus_rt::new_resource!(
    #[derive(Debug)]
    OrderCount(u64)
);

nexus_rt::new_resource!(
    #[derive(Debug)]
    TotalVolume(u64)
);

// =============================================================================
// Actor contexts (per-instance metadata)
// =============================================================================

struct QuotingCtx {
    actor_id: Token,
    instrument: &'static str,
    layer: u32,
}

struct TwapCtx {
    actor_id: Token,
    instrument: &'static str,
    slice_size: u64,
    remaining: u64,
}

// =============================================================================
// Actor step functions
// =============================================================================

/// Quoting actor — runs every time market data or positions change.
fn quoting_step(
    ctx: &mut QuotingCtx,
    mut orders: ResMut<OrderCount>,
    mut volume: ResMut<TotalVolume>,
) {
    orders.0 += 1;
    volume.0 += u64::from(ctx.layer) * 100;
    println!(
        "  [QUOTE] {} layer={} (actor {}) → order #{}, volume={}",
        ctx.instrument, ctx.layer, ctx.actor_id.index(), orders.0, volume.0
    );
}

/// TWAP actor — executes slices until done, then self-removes.
fn twap_step(
    ctx: &mut TwapCtx,
    mut orders: ResMut<OrderCount>,
    mut volume: ResMut<TotalVolume>,
    mut removals: ResMut<DeferredRemovals>,
) {
    let qty = ctx.slice_size.min(ctx.remaining);
    ctx.remaining -= qty;
    orders.0 += 1;
    volume.0 += qty;
    println!(
        "  [TWAP]  {} slice={} remaining={} (actor {})",
        ctx.instrument, qty, ctx.remaining, ctx.actor_id.index()
    );

    if ctx.remaining == 0 {
        println!(
            "  [TWAP]  {} complete — deregistering actor {}",
            ctx.instrument, ctx.actor_id.index()
        );
        removals.deregister(ctx.actor_id);
    }
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    println!("=== Actor System Example ===\n");

    // ── Setup ────────────────────────────────────────────────────────────

    let mut wb = WorldBuilder::new();
    wb.register(OrderCount(0));
    wb.register(TotalVolume(0));
    wb.register(ActorNotify::new(16, 64));
    wb.register(DeferredRemovals::default());
    wb.register(SourceRegistry::new());
    let mut world = wb.build();

    let mut system = ActorSystem::new(&world);

    // Register data sources
    let btc_md = world.resource_mut::<ActorNotify>().register_source();
    let eth_md = world.resource_mut::<ActorNotify>().register_source();
    let positions = world.resource_mut::<ActorNotify>().register_source();

    // Map natural keys
    {
        let sr = world.resource_mut::<SourceRegistry>();
        sr.insert("BTC", btc_md);
        sr.insert("ETH", eth_md);
        sr.insert("positions", positions);
    }

    // ── Register quoting actors (startup, two-phase) ─────────────────────

    println!("Registering quoting actors...");

    // BTC quoter
    let token = world.resource_mut::<ActorNotify>().alloc_actor();
    let actor = quoting_step.into_actor(
        QuotingCtx { actor_id: token, instrument: "BTC", layer: 1 },
        world.registry(),
    );
    world.resource_mut::<ActorNotify>()
        .insert(token, actor)
        .subscribe(btc_md)
        .subscribe(positions);

    // ETH quoter
    let token = world.resource_mut::<ActorNotify>().alloc_actor();
    let actor = quoting_step.into_actor(
        QuotingCtx { actor_id: token, instrument: "ETH", layer: 2 },
        world.registry(),
    );
    world.resource_mut::<ActorNotify>()
        .insert(token, actor)
        .subscribe(eth_md)
        .subscribe(positions);

    println!("  Registered {} actors\n", system.actor_count(&world));

    // ── Register TWAP actor (simulates runtime registration) ─────────────

    println!("Registering TWAP algo (BTC, 500 qty, 100/slice)...");

    let token = world.resource_mut::<ActorNotify>().alloc_actor();
    let actor = twap_step.into_actor(
        TwapCtx { actor_id: token, instrument: "BTC", slice_size: 100, remaining: 500 },
        world.registry(),
    );
    world.resource_mut::<ActorNotify>()
        .insert(token, actor)
        .subscribe(btc_md);

    println!("  Registered {} actors\n", system.actor_count(&world));

    // ── Register pipeline actor ──────────────────────────────────────────

    println!("Registering pipeline actor (BTC doubler)...");

    fn read_volume(_ctx: &mut QuotingCtx, vol: Res<TotalVolume>, _: ()) -> u64 {
        vol.0
    }

    fn double(_ctx: &mut QuotingCtx, x: u64) -> u64 {
        x * 2
    }

    fn print_result(ctx: &mut QuotingCtx, val: u64) {
        println!(
            "  [PIPE]  {} doubled volume = {} (actor {})",
            ctx.instrument, val, ctx.actor_id.index()
        );
    }

    let token = world.resource_mut::<ActorNotify>().alloc_actor();
    let reg = world.registry();
    let pipeline = CtxPipelineBuilder::<QuotingCtx, ()>::new()
        .then(read_volume, reg)
        .then(double, reg)
        .then(print_result, reg)
        .build();
    let actor = PipelineActor::new(
        QuotingCtx { actor_id: token, instrument: "BTC", layer: 0 },
        pipeline,
    );
    world.resource_mut::<ActorNotify>()
        .insert(token, actor)
        .subscribe(btc_md);

    println!("  Registered {} actors\n", system.actor_count(&world));

    // ── Simulate event loop ──────────────────────────────────────────────

    for frame in 1..=8 {
        println!("--- Frame {} ---", frame);

        // Simulate: BTC data arrives every frame
        world.resource_mut::<ActorNotify>().mark(btc_md);

        // Simulate: position update every other frame
        if frame % 2 == 0 {
            world.resource_mut::<ActorNotify>().mark(positions);
        }

        let ran = system.dispatch(&mut world);
        if !ran {
            println!("  (no actors woke)");
        }

        println!(
            "  → actors={} orders={} volume={}\n",
            system.actor_count(&world),
            world.resource::<OrderCount>().0,
            world.resource::<TotalVolume>().0,
        );
    }

    // ── SourceRegistry lookup ────────────────────────────────────────────

    println!("--- SourceRegistry lookup ---");
    let sr = world.resource::<SourceRegistry>();
    println!("  BTC source: {:?}", sr.get(&"BTC"));
    println!("  ETH source: {:?}", sr.get(&"ETH"));
    println!("  SOL source: {:?}", sr.get(&"SOL")); // None — not registered
}
