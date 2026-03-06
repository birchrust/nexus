//! Template generation patterns — resolve once, stamp many.
//!
//! When handlers are created repeatedly on the hot path (IO re-registration,
//! timer rescheduling), each `into_handler(registry)` call pays HashMap
//! lookups to resolve ResourceId values. Templates resolve once, then
//! `generate()` stamps handlers by copying pre-resolved state (~1 cycle
//! vs ~20-70 cycles for `into_handler`).
//!
//! Sections:
//! 1. Basic HandlerTemplate: resolve once, generate many
//! 2. CallbackTemplate with per-instance context
//! 3. handler_blueprint! macro shorthand
//! 4. Why templates matter (move-out-fire lifecycle)
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example templates
//! ```

use nexus_rt::template::{Blueprint, CallbackBlueprint, CallbackTemplate, HandlerTemplate};
use nexus_rt::{Handler, ResMut, WorldBuilder, handler_blueprint};

// =============================================================================
// Blueprint definitions
// =============================================================================

struct OnTick;
impl Blueprint for OnTick {
    type Event = u32;
    type Params = (ResMut<'static, u64>,);
}

struct OnTimeout;
impl Blueprint for OnTimeout {
    type Event = ();
    type Params = (ResMut<'static, u64>,);
}
impl CallbackBlueprint for OnTimeout {
    type Context = TimerCtx;
}

struct TimerCtx {
    order_id: u64,
    fires: u64,
}

// Using the macro shorthand
handler_blueprint!(OnEvent, Event = u64, Params = (ResMut<'static, u64>,));

// =============================================================================
// Handler functions
// =============================================================================

fn tick(mut counter: ResMut<u64>, event: u32) {
    *counter += event as u64;
}

fn on_timeout(ctx: &mut TimerCtx, mut counter: ResMut<u64>, _event: ()) {
    ctx.fires += 1;
    *counter += ctx.order_id;
}

fn on_event(mut counter: ResMut<u64>, event: u64) {
    *counter += event;
}

fn main() {
    // --- 1. HandlerTemplate ---

    println!("=== 1. HandlerTemplate ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();

    // Resolve params once
    let template = HandlerTemplate::<OnTick>::new(tick, world.registry());

    // Stamp out independent handlers — no HashMap lookups, just Copy
    let mut h1 = template.generate();
    let mut h2 = template.generate();
    let mut h3 = template.generate();

    h1.run(&mut world, 10u32);
    h2.run(&mut world, 20u32);
    h3.run(&mut world, 30u32);

    println!("  3 handlers, total: {}", world.resource::<u64>());
    assert_eq!(*world.resource::<u64>(), 60);

    // --- 2. CallbackTemplate with per-instance context ---

    println!("\n=== 2. CallbackTemplate ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();

    let cb_template = CallbackTemplate::<OnTimeout>::new(on_timeout, world.registry());

    // Each generate() takes a unique context
    let mut cb1 = cb_template.generate(TimerCtx {
        order_id: 100,
        fires: 0,
    });
    let mut cb2 = cb_template.generate(TimerCtx {
        order_id: 200,
        fires: 0,
    });

    cb1.run(&mut world, ());
    cb1.run(&mut world, ());
    cb2.run(&mut world, ());

    println!("  cb1 fires: {}", cb1.ctx().fires);
    println!("  cb2 fires: {}", cb2.ctx().fires);
    println!("  total: {}", world.resource::<u64>());
    assert_eq!(cb1.ctx().fires, 2);
    assert_eq!(cb2.ctx().fires, 1);
    assert_eq!(*world.resource::<u64>(), 400); // 100+100+200

    // --- 3. handler_blueprint! macro ---

    println!("\n=== 3. handler_blueprint! macro ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();

    // OnEvent was defined with the macro — same usage pattern
    let template = HandlerTemplate::<OnEvent>::new(on_event, world.registry());
    let mut h = template.generate();

    h.run(&mut world, 42u64);
    println!("  result: {}", world.resource::<u64>());
    assert_eq!(*world.resource::<u64>(), 42);

    // --- 4. Move-out-fire pattern ---
    //
    // In timer wheels and IO slabs, handlers are removed from storage,
    // fired, and optionally re-inserted. The template makes re-insertion
    // cheap — generate() is ~1 cycle vs ~20-70 for into_handler().
    //
    // Pseudocode:
    //   let handler = slab.remove(key);     // move out
    //   handler.run(&mut world, event);     // fire
    //   let new = template.generate();      // ~1 cycle
    //   slab.insert(new);                   // re-insert

    println!("\n=== 4. Move-out-fire pattern ===\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u64>(0);
    let mut world = wb.build();

    let template = HandlerTemplate::<OnTick>::new(tick, world.registry());

    // Simulate 5 rounds of move-out-fire
    let mut handler = template.generate();
    for i in 0..5u32 {
        // "fire" the handler
        handler.run(&mut world, i + 1);
        // "re-create" for next round (simulates re-insertion)
        handler = template.generate();
    }

    println!("  after 5 rounds: {}", world.resource::<u64>());
    assert_eq!(*world.resource::<u64>(), 15); // 1+2+3+4+5

    println!("\nDone.");
}
