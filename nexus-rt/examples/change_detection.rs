//! Change detection patterns — sequence-based mutation tracking.
//!
//! nexus-rt tracks mutations via monotonic sequence numbers. Each resource
//! has a `changed_at` stamp. `ResMut<T>` auto-stamps on `DerefMut`.
//! `Res<T>::is_changed()` and `Res<T>::changed_after(seq)` read the stamp.
//!
//! Sections:
//! 1. Basic is_changed() — detect current-sequence writes
//! 2. ResMut stamps on write (DerefMut)
//! 3. changed_after() — checkpoint-based detection
//! 4. Integration with scheduler (SchedulerTick)
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example change_detection
//! ```

// Param types (Res, ResMut) are taken by value — that's the API contract.
#![allow(clippy::needless_pass_by_value)]

use nexus_rt::scheduler::{SchedulerInstaller, SchedulerTick};
use nexus_rt::{Handler, IntoHandler, Res, ResMut, WorldBuilder};

// =============================================================================
// Domain types
// =============================================================================

struct Price(f64);

struct Quote {
    bid: f64,
    ask: f64,
}

// =============================================================================
// Handlers and systems
// =============================================================================

fn write_price(mut price: ResMut<Price>, new_price: f64) {
    price.0 = new_price; // stamps changed_at = current sequence
}

fn check_price_changed(price: Res<Price>, mut log: ResMut<Vec<String>>, _event: ()) {
    if price.is_changed() {
        log.push(format!("price changed to {:.2}", price.0));
    } else {
        log.push("price unchanged".to_string());
    }
}

fn check_after_checkpoint(
    price: Res<Price>,
    mut log: ResMut<Vec<String>>,
    checkpoint: nexus_rt::Sequence,
) {
    let is_now = price.is_changed();
    let since = price.changed_after(checkpoint);
    log.push(format!(
        "price={:.2}, is_changed={is_now}, changed_after(checkpoint)={since}",
        price.0
    ));
}

/// System: recompute quotes only if price changed since last scheduler pass.
fn recompute_quotes(
    price: Res<Price>,
    tick: Res<SchedulerTick>,
    mut quote: ResMut<Quote>,
) -> bool {
    if price.changed_after(tick.last()) {
        let spread = price.0 * 0.001;
        quote.bid = price.0 - spread;
        quote.ask = price.0 + spread;
        true
    } else {
        false
    }
}

fn main() {
    // --- 1. Basic is_changed() ---

    println!("=== 1. is_changed() ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(Price(100.0));
    wb.register::<Vec<String>>(Vec::new());
    let mut world = wb.build();

    let mut writer = write_price.into_handler(world.registry());
    let mut checker = check_price_changed.into_handler(world.registry());

    // Sequence 1: write price, then check — should see change
    world.next_sequence();
    writer.run(&mut world, 105.0);
    checker.run(&mut world, ());

    // Sequence 2: don't write, just check — no change
    world.next_sequence();
    checker.run(&mut world, ());

    // Sequence 3: write again, then check — should see change
    world.next_sequence();
    writer.run(&mut world, 110.0);
    checker.run(&mut world, ());

    let log = world.resource::<Vec<String>>();
    for entry in log {
        println!("  {entry}");
    }
    assert_eq!(log.len(), 3);
    assert!(log[0].contains("changed to 105"));
    assert!(log[1].contains("unchanged"));
    assert!(log[2].contains("changed to 110"));

    // --- 2. DerefMut stamping ---
    //
    // ResMut stamps changed_at when you dereference mutably,
    // even if the value doesn't actually change.

    println!("\n=== 2. DerefMut stamping ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(Price(100.0));
    wb.register::<Vec<String>>(Vec::new());
    let mut world = wb.build();

    let mut checker = check_price_changed.into_handler(world.registry());

    // Write the same value — DerefMut still stamps
    world.next_sequence();
    world.resource_mut::<Price>().0 = 100.0; // same value, but stamps!
    checker.run(&mut world, ());

    let log = world.resource::<Vec<String>>();
    println!("  {}", log[0]);
    assert!(log[0].contains("changed"));
    println!("  (DerefMut stamps even when value is the same)");

    // --- 3. changed_after() with checkpoint ---
    //
    // changed_after(seq) returns true if the resource was written after
    // the given sequence. Useful for detecting changes across multiple
    // events — not just the current one.

    println!("\n=== 3. changed_after() ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(Price(100.0));
    wb.register::<Vec<String>>(Vec::new());
    let mut world = wb.build();

    let mut writer = write_price.into_handler(world.registry());
    let mut checker = check_after_checkpoint.into_handler(world.registry());

    // Record checkpoint before any events
    let checkpoint = world.current_sequence();

    // Sequence 1: write price
    world.next_sequence();
    writer.run(&mut world, 105.0);
    checker.run(&mut world, checkpoint);

    // Sequence 2: no write — is_changed is false, but changed_after is still true
    world.next_sequence();
    checker.run(&mut world, checkpoint);

    let log = world.resource::<Vec<String>>();
    for entry in log {
        println!("  {entry}");
    }
    assert!(log[0].contains("is_changed=true"));
    assert!(log[0].contains("changed_after(checkpoint)=true"));
    assert!(log[1].contains("is_changed=false"));
    assert!(log[1].contains("changed_after(checkpoint)=true"));

    // --- 4. Scheduler integration with SchedulerTick ---

    println!("\n=== 4. SchedulerTick ===\n");

    let mut wb = WorldBuilder::new();
    wb.register(Price(50_000.0));
    wb.register(Quote { bid: 0.0, ask: 0.0 });
    wb.ensure(SchedulerTick::default());

    let mut installer = SchedulerInstaller::new();
    installer.add(recompute_quotes, wb.registry());
    let mut scheduler = wb.install_driver(installer);
    let mut writer = write_price.into_handler(wb.registry_mut());
    let mut world = wb.build();

    // Pass 1: price changed → quotes recomputed
    world.next_sequence();
    writer.run(&mut world, 50_100.0);
    let ran = scheduler.run(&mut world);
    let q = world.resource::<Quote>();
    println!("  pass 1: ran={ran}, bid={:.2}, ask={:.2}", q.bid, q.ask);
    assert_eq!(ran, 1);
    assert!((q.bid - 50_049.9).abs() < 0.2);

    // Pass 2: no price change → system detects no change via changed_after
    world.next_sequence();
    let ran = scheduler.run(&mut world);
    println!("  pass 2: ran={ran} (system runs but returns false — no price change)");
    assert_eq!(ran, 1); // root always runs, but it returns false

    println!("\nDone.");
}
