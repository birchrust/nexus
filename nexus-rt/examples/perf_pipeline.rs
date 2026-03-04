//! Pipeline + Handler dispatch codegen inspection + latency benchmark.
//!
//! Run asm inspection (pipelines):
//! ```bash
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::bare_3stage_run
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::option_3stage_run
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::world_access_run
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::boxed_pipeline_run
//! ```
//!
//! Run asm inspection (Handler dispatch):
//! ```bash
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_handler_res_read
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_handler_res_mut
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_handler_two_res
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_dyn_handler
//! ```
//!
//! Run asm inspection (combinators + adapters):
//! ```bash
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_cloned_pipeline
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_dispatch_pipeline
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_fanout_2way
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_broadcast
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_cloned_adapter
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_byref_adapter
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_adapt
//! ```
//!
//! Run benchmark:
//! ```bash
//! taskset -c 0 cargo run --release -p nexus-rt --example perf_pipeline
//! ```

use std::hint::black_box;

use nexus_rt::{
    Adapt, Broadcast, ByRef, Cloned, Handler, IntoHandler, PipelineStart, Res, ResMut,
    WorldBuilder, fan_out,
};

// =============================================================================
// Bench infrastructure
// =============================================================================

const ITERATIONS: usize = 100_000;
const WARMUP: usize = 10_000;
const BATCH: u64 = 100;

#[inline(always)]
#[cfg(target_arch = "x86_64")]
fn rdtsc_start() -> u64 {
    unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    }
}

#[inline(always)]
#[cfg(target_arch = "x86_64")]
fn rdtsc_end() -> u64 {
    unsafe {
        let mut aux = 0u32;
        let tsc = core::arch::x86_64::__rdtscp(&raw mut aux);
        core::arch::x86_64::_mm_lfence();
        tsc
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn bench_batched<F: FnMut() -> u64>(name: &str, mut f: F) -> (u64, u64, u64) {
    for _ in 0..WARMUP {
        black_box(f());
    }
    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            black_box(f());
        }
        let end = rdtsc_end();
        samples.push(end.wrapping_sub(start) / BATCH);
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 50.0);
    let p99 = percentile(&samples, 99.0);
    let p999 = percentile(&samples, 99.9);
    println!("{:<44} {:>8} {:>8} {:>8}", name, p50, p99, p999);
    (p50, p99, p999)
}

