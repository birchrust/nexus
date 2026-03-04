//! Scheduler DAG — reconciliation systems with boolean propagation.
//!
//! Demonstrates a realistic scheduler setup: market data changes propagate
//! through a DAG of reconciliation systems. Systems use `changed_after`
//! to detect whether upstream resources were modified since the last pass.
//!
//! Run with:
//! ```bash
//! cargo run --release -p nexus-rt --example scheduler_dag
//! ```

#![allow(clippy::needless_pass_by_value)]

use nexus_rt::scheduler::{SchedulerInstaller, SchedulerTick};
use nexus_rt::{Handler, IntoHandler, Res, ResMut, WorldBuilder};

// ── Domain types ────────────────────────────────────────────────────────

struct MidPrice(f64);

struct TheoreticalValue(f64);

struct SpreadBps(f64);

struct QuoteState {
    bid: f64,
    ask: f64,
}

struct RiskFlag(bool);

// ── Systems ─────────────────────────────────────────────────────────────

/// Recompute theoretical value from mid price. Propagates only if the
/// mid actually changed since last scheduler pass.
fn compute_theo(
    mid: Res<MidPrice>,
    tick: Res<SchedulerTick>,
    mut theo: ResMut<TheoreticalValue>,
) -> bool {
    if mid.changed_after(tick.last()) {
        theo.0 = mid.0 * 1.001; // trivial model
        true
    } else {
        false
    }
}

/// Recompute quotes from theoretical value and spread.
fn compute_quotes(
    theo: Res<TheoreticalValue>,
    spread: Res<SpreadBps>,
    mut quotes: ResMut<QuoteState>,
) -> bool {
    let half_spread = theo.0 * spread.0 / 10_000.0 / 2.0;
    quotes.bid = theo.0 - half_spread;
    quotes.ask = theo.0 + half_spread;
    true
}

/// Check risk limits after quote update. Could gate downstream publishing.
fn check_risk(quotes: Res<QuoteState>, mut flag: ResMut<RiskFlag>) -> bool {
    let spread = quotes.ask - quotes.bid;
    flag.0 = spread < 100.0; // within limits
    flag.0
}

// ── Event handler (simulated market data feed) ──────────────────────────

fn on_market_tick(mut mid: ResMut<MidPrice>, new_price: f64) {
    mid.0 = new_price;
}

// ── main ────────────────────────────────────────────────────────────────

fn main() {
    // -- Build ----------------------------------------------------------------

    let mut wb = WorldBuilder::new();
    wb.register(MidPrice(50_000.0));
    wb.register(TheoreticalValue(0.0));
    wb.register(SpreadBps(10.0)); // 10 bps
    wb.register(QuoteState { bid: 0.0, ask: 0.0 });
    wb.register(RiskFlag(false));

    // Pre-register SchedulerTick so systems reading it can resolve params.
    wb.ensure(SchedulerTick::default());

    // Build the DAG: compute_theo → compute_quotes → check_risk
    let mut installer = SchedulerInstaller::new();
    let theo = installer.add(compute_theo, wb.registry());
    let quotes = installer.add(compute_quotes, wb.registry());
    let risk = installer.add(check_risk, wb.registry());
    installer.after(quotes, theo);
    installer.after(risk, quotes);

    let mut scheduler = wb.install_driver(installer);

    // Event handler for market data
    let mut market_handler = on_market_tick.into_handler(wb.registry_mut());

    let mut world = wb.build();

    // -- Simulate event loop --------------------------------------------------

    let prices = [50_100.0, 50_200.0, 50_200.0, 50_300.0];

    for (i, &price) in prices.iter().enumerate() {
        // Event phase: driver delivers market data
        world.next_sequence();
        market_handler.run(&mut world, price);

        // Reconciliation phase: scheduler runs after events
        let ran = scheduler.run(&mut world);

        let quotes = world.resource::<QuoteState>();
        let risk = world.resource::<RiskFlag>();

        println!(
            "pass {}: mid={:.0}, ran={}/{}, bid={:.2}, ask={:.2}, risk_ok={}",
            i + 1,
            price,
            ran,
            3,
            quotes.bid,
            quotes.ask,
            risk.0,
        );
    }

    // Verify: third pass should skip because mid didn't change
    // (50_200 → 50_200, no DerefMut → changed_after returns false)

    println!("\nDone. Pass 3 ran all systems because the handler always writes via DerefMut.");
    println!("To get skip behavior, the handler must check before writing:");
    println!("  if mid.0 != new_price {{ mid.0 = new_price; }}");
}
