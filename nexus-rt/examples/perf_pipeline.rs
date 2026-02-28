//! Pipeline + System dispatch codegen inspection + latency benchmark.
//!
//! Run asm inspection (pipelines):
//! ```bash
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::bare_3stage_run
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::option_3stage_run
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::world_access_run
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::world_access_resolved_run
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::boxed_pipeline_run
//! ```
//!
//! Run asm inspection (System dispatch):
//! ```bash
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_system_res_read
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_system_res_mut
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_system_two_res
//! cargo asm -p nexus-rt --example perf_pipeline perf_pipeline::probe_dyn_system
//! ```
//!
//! Run benchmark:
//! ```bash
//! taskset -c 0 cargo run --release -p nexus-rt --example perf_pipeline
//! ```

use std::hint::black_box;

use nexus_rt::{IntoSystem, PipelineStart, Res, ResMut, System, WorldBuilder};

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
        let tsc = core::arch::x86_64::__rdtscp(&mut aux as *mut _);
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
    p: &mut nexus_rt::PipelineBuilder<
        u64,
        u64,
        impl FnMut(&mut nexus_rt::World, u64) -> u64,
    >,
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

/// Pipeline that reads World via HashMap (cold-path API).
#[inline(never)]
pub fn world_access_run(
    p: &mut nexus_rt::PipelineBuilder<
        u64,
        u64,
        impl FnMut(&mut nexus_rt::World, u64) -> u64,
    >,
    world: &mut nexus_rt::World,
    input: u64,
) -> u64 {
    p.run(world, input)
}

/// Pipeline that reads World via pre-resolved ResourceId (hot-path pattern).
#[inline(never)]
pub fn world_access_resolved_run(
    p: &mut nexus_rt::PipelineBuilder<
        u64,
        u64,
        impl FnMut(&mut nexus_rt::World, u64) -> u64,
    >,
    world: &mut nexus_rt::World,
    input: u64,
) -> u64 {
    p.run(world, input)
}

