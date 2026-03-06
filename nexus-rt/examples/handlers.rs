//! Handler composition patterns — the handler tier.
//!
//! Demonstrates the unified `Handler<E>` trait and the various ways to
//! create, compose, and store handlers. All handler types implement the
//! same trait, so they can be boxed and stored interchangeably.
//!
//! Sections:
//! 1. Basic IntoHandler (named function → handler)
//! 2. Callback with per-instance context
//! 3. Boxing into Box<dyn Handler<E>> / Virtual<E>
//! 4. FanOut (static fan-out, 2 handlers)
//! 5. Broadcast (dynamic fan-out)
//! 6. ByRef/Cloned adapters for event type bridging
//! 7. Storing heterogeneous handlers in a Vec
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example handlers
//! ```

// Param types (Res, ResMut) are taken by value — that's the API contract.
// Reference event types (&u32) demonstrate Handler<&E> dispatch.
#![allow(clippy::needless_pass_by_value, clippy::trivially_copy_pass_by_ref)]

use nexus_rt::{
    Broadcast, Cloned, Handler, IntoCallback, IntoHandler, ResMut, Virtual, WorldBuilder, fan_out,
};

// =============================================================================
// Step functions
// =============================================================================

fn add_to_total(mut total: ResMut<u64>, event: u32) {
    *total += event as u64;
}

fn add_to_total_ref(mut total: ResMut<u64>, event: &u32) {
    *total += *event as u64;
}

fn multiply_total(mut total: ResMut<u64>, event: u32) {
    *total *= event as u64;
}

fn multiply_total_ref(mut total: ResMut<u64>, event: &u32) {
    *total *= *event as u64;
}

// Callback: context first, params in the middle, event last
fn on_tick(ctx: &mut TickCtx, mut total: ResMut<u64>, event: u32) {
    ctx.calls += 1;
    *total += event as u64 * ctx.multiplier;
}

struct TickCtx {
    multiplier: u64,
    calls: u64,
}

fn main() {
    // --- 1. Basic IntoHandler ---

    println!("=== 1. IntoHandler ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();

    let mut handler = add_to_total.into_handler(world.registry());
    handler.run(&mut world, 10u32);
    handler.run(&mut world, 20u32);

    println!("  total after add 10+20: {}", world.resource::<u64>());
    assert_eq!(*world.resource::<u64>(), 30);

    // --- 2. Callback with per-instance context ---

    println!("\n=== 2. Callback ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();

    let mut cb = on_tick.into_callback(
        TickCtx {
            multiplier: 3,
            calls: 0,
        },
        world.registry(),
    );
    cb.run(&mut world, 5u32);
    cb.run(&mut world, 10u32);

    // Context is pub — accessible outside dispatch
    println!("  total: {}", world.resource::<u64>());
    println!("  callback calls: {}", cb.ctx.calls);
    assert_eq!(*world.resource::<u64>(), 45); // 5*3 + 10*3
    assert_eq!(cb.ctx.calls, 2);

    // --- 3. Boxing into Virtual<E> ---
    //
    // Pipeline, DAG, and composed types are deeply nested generics.
    // Box them for storage. Cost: one vtable dispatch at the boundary.

    println!("\n=== 3. Boxing (Virtual<E>) ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();

    let h = add_to_total.into_handler(world.registry());
    let mut boxed: Virtual<u32> = Box::new(h);

    boxed.run(&mut world, 42u32);
    println!("  boxed handler result: {}", world.resource::<u64>());
    assert_eq!(*world.resource::<u64>(), 42);

    // --- 4. FanOut (static fan-out) ---
    //
    // Dispatches &E to a fixed set of handlers. Zero allocation.

    println!("\n=== 4. FanOut ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(1);
    let mut world = wb.build();

    let h1 = add_to_total_ref.into_handler(world.registry());
    let h2 = multiply_total_ref.into_handler(world.registry());
    let mut fan = fan_out!(h1, h2);

    // h1 runs first (adds), then h2 (multiplies). Both see &u32.
    fan.run(&mut world, 5u32);
    println!(
        "  after fan_out(add, mul) with 5: {}",
        world.resource::<u64>()
    );
    assert_eq!(*world.resource::<u64>(), 30); // (1+5)*5

    // --- 5. Broadcast (dynamic fan-out) ---

    println!("\n=== 5. Broadcast ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();

    let h1 = add_to_total_ref.into_handler(world.registry());
    let h2 = add_to_total_ref.into_handler(world.registry());
    let mut bcast = Broadcast::new();
    bcast.add(h1);
    bcast.add(h2);

    bcast.run(&mut world, 7u32);
    println!("  broadcast 2x add with 7: {}", world.resource::<u64>());
    assert_eq!(*world.resource::<u64>(), 14); // 7+7

    // --- 6. Cloned adapter ---
    //
    // Cloned wraps Handler<E> to implement Handler<&E>.
    // Needed when an owned-event handler goes into FanOut.

    println!("\n=== 6. Cloned adapter ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();

    let owned_handler = add_to_total.into_handler(world.registry());
    let ref_handler = add_to_total_ref.into_handler(world.registry());
    // Cloned adapts the owned handler to accept &u32
    let mut fan = fan_out!(Cloned(owned_handler), ref_handler);

    fan.run(&mut world, 10u32);
    println!(
        "  fan_out with Cloned adapter: {}",
        world.resource::<u64>()
    );
    assert_eq!(*world.resource::<u64>(), 20); // 10+10

    // --- 7. Heterogeneous handler Vec ---

    println!("\n=== 7. Heterogeneous Vec<Virtual<E>> ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();

    let h1: Virtual<u32> = Box::new(add_to_total.into_handler(world.registry()));
    let h2: Virtual<u32> = Box::new(multiply_total.into_handler(world.registry()));
    let mut handlers: Vec<Virtual<u32>> = vec![h1, h2];

    *world.resource_mut::<u64>() = 5;
    for h in &mut handlers {
        h.run(&mut world, 3u32);
    }
    println!(
        "  after add then mul with 3: {}",
        world.resource::<u64>()
    );
    assert_eq!(*world.resource::<u64>(), 24); // (5+3)*3

    println!("\nDone.");
}
