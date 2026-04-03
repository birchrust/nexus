//! Reactor system usage examples.
//!
//! Demonstrates the two registration patterns:
//! 1. **Startup (main)** — two-phase: `create_reactor` → `into_reactor` → `insert`
//! 2. **Runtime (handler)** — one-shot via `RegistryRef` param
//!
//! Also shows: pipeline reactors, self-removal, data source registry,
//! and the full mark → dispatch cycle.
//!
//! ```bash
//! cargo run -p nexus-rt --example reactors
//! ```

use nexus_notify::Token;
use nexus_rt::{
    CtxPipelineBuilder, DeferredRemovals, PipelineReactor, ReactorNotify, Res, ResMut,
    SourceRegistry, WorldBuilder,
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
// Reactor contexts (per-instance metadata)
// =============================================================================

struct QuotingCtx {
    reactor_id: Token,
    instrument: &'static str,
    layer: u32,
}

struct TwapCtx {
    reactor_id: Token,
    instrument: &'static str,
    slice_size: u64,
    remaining: u64,
}

// =============================================================================
// Reactor step functions
// =============================================================================

/// Quoting reactor — runs every time market data or positions change.
fn quoting_step(
    ctx: &mut QuotingCtx,
    mut orders: ResMut<OrderCount>,
    mut volume: ResMut<TotalVolume>,
) {
    orders.0 += 1;
    volume.0 += u64::from(ctx.layer) * 100;
    println!(
        "  [QUOTE] {} layer={} (reactor {}) → order #{}, volume={}",
        ctx.instrument,
        ctx.layer,
        ctx.reactor_id.index(),
        orders.0,
        volume.0
    );
}

/// TWAP reactor — executes slices until done, then self-removes.
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
        "  [TWAP]  {} slice={} remaining={} (reactor {})",
        ctx.instrument,
        qty,
        ctx.remaining,
        ctx.reactor_id.index()
    );

    if ctx.remaining == 0 {
        println!(
            "  [TWAP]  {} complete — deregistering reactor {}",
            ctx.instrument,
            ctx.reactor_id.index()
        );
        removals.deregister(ctx.reactor_id);
    }
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    println!("=== Reactor System Example ===\n");

    // ── Setup ────────────────────────────────────────────────────────────

    let mut wb = WorldBuilder::new();
    wb.register(OrderCount(0));
    wb.register(TotalVolume(0));
    wb.register(SourceRegistry::new());
    // ReactorNotify + DeferredRemovals are auto-registered by build()
    let mut world = wb.build();

    // Register data sources
    let btc_md = world.register_source();
    let eth_md = world.register_source();
    let positions = world.register_source();

    // Map natural keys
    {
        let sr = world.resource_mut::<SourceRegistry>();
        sr.insert("BTC", btc_md);
        sr.insert("ETH", eth_md);
        sr.insert("positions", positions);
    }

    // ── Register quoting reactors (startup, two-phase) ─────────────────────

    println!("Registering quoting reactors...");

    // BTC quoter — spawn_reactor handles borrow juggling internally
    world
        .spawn_reactor(
            |id| QuotingCtx {
                reactor_id: id,
                instrument: "BTC",
                layer: 1,
            },
            quoting_step,
        )
        .subscribe(btc_md)
        .subscribe(positions);

    // ETH quoter
    world
        .spawn_reactor(
            |id| QuotingCtx {
                reactor_id: id,
                instrument: "ETH",
                layer: 2,
            },
            quoting_step,
        )
        .subscribe(eth_md)
        .subscribe(positions);

    println!("  Registered {} reactors\n", world.reactor_count());

    // ── Register TWAP reactor (simulates runtime registration) ─────────────

    println!("Registering TWAP algo (BTC, 500 qty, 100/slice)...");

    world
        .spawn_reactor(
            |id| TwapCtx {
                reactor_id: id,
                instrument: "BTC",
                slice_size: 100,
                remaining: 500,
            },
            twap_step,
        )
        .subscribe(btc_md);

    println!("  Registered {} reactors\n", world.reactor_count());

    // ── Register pipeline reactor ──────────────────────────────────────────

    println!("Registering pipeline reactor (BTC doubler)...");

    fn read_volume(_ctx: &mut QuotingCtx, vol: Res<TotalVolume>, _: ()) -> u64 {
        vol.0
    }

    fn double(_ctx: &mut QuotingCtx, x: u64) -> u64 {
        x * 2
    }

    fn print_result(ctx: &mut QuotingCtx, val: u64) {
        println!(
            "  [PIPE]  {} doubled volume = {} (reactor {})",
            ctx.instrument,
            val,
            ctx.reactor_id.index()
        );
    }

    // Pipeline reactor — use two-phase to get a proper token
    let token = world.resource_mut::<ReactorNotify>().create_reactor();
    let reg = world.registry();
    let pipeline = CtxPipelineBuilder::<QuotingCtx, ()>::new()
        .then(read_volume, reg)
        .then(double, reg)
        .then(print_result, reg)
        .build();

    world
        .resource_mut::<ReactorNotify>()
        .insert_reactor(
            token,
            PipelineReactor::new(
                QuotingCtx {
                    reactor_id: token,
                    instrument: "BTC",
                    layer: 0,
                },
                pipeline,
            ),
        )
        .subscribe(btc_md);

    println!("  Registered {} reactors\n", world.reactor_count());

    // ── Simulate event loop ──────────────────────────────────────────────

    for frame in 1..=8 {
        println!("--- Frame {} ---", frame);

        // Simulate: BTC data arrives every frame
        world.resource_mut::<ReactorNotify>().mark(btc_md);

        // Simulate: position update every other frame
        if frame % 2 == 0 {
            world.resource_mut::<ReactorNotify>().mark(positions);
        }

        let ran = world.dispatch_reactors();
        if !ran {
            println!("  (no reactors woke)");
        }

        println!(
            "  → reactors={} orders={} volume={}\n",
            world.reactor_count(),
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