/// Built Pipeline<u64> through dyn dispatch.
#[inline(never)]
pub fn boxed_pipeline_run(
    p: &mut nexus_rt::Pipeline<u64>,
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
// System dispatch probes — SystemParam fetch hot path
// =============================================================================

// Named functions required for IntoSystem (closures don't work with HRTB).

fn system_res_read(counter: Res<u64>, input: u64) {
    black_box((*counter).wrapping_add(input));
}

fn system_res_mut_write(mut counter: ResMut<u64>, input: u64) {
    *counter = (*counter).wrapping_add(input);
}

fn system_two_res(a: Res<u64>, b: Res<u32>, input: u64) {
    black_box((*a).wrapping_add(input).wrapping_add(*b as u64));
}

/// Monomorphized System dispatch with Res<u64>.
/// Full path: System::run → SystemParam::fetch → World::get_ptr + changed_at + current_tick.
#[inline(never)]
pub fn probe_system_res_read(
    sys: &mut impl System<u64>,
    world: &mut nexus_rt::World,
    input: u64,
) {
    sys.run(world, input);
}

/// Monomorphized System dispatch with ResMut<u64>.
/// Full path: fetch + DerefMut stamps changed_at on write.
#[inline(never)]
pub fn probe_system_res_mut(
    sys: &mut impl System<u64>,
    world: &mut nexus_rt::World,
    input: u64,
) {
    sys.run(world, input);
}

/// Monomorphized System dispatch with two Res params (tuple fetch).
#[inline(never)]
pub fn probe_system_two_res(
    sys: &mut impl System<u64>,
    world: &mut nexus_rt::World,
    input: u64,
) {
    sys.run(world, input);
}

/// Dyn-dispatched System — vtable call + SystemParam fetch.
#[inline(never)]
pub fn probe_dyn_system(
    sys: &mut dyn System<u64>,
    world: &mut nexus_rt::World,
    input: u64,
) {
    sys.run(world, input);
}

// =============================================================================
// Main — benchmark
// =============================================================================

fn main() {
    let mut wb = WorldBuilder::new();
    wb.register::<u64>(42);
    wb.register::<u32>(7);
    let mut world = wb.build();

    // --- Bare 3-stage pipeline (no Option, no World access) ---

    let mut bare = PipelineStart::<u64>::new()
        .pipe(|_w, x| x.wrapping_mul(3))
        .pipe(|_w, x| x.wrapping_add(7))
        .pipe(|_w, x| x >> 1);

    // --- Option 3-stage pipeline ---

    let mut option = PipelineStart::<u64>::new()
        .pipe(|_w, x| if x > 0 { Some(x) } else { None })
        .map(|_w, x| x.wrapping_mul(3))
        .filter(|_w, x| *x < 1_000_000);

    // --- World-accessing pipeline (HashMap lookup per stage) ---

    let mut world_access = PipelineStart::<u64>::new()
        .pipe(|w, x| x.wrapping_add(*w.resource::<u64>()))
        .pipe(|w, x| x.wrapping_mul(*w.resource::<u64>()));

    // --- World-accessing pipeline (pre-resolved ResourceId) ---

    let res_id = world.id::<u64>();
    let mut world_resolved = PipelineStart::<u64>::new()
        .pipe(move |w, x| {
            // SAFETY: id from same registry, single-threaded dispatch.
            let val = unsafe { w.get::<u64>(res_id) };
            x.wrapping_add(*val)
        })
        .pipe(move |w, x| {
            // SAFETY: id from same registry, single-threaded dispatch.
            let val = unsafe { w.get::<u64>(res_id) };
            x.wrapping_mul(*val)
        });

    // --- Built (boxed) pipeline ---

    let mut boxed = PipelineStart::<u64>::new()
        .pipe(|_w, x| x.wrapping_mul(3))
        .pipe(|_w, x| x.wrapping_add(7))
        .pipe(|_w, x| {
            let _ = x;
        })
        .build();

    // --- Option pipeline with catch ---

    let mut catch_pipeline = PipelineStart::<u64>::new()
        .pipe(|_w, x| -> Result<u64, &str> {
            if x > 0 {
                Ok(x)
            } else {
                Err("zero")
            }
        })
        .catch(|_w, _err| {})
        .map(|_w, x| x.wrapping_mul(2))
        .unwrap_or(0);

    // --- System dispatch setup ---

    let mut sys_res = system_res_read.into_system(world.registry());
    let mut sys_res_mut = system_res_mut_write.into_system(world.registry());
    let mut sys_two = system_two_res.into_system(world.registry());
    let mut sys_dyn: Box<dyn System<u64>> =
        Box::new(system_res_read.into_system(world.registry()));

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

    bench_batched("world-access 2-stage (HashMap)", || {
        input = input.wrapping_add(1);
        world_access_run(&mut world_access, &mut world, black_box(input))
    });

    bench_batched("world-access 2-stage (pre-resolved)", || {
        input = input.wrapping_add(1);
        world_access_resolved_run(&mut world_resolved, &mut world, black_box(input))
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

    // --- System dispatch benchmarks ---

    println!();
    print_header("System Dispatch Latency (cycles)");

    bench_batched("System + Res<u64> (read)", || {
        input = input.wrapping_add(1);
        probe_system_res_read(&mut sys_res, &mut world, black_box(input));
        0
    });

    bench_batched("System + ResMut<u64> (write+stamp)", || {
        input = input.wrapping_add(1);
        probe_system_res_mut(&mut sys_res_mut, &mut world, black_box(input));
        0
    });

    bench_batched("System + 2x Res (tuple fetch)", || {
        input = input.wrapping_add(1);
        probe_system_two_res(&mut sys_two, &mut world, black_box(input));
        0
    });

    bench_batched("Box<dyn System> + Res<u64>", || {
        input = input.wrapping_add(1);
        probe_dyn_system(&mut *sys_dyn, &mut world, black_box(input));
        0
    });

    println!();
}
