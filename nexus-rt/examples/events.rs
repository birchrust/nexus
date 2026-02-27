//! Event-driven communication between systems.
//!
//! Systems communicate through [`Events<T>`] buffers registered in World.
//! Writers push events during dispatch, readers consume them afterward.
//!
//! Key concepts:
//! - [`EventWriter<T>`] — exclusive write access to `Events<T>` (one writer)
//! - [`EventReader<T>`] — shared read access to `Events<T>` (many readers)
//! - Clearing is the runtime's job — call `events.clear()` between ticks
//! - `register_default::<Events<T>>()` is a convenient shorthand
//!
//! This example shows a pipeline: sensor readings → threshold detection →
//! alert accumulation, all connected through event buffers.
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example events
//! ```

use nexus_rt::{EventReader, EventWriter, Events, IntoSystem, Local, ResMut, System, WorldBuilder};

// -- Event types -------------------------------------------------------------

struct Alert {
    sensor: &'static str,
    value: f64,
}

// -- Incoming event (dispatch event, not Events<T>) --------------------------

struct SensorReading {
    sensor: &'static str,
    value: f64,
    threshold: f64,
}

// -- Systems -----------------------------------------------------------------

/// Checks a sensor reading against its threshold. Emits Alert if exceeded.
/// Uses Local<u64> to count how many alerts this system has produced.
fn threshold_check(
    mut alerts: EventWriter<Alert>,
    mut alert_count: Local<u64>,
    reading: SensorReading,
) {
    if reading.value > reading.threshold {
        *alert_count += 1;
        println!(
            "[threshold] {} = {:.1} > {:.1} — alert #{} emitted",
            reading.sensor, reading.value, reading.threshold, *alert_count
        );
        alerts.send(Alert {
            sensor: reading.sensor,
            value: reading.value,
        });
    } else {
        println!(
            "[threshold] {} = {:.1} <= {:.1} — ok",
            reading.sensor, reading.value, reading.threshold
        );
    }
}

/// Reads all alerts accumulated during dispatch and records them.
fn collect_alerts(reader: EventReader<Alert>, mut log: ResMut<Vec<String>>, _: ()) {
    for alert in reader.iter() {
        let entry = format!("ALERT: {} = {:.1}", alert.sensor, alert.value);
        println!("[collect] {}", entry);
        log.push(entry);
    }
}

/// A second reader — multiple readers can coexist.
fn count_alerts(reader: EventReader<Alert>, mut total: ResMut<u64>, _: ()) {
    let n = reader.len();
    *total += n as u64;
    if n > 0 {
        println!("[count] {} alerts this tick, {} total", n, *total);
    }
}

fn main() {
    let mut builder = WorldBuilder::new();
    builder
        .register_default::<Events<Alert>>()
        .register::<Vec<String>>(Vec::new())
        .register::<u64>(0);
    let mut world = builder.build();

    let mut check = threshold_check.into_system(world.registry());
    let mut collect = collect_alerts.into_system(world.registry());
    let mut count = count_alerts.into_system(world.registry());

    // -- Tick 1: several sensor readings --------------------------------------

    println!("=== Tick 1 ===\n");

    let readings = [
        SensorReading {
            sensor: "temp",
            value: 85.0,
            threshold: 80.0,
        },
        SensorReading {
            sensor: "pressure",
            value: 50.0,
            threshold: 100.0,
        },
        SensorReading {
            sensor: "voltage",
            value: 250.0,
            threshold: 240.0,
        },
    ];

    // Dispatch phase: fire each reading into the check system.
    for reading in readings {
        check.run(&mut world, reading);
    }

    // Post-dispatch: readers consume accumulated alerts.
    println!();
    collect.run(&mut world, ());
    count.run(&mut world, ());

    // Clear event buffer between ticks (runtime's responsibility).
    world.resource_mut::<Events<Alert>>().clear();

    // -- Tick 2: more readings ------------------------------------------------

    println!("\n=== Tick 2 ===\n");

    let readings = [
        SensorReading {
            sensor: "temp",
            value: 75.0,
            threshold: 80.0,
        },
        SensorReading {
            sensor: "pressure",
            value: 150.0,
            threshold: 100.0,
        },
    ];

    for reading in readings {
        check.run(&mut world, reading);
    }

    println!();
    collect.run(&mut world, ());
    count.run(&mut world, ());
    world.resource_mut::<Events<Alert>>().clear();

    // -- Results --------------------------------------------------------------

    println!("\n=== Results ===\n");
    let log = world.resource::<Vec<String>>();
    println!("Alert log ({} entries):", log.len());
    for entry in log.iter() {
        println!("  {}", entry);
    }
    println!("Total alerts: {}", world.resource::<u64>());

    assert_eq!(*world.resource::<u64>(), 3);
    assert_eq!(world.resource::<Vec<String>>().len(), 3);

    println!("\nDone.");
}
