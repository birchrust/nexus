//! Per-system local state with `Local<T>`.
//!
//! `Local<T>` stores state inside the [`FunctionSystem`] itself, not in
//! [`World`]. Each system instance gets its own independent copy, initialized
//! with `T::default()`.
//!
//! Use cases:
//! - Per-system counters (events processed, errors seen)
//! - Per-system buffers (batch accumulation before flush)
//! - Per-system cursors (tracking read position in a log)
//!
//! `Local<T>` does **not** need to be registered in World. It lives entirely
//! within the system's internal state.
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example local_state
//! ```

use nexus_rt::{IntoSystem, Local, ResMut, System, WorldBuilder};

// -- Example 1: Simple counter -----------------------------------------------

/// Each invocation increments the local counter.
/// The counter persists across `run()` calls on the same system instance.
fn counting_system(mut count: Local<u64>, event: &'static str) {
    *count += 1;
    println!("[counter] call #{}: event = {}", *count, event);
}

// -- Example 2: Independent instances ----------------------------------------

/// Two instances of the same function get independent local state.
fn accumulator(mut sum: Local<i64>, mut total: ResMut<i64>, value: i64) {
    *sum += value;
    *total += value;
    println!("[accumulator] local_sum={}, world_total={}", *sum, *total);
}

// -- Example 3: Batch buffer -------------------------------------------------

/// Accumulates events locally, flushes to World every N events.
fn batch_writer(mut buf: Local<Vec<u32>>, mut output: ResMut<Vec<u32>>, value: u32) {
    buf.push(value);
    if buf.len() >= 3 {
        println!("[batch] flushing {} events to output", buf.len());
        output.extend(buf.drain(..));
    }
}

fn main() {
    println!("=== Example 1: Simple counter ===\n");
    {
        let mut world = WorldBuilder::new().build();
        let mut sys = counting_system.into_system(world.registry_mut());

        sys.run(&mut world, "alpha");
        sys.run(&mut world, "beta");
        sys.run(&mut world, "gamma");
    }

    println!("\n=== Example 2: Independent instances ===\n");
    {
        let mut builder = WorldBuilder::new();
        builder.register::<i64>(0);
        let mut world = builder.build();

        // Two systems from the same function — each has its own Local<i64>.
        let mut sys_a = accumulator.into_system(world.registry_mut());
        let mut sys_b = accumulator.into_system(world.registry_mut());

        println!("sys_a gets 10:");
        sys_a.run(&mut world, 10i64);

        println!("sys_b gets 20:");
        sys_b.run(&mut world, 20i64);

        println!("sys_a gets 5:");
        sys_a.run(&mut world, 5i64);

        // sys_a local: 15, sys_b local: 20, world total: 35
        println!("\nWorld total: {}", world.resource::<i64>());
        assert_eq!(*world.resource::<i64>(), 35);
    }

    println!("\n=== Example 3: Batch buffer ===\n");
    {
        let mut builder = WorldBuilder::new();
        builder.register::<Vec<u32>>(Vec::new());
        let mut world = builder.build();

        let mut sys = batch_writer.into_system(world.registry_mut());

        // First two events accumulate locally.
        sys.run(&mut world, 1u32);
        println!("  output len: {}", world.resource::<Vec<u32>>().len());

        sys.run(&mut world, 2u32);
        println!("  output len: {}", world.resource::<Vec<u32>>().len());

        // Third event triggers flush.
        sys.run(&mut world, 3u32);
        println!("  output len: {}", world.resource::<Vec<u32>>().len());

        // Fourth event starts a new batch.
        sys.run(&mut world, 4u32);
        println!("  output len: {}", world.resource::<Vec<u32>>().len());

        let output = world.resource::<Vec<u32>>();
        println!("\nFinal output: {:?}", &*output);
        assert_eq!(&*output, &[1, 2, 3]);
    }

    println!("\nDone.");
}
