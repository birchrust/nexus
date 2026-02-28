//! Optional resource dependencies with `Option<Res<T>>` / `Option<ResMut<T>>`.
//!
//! Systems that use `Option<Res<T>>` or `Option<ResMut<T>>` will run
//! regardless of whether `T` is registered in World:
//!
//! - **Registered:** the system receives `Some(Res<T>)` / `Some(ResMut<T>)`
//! - **Not registered:** the system receives `None`
//!
//! This is resolved at build time via `registry.try_id::<T>()`. There is
//! no runtime overhead — the Option<ResourceId> state is cached once.
//!
//! Use cases:
//! - Feature flags (enable debug logging only if DebugConfig is registered)
//! - Graceful degradation (metrics sink may or may not exist)
//! - Plugin systems (optional extensions that aren't always present)
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example optional_resources
//! ```

use nexus_rt::{IntoSystem, Res, ResMut, System, WorldBuilder};

// -- Domain types ------------------------------------------------------------

struct Config {
    threshold: f64,
}

/// Optional — only present if debug mode is enabled.
struct DebugLog {
    entries: Vec<String>,
}

/// Optional — only present if metrics are enabled.
struct Metrics {
    events_processed: u64,
}

// -- Systems -----------------------------------------------------------------

/// Always runs. Uses Config (required) and DebugLog (optional).
fn process_event(config: Res<Config>, mut debug: Option<ResMut<DebugLog>>, value: f64) {
    let above = value > config.threshold;
    println!(
        "[process] value={:.1}, threshold={:.1}, above={}",
        value, config.threshold, above
    );

    if let Some(ref mut log) = debug {
        log.entries
            .push(format!("processed {:.1} (above={})", value, above));
        println!("  -> logged to debug ({} entries)", log.entries.len());
    }
}

/// Always runs. Increments metrics if they exist.
fn track_metrics(metrics: Option<ResMut<Metrics>>, _event: f64) {
    match metrics {
        Some(mut m) => {
            m.events_processed += 1;
            println!("[metrics] events_processed = {}", m.events_processed);
        }
        None => {
            println!("[metrics] no metrics sink registered — skipping");
        }
    }
}

fn main() {
    println!("=== Scenario 1: All optional resources present ===\n");
    {
        let mut builder = WorldBuilder::new();
        builder
            .register(Config { threshold: 5.0 })
            .register(DebugLog {
                entries: Vec::new(),
            })
            .register(Metrics {
                events_processed: 0,
            });
        let mut world = builder.build();

        let mut process = process_event.into_system(world.registry_mut());
        let mut track = track_metrics.into_system(world.registry_mut());

        for value in [3.0, 7.5, 1.2] {
            process.run(&mut world, value);
            track.run(&mut world, value);
            println!();
        }

        let log = world.resource::<DebugLog>();
        println!("Debug log entries: {:?}", log.entries);

        let metrics = world.resource::<Metrics>();
        println!("Events processed: {}", metrics.events_processed);
    }

    println!("\n=== Scenario 2: No optional resources ===\n");
    {
        let mut builder = WorldBuilder::new();
        builder.register(Config { threshold: 5.0 });
        // DebugLog and Metrics intentionally not registered.
        let mut world = builder.build();

        let mut process = process_event.into_system(world.registry_mut());
        let mut track = track_metrics.into_system(world.registry_mut());

        for value in [3.0, 7.5] {
            process.run(&mut world, value);
            track.run(&mut world, value);
            println!();
        }

        println!("Systems ran cleanly without optional resources.");
    }

    println!("\nDone.");
}
