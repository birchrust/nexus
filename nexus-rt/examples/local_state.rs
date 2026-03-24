//! Per-handler local state with `Local<T>`.
//!
//! `Local<T>` stores state inside the handler itself, not in
//! [`World`]. Each handler instance gets its own independent copy, initialized
//! with `T::default()`.
//!
//! Use cases:
//! - Per-handler counters (events processed, errors seen)
//! - Per-handler buffers (batch accumulation before flush)
//! - Per-handler cursors (tracking read position in a log)
//!
//! `Local<T>` does **not** need to be registered in World. It lives entirely
//! within the handler's internal state.
//!
//! Run with:
//! ```bash
//! cargo run -p nexus-rt --example local_state
//! ```

use nexus_rt::{Handler, IntoHandler, Local, ResMut, WorldBuilder, new_resource};

new_resource!(RunningTotal(i64));
new_resource!(Output(Vec<u32>));

// -- Example 1: Simple counter -----------------------------------------------

/// Each invocation increments the local counter.
/// The counter persists across `run()` calls on the same handler instance.
fn counting_handler(mut count: Local<u64>, event: &'static str) {
    *count += 1;
    println!("[counter] call #{}: event = {}", *count, event);
}

// -- Example 2: Independent instances ----------------------------------------

/// Two instances of the same function get independent local state.
fn accumulator(mut sum: Local<i64>, mut total: ResMut<RunningTotal>, value: i64) {
    *sum += value;
    **total += value;
    println!("[accumulator] local_sum={}, world_total={}", *sum, total.0);
}

// -- Example 3: Batch buffer -------------------------------------------------

/// Accumulates events locally, flushes to World every N events.
fn batch_writer(mut buf: Local<Vec<u32>>, mut output: ResMut<Output>, value: u32) {
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
        let mut sys = counting_handler.into_handler(world.registry());

        sys.run(&mut world, "alpha");
        sys.run(&mut world, "beta");
        sys.run(&mut world, "gamma");
    }

    println!("\n=== Example 2: Independent instances ===\n");
    {
        let mut builder = WorldBuilder::new();
        builder.register(RunningTotal(0));
        let mut world = builder.build();

        // Two handlers from the same function — each has its own Local<i64>.
        let mut sys_a = accumulator.into_handler(world.registry());
        let mut sys_b = accumulator.into_handler(world.registry());

        println!("sys_a gets 10:");
        sys_a.run(&mut world, 10i64);

        println!("sys_b gets 20:");
        sys_b.run(&mut world, 20i64);

        println!("sys_a gets 5:");
        sys_a.run(&mut world, 5i64);

        // sys_a local: 15, sys_b local: 20, world total: 35
        println!("\nWorld total: {}", world.resource::<RunningTotal>().0);
        assert_eq!(world.resource::<RunningTotal>().0, 35);
    }

    println!("\n=== Example 3: Batch buffer ===\n");
    {
        let mut builder = WorldBuilder::new();
        builder.register(Output(Vec::new()));
        let mut world = builder.build();

        let mut sys = batch_writer.into_handler(world.registry());

        // First two events accumulate locally.
        sys.run(&mut world, 1u32);
        println!("  output len: {}", world.resource::<Output>().len());

        sys.run(&mut world, 2u32);
        println!("  output len: {}", world.resource::<Output>().len());

        // Third event triggers flush.
        sys.run(&mut world, 3u32);
        println!("  output len: {}", world.resource::<Output>().len());

        // Fourth event starts a new batch.
        sys.run(&mut world, 4u32);
        println!("  output len: {}", world.resource::<Output>().len());

        let output = world.resource::<Output>();
        println!("\nFinal output: {:?}", &**output);
        assert_eq!(&**output, &[1, 2, 3]);
    }

    println!("\nDone.");
}