fn print_header(title: &str) {
    println!("=== {} ===\n", title);
    println!(
        "{:<44} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(72));
}

// =============================================================================
// Pipeline codegen probes
// =============================================================================

/// 3-stage bare pipeline: multiply, add, shift.
#[inline(never)]
pub fn bare_3stage_run(
    p: &mut nexus_rt::PipelineBuilder<u64, u64, impl FnMut(&mut nexus_rt::World, u64) -> u64>,
    world: &mut nexus_rt::World,
    input: u64,
) -> u64 {
    p.run(world, input)
}

/// 3-stage Option pipeline: Some, map, filter.
#[inline(never)]
pub fn option_3stage_run(
    p: &mut nexus_rt::PipelineBuilder<
        u64,
        Option<u64>,
        impl FnMut(&mut nexus_rt::World, u64) -> Option<u64>,
    >,
    world: &mut nexus_rt::World,
    input: u64,
) -> Option<u64> {
    p.run(world, input)
}

/// Pipeline that reads World via pre-resolved Res<T> stages.
#[inline(never)]
pub fn world_access_run(
    p: &mut nexus_rt::PipelineBuilder<u64, u64, impl FnMut(&mut nexus_rt::World, u64) -> u64>,
    world: &mut nexus_rt::World,
    input: u64,
) -> u64 {
    p.run(world, input)
}

/// Built Pipeline through dyn dispatch.
#[inline(never)]
pub fn boxed_pipeline_run(
    p: &mut dyn nexus_rt::Handler<u64>,
    world: &mut nexus_rt::World,
    input: u64,
) {
    p.run(world, input);
}

/// Baseline: equivalent hand-written function (no pipeline).
#[inline(never)]
pub fn baseline_handwritten(world: &mut nexus_rt::World, input: u64) -> u64 {
    let x = input.wrapping_mul(3);
    let x = x.wrapping_add(7);
    let _ = world;
    x >> 1
}

// =============================================================================
// Stage functions for World-accessing pipeline
// =============================================================================

fn add_resource(val: Res<u64>, x: u64) -> u64 {
    x.wrapping_add(*val)
}

fn mul_resource(val: Res<u64>, x: u64) -> u64 {
    x.wrapping_mul(*val)
}

fn sub_resource(val: Res<u32>, x: u64) -> u64 {
    x.wrapping_sub(*val as u64)
}

// =============================================================================
// Handler dispatch probes — Param fetch hot path
// =============================================================================

fn handler_res_read(counter: Res<u64>, input: u64) {
    black_box((*counter).wrapping_add(input));
}

fn handler_res_mut_write(mut counter: ResMut<u64>, input: u64) {
    *counter = (*counter).wrapping_add(input);
}

fn handler_two_res(a: Res<u64>, b: Res<u32>, input: u64) {
    black_box((*a).wrapping_add(input).wrapping_add(*b as u64));
}

/// Monomorphized Handler dispatch with Res<u64>.
/// Full path: Handler::run → Param::fetch → World::get_ptr + changed_at + current_sequence.
#[inline(never)]
pub fn probe_handler_res_read(
    sys: &mut impl Handler<u64>,
    world: &mut nexus_rt::World,
    input: u64,
) {
    sys.run(world, input);
}

/// Monomorphized Handler dispatch with ResMut<u64>.
/// Full path: fetch + DerefMut stamps changed_at on write.
#[inline(never)]
pub fn probe_handler_res_mut(sys: &mut impl Handler<u64>, world: &mut nexus_rt::World, input: u64) {
    sys.run(world, input);
}

/// Monomorphized Handler dispatch with two Res params (tuple fetch).
#[inline(never)]
pub fn probe_handler_two_res(sys: &mut impl Handler<u64>, world: &mut nexus_rt::World, input: u64) {
    sys.run(world, input);
}

/// Dyn-dispatched Handler — vtable call + Param fetch.
#[inline(never)]
pub fn probe_dyn_handler(sys: &mut dyn Handler<u64>, world: &mut nexus_rt::World, input: u64) {
    sys.run(world, input);
}

// =============================================================================
// Combinator + Adapter codegen probes
// =============================================================================

/// Pipeline .cloned(): &u64 → u64 transition.
/// Should compile to: chain call → move (Copy type elides clone).
#[inline(never)]
pub fn probe_cloned_pipeline<'a>(
    p: &mut nexus_rt::PipelineBuilder<
        &'a u64,
        u64,
        impl FnMut(&mut nexus_rt::World, &'a u64) -> u64,
    >,
    world: &mut nexus_rt::World,
    input: &'a u64,
) -> u64 {
    p.run(world, input)
}

/// Pipeline .dispatch() to built handler.
/// Should compile to: chain call → direct handler call.
#[inline(never)]
pub fn probe_dispatch_pipeline(p: &mut impl Handler<u64>, world: &mut nexus_rt::World, input: u64) {
    p.run(world, input);
}

/// FanOut 2-way: monomorphized tuple dispatch.
/// Should compile to: 2 direct handler calls (no loop).
#[inline(never)]
pub fn probe_fanout_2way(h: &mut impl Handler<u64>, world: &mut nexus_rt::World, input: u64) {
    h.run(world, input);
}

/// Broadcast 2-way: dynamic dispatch baseline.
/// Expect: Vec iteration + 2 vtable calls.
#[inline(never)]
pub fn probe_broadcast(h: &mut Broadcast<u64>, world: &mut nexus_rt::World, input: u64) {
    h.run(world, input);
}

/// Cloned adapter: Handler<u64> wrapped as Handler<&u64>.
/// For Copy types, clone should be elided entirely.
#[inline(never)]
pub fn probe_cloned_adapter(
    h: &mut Cloned<impl Handler<u64>>,
    world: &mut nexus_rt::World,
    input: &u64,
) {
    h.run(world, input);
}

/// ByRef adapter: Handler<&u64> wrapped as Handler<u64>.
/// Should be zero-cost — just borrows the event.
#[inline(never)]
pub fn probe_byref_adapter<H: for<'e> Handler<&'e u64> + Send>(
    h: &mut ByRef<H>,
    world: &mut nexus_rt::World,
    input: u64,
) {
    h.run(world, input);
}

/// Adapt adapter: decode(u64) → Option<u64> → Handler<u64>.
/// Should be: decode call → branch on None → handler call.
#[inline(never)]
pub fn probe_adapt(
    h: &mut Adapt<impl FnMut(u64) -> Option<u64> + Send, impl Handler<u64>>,
    world: &mut nexus_rt::World,
    input: u64,
) {
    h.run(world, input);
}

// =============================================================================
// Stage functions for combinator/adapter setup
// =============================================================================

fn ref_identity(x: &u64) -> &u64 {
    x
}

fn ref_accumulate(mut total: ResMut<u64>, event: &u64) {
    *total = total.wrapping_add(*event);
}

fn decode_u64(x: u64) -> Option<u64> {
    if x > 0 { Some(x) } else { None }
}

// =============================================================================
// Main — benchmark
// =============================================================================

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register::<u64>(42);
    wb.register::<u32>(7);
    let mut world = wb.build();
    let r = world.registry_mut();

    // --- Bare 3-stage pipeline (no Option, no World access) ---

    let mut bare = PipelineStart::<u64>::new()
        .stage(|x: u64| x.wrapping_mul(3), r)
        .stage(|x: u64| x.wrapping_add(7), r)
        .stage(|x: u64| x >> 1, r);

    // --- Option 3-stage pipeline ---

    let mut option = PipelineStart::<u64>::new()
        .stage(
            |x: u64| -> Option<u64> { if x > 0 { Some(x) } else { None } },
            r,
        )
        .map(|x: u64| x.wrapping_mul(3), r)
        .filter(|_w, x| *x < 1_000_000);

    // --- World-accessing pipeline (pre-resolved via Res<T>) ---

    let mut world_resolved = PipelineStart::<u64>::new()
        .stage(add_resource, r)
        .stage(mul_resource, r);

    // --- World-accessing 3-stage pipeline ---

    let mut stage_3 = PipelineStart::<u64>::new()
        .stage(add_resource, r)
        .stage(mul_resource, r)
        .stage(sub_resource, r);

    // --- Built (boxed) pipeline ---

    let mut boxed = PipelineStart::<u64>::new()
        .stage(|x: u64| x.wrapping_mul(3), r)
        .stage(|x: u64| x.wrapping_add(7), r)
        .stage(|_x: u64| {}, r)
        .build();

    // --- Batch pipelines (same chains as their linear counterparts) ---

    fn sink(mut acc: ResMut<u64>, x: u64) {
        *acc = acc.wrapping_add(x);
    }

    // Bare: 3 compute stages + sink (same chain for both batch and linear)
    let mut batch_bare = PipelineStart::<u64>::new()
        .stage(|x: u64| x.wrapping_mul(3), r)
        .stage(|x: u64| x.wrapping_add(7), r)
        .stage(sink, r)
        .build_batch(1024);

    let mut linear_bare = PipelineStart::<u64>::new()
        .stage(|x: u64| x.wrapping_mul(3), r)
        .stage(|x: u64| x.wrapping_add(7), r)
        .stage(sink, r);

    // Res<T>: 3 world-access stages + sink (same chain for both)
    let mut batch_res = PipelineStart::<u64>::new()
        .stage(add_resource, r)
        .stage(mul_resource, r)
        .stage(sub_resource, r)
        .stage(sink, r)
        .build_batch(1024);

    let mut linear_res = PipelineStart::<u64>::new()
        .stage(add_resource, r)
        .stage(mul_resource, r)
        .stage(sub_resource, r)
        .stage(sink, r);

    // --- Result→catch→map→unwrap_or ---

    let mut catch_pipeline = PipelineStart::<u64>::new()
        .stage(
            |x: u64| -> Result<u64, &'static str> { if x > 0 { Ok(x) } else { Err("zero") } },
            r,
        )
        .catch(|_err: &'static str| {}, r)
        .map(|x: u64| x.wrapping_mul(2), r)
        .unwrap_or(0);

    // --- Combinator pipeline setup (uses r) ---

    // Pipeline .cloned(): &u64 → u64
    let input_val = 42u64;
    let mut cloned_pipe = PipelineStart::<&u64>::new().stage(ref_identity, r).cloned();

    // Pipeline .dispatch(): pipeline → handler
    let dispatch_inner = PipelineStart::<u64>::new().stage(sink, r).build();
    let mut dispatch_pipe = PipelineStart::<u64>::new()
        .stage(|x: u64| x.wrapping_mul(3), r)
        .dispatch(dispatch_inner)
        .build();

    // --- Handler dispatch setup (uses world.registry_mut(), r no longer needed) ---

    let mut sys_res = handler_res_read.into_handler(world.registry_mut());
    let mut sys_res_mut = handler_res_mut_write.into_handler(world.registry_mut());
    let mut sys_two = handler_two_res.into_handler(world.registry_mut());
    let mut sys_dyn: Box<dyn Handler<u64>> =
        Box::new(handler_res_read.into_handler(world.registry_mut()));

    // --- Combinator + Adapter setup (uses world.registry_mut()) ---

    // FanOut 2-way
    let fan_h1 = ref_accumulate.into_handler(world.registry_mut());
    let fan_h2 = ref_accumulate.into_handler(world.registry_mut());
    let mut fanout = fan_out!(fan_h1, fan_h2);

    // Broadcast 2-way
    let mut broadcast: Broadcast<u64> = Broadcast::new();
    broadcast.add(ref_accumulate.into_handler(world.registry_mut()));
    broadcast.add(ref_accumulate.into_handler(world.registry_mut()));

    // Cloned adapter: Handler<u64> → Handler<&u64>
    let mut cloned_adapt = Cloned(sink.into_handler(world.registry_mut()));

    // ByRef adapter: Handler<&u64> → Handler<u64>
    let mut byref_adapt = ByRef(ref_accumulate.into_handler(world.registry_mut()));

    // Adapt adapter: decode → Option → handler
    let mut adapt_adapt = Adapt::new(decode_u64, sink.into_handler(world.registry_mut()));

    // --- Pipeline benchmarks ---

    print_header("Pipeline Dispatch Latency (cycles)");

    let mut input = 1u64;

    bench_batched("baseline (hand-written fn)", || {
        input = input.wrapping_add(1);
        baseline_handwritten(&mut world, black_box(input))
    });

    bench_batched("bare 3-stage pipe", || {
        input = input.wrapping_add(1);
        bare_3stage_run(&mut bare, &mut world, black_box(input))
    });

    bench_batched("option 3-stage (Some path)", || {
        input = input.wrapping_add(1);
        option_3stage_run(&mut option, &mut world, black_box(input + 1)).unwrap_or(0)
    });

    bench_batched("option 3-stage (None path)", || {
        option_3stage_run(&mut option, &mut world, black_box(0)).unwrap_or(0)
    });

    bench_batched("world-access 2-stage (Res<T>)", || {
        input = input.wrapping_add(1);
        world_access_run(&mut world_resolved, &mut world, black_box(input))
    });

    bench_batched("boxed Pipeline (dyn dispatch)", || {
        input = input.wrapping_add(1);
        boxed_pipeline_run(&mut boxed, &mut world, black_box(input));
        0
    });

    bench_batched("result→catch→map→unwrap_or", || {
        input = input.wrapping_add(1);
        catch_pipeline.run(&mut world, black_box(input))
    });

    // --- Handler dispatch benchmarks ---

    println!();
    print_header("Handler Dispatch Latency (cycles)");

    bench_batched("Handler + Res<u64> (read)", || {
        input = input.wrapping_add(1);
        probe_handler_res_read(&mut sys_res, &mut world, black_box(input));
        0
    });

    bench_batched("Handler + ResMut<u64> (write+stamp)", || {
        input = input.wrapping_add(1);
        probe_handler_res_mut(&mut sys_res_mut, &mut world, black_box(input));
        0
    });

    bench_batched("Handler + 2x Res (tuple fetch)", || {
        input = input.wrapping_add(1);
        probe_handler_two_res(&mut sys_two, &mut world, black_box(input));
        0
    });

    bench_batched("Box<dyn Handler> + Res<u64>", || {
        input = input.wrapping_add(1);
        probe_dyn_handler(&mut *sys_dyn, &mut world, black_box(input));
        0
    });

    // --- Stage pipeline with Res<T> (3-stage) ---

    println!();
    print_header("Stage Pipeline with Res<T> (cycles)");

    bench_batched("3-stage pipeline (Res<T>)", || {
        input = input.wrapping_add(1);
        stage_3.run(&mut world, black_box(input))
    });

    // --- Combinator + Adapter benchmarks ---

    println!();
    print_header("Combinator + Adapter Latency (cycles)");

    bench_batched("pipeline .cloned() (&u64 → u64)", || {
        probe_cloned_pipeline(&mut cloned_pipe, &mut world, black_box(&input_val))
    });

    bench_batched("pipeline .dispatch() → handler", || {
        input = input.wrapping_add(1);
        probe_dispatch_pipeline(&mut dispatch_pipe, &mut world, black_box(input));
        0
    });

    bench_batched("FanOut 2-way (monomorphized)", || {
        input = input.wrapping_add(1);
        probe_fanout_2way(&mut fanout, &mut world, black_box(input));
        0
    });

    bench_batched("Broadcast 2-way (dyn dispatch)", || {
        input = input.wrapping_add(1);
        probe_broadcast(&mut broadcast, &mut world, black_box(input));
        0
    });

    bench_batched("Cloned adapter (u64 Copy)", || {
        probe_cloned_adapter(&mut cloned_adapt, &mut world, black_box(&input_val));
        0
    });

    bench_batched("ByRef adapter", || {
        input = input.wrapping_add(1);
        probe_byref_adapter(&mut byref_adapt, &mut world, black_box(input));
        0
    });

    bench_batched("Adapt adapter (decode → handler)", || {
        input = input.wrapping_add(1);
        probe_adapt(&mut adapt_adapt, &mut world, black_box(input));
        0
    });

    // --- Batch vs Linear throughput (total cycles for 100 items) ---

    println!();
    print_header("Batch vs Linear Throughput (total cycles, 100 items)");

    let items_100: Vec<u64> = (0..100).collect();

    // Batch bare: fill + run
    {
        for _ in 0..WARMUP {
            batch_bare.input_mut().extend_from_slice(&items_100);
            batch_bare.run(&mut world);
        }
        let mut samples = Vec::with_capacity(ITERATIONS);
        for _ in 0..ITERATIONS {
            batch_bare.input_mut().extend_from_slice(&items_100);
            let start = rdtsc_start();
            batch_bare.run(&mut world);
            let end = rdtsc_end();
            samples.push(end.wrapping_sub(start));
        }
        samples.sort_unstable();
        println!(
            "{:<44} {:>8} {:>8} {:>8}",
            "batch bare (100 items)",
            percentile(&samples, 50.0),
            percentile(&samples, 99.0),
            percentile(&samples, 99.9),
        );
    }

    // Linear bare: 100 individual calls (same chain)
    {
        for _ in 0..WARMUP {
            for i in 0..100u64 {
                linear_bare.run(&mut world, black_box(i));
            }
        }
        let mut samples = Vec::with_capacity(ITERATIONS);
        for _ in 0..ITERATIONS {
            let start = rdtsc_start();
            for i in 0..100u64 {
                linear_bare.run(&mut world, black_box(i));
            }
            let end = rdtsc_end();
            samples.push(end.wrapping_sub(start));
        }
        samples.sort_unstable();
        println!(
            "{:<44} {:>8} {:>8} {:>8}",
            "linear bare (100 calls)",
            percentile(&samples, 50.0),
            percentile(&samples, 99.0),
            percentile(&samples, 99.9),
        );
    }

    // Batch Res<T>: fill + run
    {
        for _ in 0..WARMUP {
            batch_res.input_mut().extend_from_slice(&items_100);
            batch_res.run(&mut world);
        }
        let mut samples = Vec::with_capacity(ITERATIONS);
        for _ in 0..ITERATIONS {
            batch_res.input_mut().extend_from_slice(&items_100);
            let start = rdtsc_start();
            batch_res.run(&mut world);
            let end = rdtsc_end();
            samples.push(end.wrapping_sub(start));
        }
        samples.sort_unstable();
        println!(
            "{:<44} {:>8} {:>8} {:>8}",
            "batch Res<T> (100 items)",
            percentile(&samples, 50.0),
            percentile(&samples, 99.0),
            percentile(&samples, 99.9),
        );
    }

    // Linear Res<T>: 100 individual calls (same chain)
    {
        for _ in 0..WARMUP {
            for i in 0..100u64 {
                linear_res.run(&mut world, black_box(i));
            }
        }
        let mut samples = Vec::with_capacity(ITERATIONS);
        for _ in 0..ITERATIONS {
            let start = rdtsc_start();
            for i in 0..100u64 {
                linear_res.run(&mut world, black_box(i));
            }
            let end = rdtsc_end();
            samples.push(end.wrapping_sub(start));
        }
        samples.sort_unstable();
        println!(
            "{:<44} {:>8} {:>8} {:>8}",
            "linear Res<T> (100 calls)",
            percentile(&samples, 50.0),
            percentile(&samples, 99.0),
            percentile(&samples, 99.9),
        );
    }

    println!();
}
