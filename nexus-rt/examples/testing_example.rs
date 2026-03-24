//! Testing with TestHarness — isolated handler unit testing.
//!
//! `TestHarness` owns a World and auto-advances the sequence counter
//! before each dispatch. No drivers needed — just handlers and assertions.
//!
//! Sections:
//! 1. Basic TestHarness setup and dispatch
//! 2. dispatch_many for batch testing
//! 3. Testing change detection
//! 4. Testing with multiple handlers and shared state
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example testing_example
//! ```

// Param types (Res, ResMut) are taken by value — that's the API contract.
#![allow(clippy::needless_pass_by_value)]

use nexus_rt::testing::TestHarness;
use nexus_rt::{Handler, IntoHandler, Res, ResMut, WorldBuilder, new_resource};

new_resource!(Total(u64));
new_resource!(MaxVal(f64));
new_resource!(Log(Vec<String>));

// =============================================================================
// Handlers under test
// =============================================================================

fn accumulate(mut total: ResMut<Total>, event: u64) {
    **total += event;
}

fn track_max(mut max: ResMut<MaxVal>, event: f64) {
    if event > **max {
        **max = event;
    }
}

fn check_changed(val: Res<Total>, mut log: ResMut<Log>, _event: ()) {
    let status = if val.is_changed() {
        "changed"
    } else {
        "unchanged"
    };
    log.push(format!("val={}, {status}", val.0));
}

fn main() {
    // --- 1. Basic setup ---

    println!("=== 1. Basic TestHarness ===\n");

    let mut builder = WorldBuilder::new();
    builder.register(Total(0));
    let mut harness = TestHarness::new(builder);

    let mut handler = accumulate.into_handler(harness.registry());

    harness.dispatch(&mut handler, 10u64);
    harness.dispatch(&mut handler, 5u64);

    let total = harness.world().resource::<Total>().0;
    println!("  total after 10+5: {total}");
    assert_eq!(total, 15);

    // --- 2. dispatch_many for batch testing ---

    println!("\n=== 2. dispatch_many ===\n");

    let mut builder = WorldBuilder::new();
    builder.register(MaxVal(f64::NEG_INFINITY));
    let mut harness = TestHarness::new(builder);

    let mut handler = track_max.into_handler(harness.registry());
    harness.dispatch_many(&mut handler, [3.0, 7.0, 2.0, 9.0, 1.0]);

    let max = harness.world().resource::<MaxVal>().0;
    println!("  max of [3, 7, 2, 9, 1]: {max}");
    assert!((max - 9.0).abs() < f64::EPSILON);

    // --- 3. Testing change detection ---
    //
    // is_changed() returns true only within the SAME sequence as the write.
    // Advance the sequence manually, write, then run the checker in that
    // same sequence so it sees is_changed()=true.

    println!("\n=== 3. Change detection ===\n");

    let mut builder = WorldBuilder::new();
    builder.register(Total(0));
    builder.register(Log(Vec::new()));
    let mut harness = TestHarness::new(builder);

    let mut checker = check_changed.into_handler(harness.registry());

    // Advance sequence, write at that sequence, then run checker directly
    // in the same sequence — it sees is_changed()=true
    harness.world_mut().next_sequence();
    *harness.world_mut().resource_mut::<Total>() = Total(42);
    checker.run(harness.world_mut(), ());

    // dispatch advances sequence before running — no write at new sequence — unchanged
    harness.dispatch(&mut checker, ());

    let log = harness.world().resource::<Log>();
    for entry in log.iter() {
        println!("  {entry}");
    }
    assert!(log[0].contains("changed"));
    assert!(log[1].contains("unchanged"));

    // --- 4. Multiple handlers, shared state ---
    //
    // Multiple handlers can touch the same resources.
    // TestHarness advances sequence per dispatch call,
    // giving each handler its own sequence context.

    println!("\n=== 4. Multiple handlers ===\n");

    let mut builder = WorldBuilder::new();
    builder.register(Total(0));
    builder.register(MaxVal(f64::NEG_INFINITY));
    let mut harness = TestHarness::new(builder);

    let mut adder = accumulate.into_handler(harness.registry());
    let mut maxer = track_max.into_handler(harness.registry());

    // Interleave dispatches — different event types, shared world
    harness.dispatch(&mut adder, 30u64);
    harness.dispatch(&mut maxer, 5.0);
    harness.dispatch(&mut adder, 40u64);
    harness.dispatch(&mut maxer, 9.0);
    harness.dispatch(&mut adder, 50u64);
    harness.dispatch(&mut maxer, 3.0);

    let total = harness.world().resource::<Total>().0;
    let max = harness.world().resource::<MaxVal>().0;
    println!("  total: {total}, max: {max}");
    assert_eq!(total, 120);
    assert!((max - 9.0).abs() < f64::EPSILON);

    // world_mut() is available for manual state manipulation
    *harness.world_mut().resource_mut::<Total>() = Total(0);
    assert_eq!(harness.world().resource::<Total>().0, 0);
    println!("  reset total via world_mut(): 0");

    println!("\nDone.");
}
