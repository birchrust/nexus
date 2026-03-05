//! DAG pipeline dispatch latency benchmark.
//!
//! Run asm inspection:
//! ```bash
//! cargo asm -p nexus-rt --example perf_dag perf_dag::probe_dag_linear_3
//! cargo asm -p nexus-rt --example perf_dag perf_dag::probe_dag_diamond
//! cargo asm -p nexus-rt --example perf_dag perf_dag::probe_dag_dyn
//! ```
//!
//! Run benchmark:
//! ```bash
//! taskset -c 0 cargo run --release -p nexus-rt --example perf_dag
//! ```

use std::hint::black_box;

use nexus_rt::dag::{DagBuilder, TypedDagStart};
use nexus_rt::{Handler, PipelineStart, Res, ResMut, WorldBuilder};

// =============================================================================
// Bench infrastructure (inline — no shared utils crate yet)
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

fn bench_batched<F: FnMut()>(name: &str, mut f: F) -> (u64, u64, u64) {
    for _ in 0..WARMUP {
        f();
    }
    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            f();
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
// DAG stage functions — trivial work to isolate dispatch overhead
// =============================================================================

fn root_mul2(x: u32) -> u32 {
    x.wrapping_mul(2)
}

fn add_one(val: &u32) -> u32 {
    val.wrapping_add(1)
}

fn mul3(val: &u32) -> u32 {
    val.wrapping_mul(3)
}

fn sink_store(mut out: ResMut<u32>, val: &u32) {
    *out = *val;
}

fn sink_add(mut out: ResMut<u32>, val: &u32) {
    *out = out.wrapping_add(*val);
}

fn merge2_add(a: &u32, b: &u32) -> u32 {
    a.wrapping_add(*b)
}

fn merge2_add_sink(mut out: ResMut<u32>, a: &u32, b: &u32) {
    *out = a.wrapping_add(*b);
}

fn merge4_add(a: &u32, b: &u32, c: &u32, d: &u32) -> u32 {
    a.wrapping_add(*b).wrapping_add(*c).wrapping_add(*d)
}

// Pipeline comparison stages (by value)
fn pipe_mul2(x: u32) -> u32 {
    x.wrapping_mul(2)
}

fn pipe_add1(x: u32) -> u32 {
    x.wrapping_add(1)
}

fn pipe_sink(mut out: ResMut<u32>, x: u32) {
    *out = x;
}

// World-accessing DAG stage
fn scale_by_res(factor: Res<u32>, val: &u32) -> u32 {
    val.wrapping_mul(*factor)
}

// =============================================================================
// DAG codegen probes
// =============================================================================

/// 3-stage linear DAG dispatch: root → add_one → sink.
/// Expect: ManuallyDrop + root vtable call, 2x vtable calls, drop loop.
#[inline(never)]
pub fn probe_dag_linear_3(p: &mut impl Handler<u32>, world: &mut nexus_rt::World, input: u32) {
    p.run(world, input);
}

/// Diamond DAG dispatch: root → [a, b] → merge → sink (5 stages).
#[inline(never)]
pub fn probe_dag_diamond(p: &mut impl Handler<u32>, world: &mut nexus_rt::World, input: u32) {
    p.run(world, input);
}

/// DAG through dyn Handler — double vtable indirection.
#[inline(never)]
pub fn probe_dag_dyn(p: &mut dyn Handler<u32>, world: &mut nexus_rt::World, input: u32) {
    p.run(world, input);
}

/// Equivalent linear Pipeline (monomorphized, zero vtable).
#[inline(never)]
pub fn probe_pipeline_linear_3(
    p: &mut nexus_rt::PipelineBuilder<u32, (), impl FnMut(&mut nexus_rt::World, u32)>,
    world: &mut nexus_rt::World,
    input: u32,
) {
    p.run(world, input);
}

/// Typed DAG linear 3 — monomorphized, zero vtable.
#[inline(never)]
pub fn probe_typed_dag_linear_3(
    p: &mut impl Handler<u32>,
    world: &mut nexus_rt::World,
    input: u32,
) {
    p.run(world, input);
}

/// Typed DAG diamond — monomorphized, zero vtable.
#[inline(never)]
pub fn probe_typed_dag_diamond(p: &mut impl Handler<u32>, world: &mut nexus_rt::World, input: u32) {
    p.run(world, input);
}

// =============================================================================
// DAG topology builders
// =============================================================================

/// Linear: root → N-2 middle → sink (minimum N=2).
fn build_linear(n: usize, reg: &nexus_rt::Registry) -> nexus_rt::dag::DagPipeline<u32> {
    assert!(n >= 2);
    let mut dag = DagBuilder::<u32>::new();
    let root = dag.root(root_mul2, reg);
    let mut prev = root;
    for _ in 0..n - 2 {
        let s = dag.stage(add_one, reg);
        dag.edge(prev, s);
        prev = s;
    }
    let sink = dag.stage(sink_store, reg);
    dag.edge(prev, sink);
    dag.build()
}

/// Fan-out: root → N sinks.
fn build_fan_out(n: usize, reg: &nexus_rt::Registry) -> nexus_rt::dag::DagPipeline<u32> {
    let mut dag = DagBuilder::<u32>::new();
    let root = dag.root(root_mul2, reg);
    for _ in 0..n {
        let sink = dag.stage(sink_add, reg);
        dag.edge(root, sink);
    }
    dag.build()
}

/// Diamond-2: root → [a, b] → merge → sink.
fn build_diamond_2(reg: &nexus_rt::Registry) -> nexus_rt::dag::DagPipeline<u32> {
    let mut dag = DagBuilder::<u32>::new();
    let root = dag.root(root_mul2, reg);
    let a = dag.stage(add_one, reg);
    let b = dag.stage(mul3, reg);
    dag.edge(root, a);
    dag.edge(root, b);
    let merge = dag.merge2(a, b, merge2_add, reg);
    let sink = dag.stage(sink_store, reg);
    dag.edge(merge, sink);
    dag.build()
}

/// Diamond-4: root → [a, b, c, d] → merge4 → sink.
fn build_diamond_4(reg: &nexus_rt::Registry) -> nexus_rt::dag::DagPipeline<u32> {
    let mut dag = DagBuilder::<u32>::new();
    let root = dag.root(root_mul2, reg);
    let a = dag.stage(add_one, reg);
    let b = dag.stage(mul3, reg);
    let c = dag.stage(add_one, reg);
    let d = dag.stage(mul3, reg);
    dag.edge(root, a);
    dag.edge(root, b);
    dag.edge(root, c);
    dag.edge(root, d);
    let merge = dag.merge4(a, b, c, d, merge4_add, reg);
    let sink = dag.stage(sink_store, reg);
    dag.edge(merge, sink);
    dag.build()
}

/// Complex: root → [a, b]; a → c; b + c → merge → sink.
/// 5 stages, mixed fan-out + linear + merge.
fn build_complex(reg: &nexus_rt::Registry) -> nexus_rt::dag::DagPipeline<u32> {
    let mut dag = DagBuilder::<u32>::new();
    let root = dag.root(root_mul2, reg);
    let a = dag.stage(add_one, reg);
    let b = dag.stage(mul3, reg);
    dag.edge(root, a);
    dag.edge(root, b);
    let c = dag.stage(add_one, reg);
    dag.edge(a, c);
    let _merge = dag.merge2(b, c, merge2_add_sink, reg);
    dag.build()
}

/// Complex with Res<T>: root → scale → [a, b] → merge → sink.
/// Tests Param fetch overhead within DAG stages.
fn build_complex_res(reg: &nexus_rt::Registry) -> nexus_rt::dag::DagPipeline<u32> {
    let mut dag = DagBuilder::<u32>::new();
    let root = dag.root(root_mul2, reg);
    let scaled = dag.stage(scale_by_res, reg);
    dag.edge(root, scaled);
    let a = dag.stage(add_one, reg);
    let b = dag.stage(mul3, reg);
    dag.edge(scaled, a);
    dag.edge(scaled, b);
    let _merge = dag.merge2(a, b, merge2_add_sink, reg);
    dag.build()
}

// =============================================================================
// Typed DAG topology builders (monomorphized)
// =============================================================================

/// Typed DAG linear 3: root → add_one → sink.
fn build_typed_linear_3(
    reg: &nexus_rt::Registry,
) -> nexus_rt::TypedDag<u32, impl FnMut(&mut nexus_rt::World, u32) + Send + use<>> {
    TypedDagStart::<u32>::new()
        .root(root_mul2, reg)
        .then(add_one, reg)
        .then(sink_store, reg)
        .build()
}

/// Typed DAG linear 5: root → add_one × 3 → sink.
fn build_typed_linear_5(
    reg: &nexus_rt::Registry,
) -> nexus_rt::TypedDag<u32, impl FnMut(&mut nexus_rt::World, u32) + Send + use<>> {
    TypedDagStart::<u32>::new()
        .root(root_mul2, reg)
        .then(add_one, reg)
        .then(add_one, reg)
        .then(add_one, reg)
        .then(sink_store, reg)
        .build()
}

/// Typed DAG diamond-2: root → [a, b] → merge → sink.
fn build_typed_diamond_2(
    reg: &nexus_rt::Registry,
) -> nexus_rt::TypedDag<u32, impl FnMut(&mut nexus_rt::World, u32) + Send + use<>> {
    TypedDagStart::<u32>::new()
        .root(root_mul2, reg)
        .fork()
        .arm(|a| a.then(add_one, reg))
        .arm(|b| b.then(mul3, reg))
        .merge(merge2_add, reg)
        .then(sink_store, reg)
        .build()
}

/// Typed DAG fan-out 2: root → [sink, sink].
fn build_typed_fan_out_2(
    reg: &nexus_rt::Registry,
) -> nexus_rt::TypedDag<u32, impl FnMut(&mut nexus_rt::World, u32) + Send + use<>> {
    TypedDagStart::<u32>::new()
        .root(root_mul2, reg)
        .fork()
        .arm(|a| a.then(sink_store, reg))
        .arm(|b| b.then(sink_add, reg))
        .join()
        .build()
}

/// Typed DAG complex: root → [a: add_one → add_one, b: mul3] → merge → sink.
fn build_typed_complex(
    reg: &nexus_rt::Registry,
) -> nexus_rt::TypedDag<u32, impl FnMut(&mut nexus_rt::World, u32) + Send + use<>> {
    TypedDagStart::<u32>::new()
        .root(root_mul2, reg)
        .fork()
        .arm(|a| a.then(add_one, reg).then(add_one, reg))
        .arm(|b| b.then(mul3, reg))
        .merge(merge2_add, reg)
        .then(sink_store, reg)
        .build()
}

/// Typed DAG complex+Res: root → scale → [a, b] → merge → sink.
fn build_typed_complex_res(
    reg: &nexus_rt::Registry,
) -> nexus_rt::TypedDag<u32, impl FnMut(&mut nexus_rt::World, u32) + Send + use<>> {
    TypedDagStart::<u32>::new()
        .root(root_mul2, reg)
        .then(scale_by_res, reg)
        .fork()
        .arm(|a| a.then(add_one, reg))
        .arm(|b| b.then(mul3, reg))
        .merge(merge2_add_sink, reg)
        .build()
}

// =============================================================================
// Main — benchmark
// =============================================================================

fn main() {
    println!("DAG PIPELINE DISPATCH LATENCY BENCHMARK");
    println!("=======================================\n");
    println!("Iterations: {ITERATIONS}, Warmup: {WARMUP}, Batch: {BATCH}");
    println!("All times in CPU cycles\n");

    let mut wb = WorldBuilder::new();
    wb.register::<u32>(0);
    let mut world = wb.build();

    // -- Build all DAGs and pipelines upfront (reg borrows world immutably) --

    let reg = world.registry();

    let mut dag_lin2 = build_linear(2, reg);
    let mut dag_lin3 = build_linear(3, reg);
    let mut dag_lin5 = build_linear(5, reg);
    let mut dag_lin10 = build_linear(10, reg);

    let mut dag_fan2 = build_fan_out(2, reg);
    let mut dag_fan4 = build_fan_out(4, reg);
    let mut dag_fan8 = build_fan_out(8, reg);

    let mut dag_dia2 = build_diamond_2(reg);
    let mut dag_dia4 = build_diamond_4(reg);

    let mut dag_complex = build_complex(reg);
    let mut dag_complex_res = build_complex_res(reg);

    let mut dag_dyn_lin3: Box<dyn Handler<u32>> = Box::new(build_linear(3, reg));
    let mut dag_dyn_dia2: Box<dyn Handler<u32>> = Box::new(build_diamond_2(reg));

    let mut pipe3 = PipelineStart::<u32>::new()
        .stage(pipe_mul2, reg)
        .stage(pipe_add1, reg)
        .stage(pipe_sink, reg);

    let mut pipe5 = PipelineStart::<u32>::new()
        .stage(pipe_mul2, reg)
        .stage(pipe_add1, reg)
        .stage(pipe_add1, reg)
        .stage(pipe_add1, reg)
        .stage(pipe_sink, reg);

    let mut pipe3_boxed = PipelineStart::<u32>::new()
        .stage(pipe_mul2, reg)
        .stage(pipe_add1, reg)
        .stage(pipe_sink, reg)
        .build();

    // -- Typed DAGs --
    let mut tdag_lin3 = build_typed_linear_3(reg);
    let mut tdag_lin5 = build_typed_linear_5(reg);
    let mut tdag_dia2 = build_typed_diamond_2(reg);
    let mut tdag_fan2 = build_typed_fan_out_2(reg);
    let mut tdag_complex = build_typed_complex(reg);
    let mut tdag_complex_res = build_typed_complex_res(reg);

    // Probe DAGs (for cargo-asm, built separately)
    let mut probe_lin3 = build_linear(3, reg);
    let mut probe_dia2 = build_diamond_2(reg);
    let mut probe_dyn: Box<dyn Handler<u32>> = Box::new(build_linear(3, reg));
    let mut probe_pipe = PipelineStart::<u32>::new()
        .stage(pipe_mul2, reg)
        .stage(pipe_add1, reg)
        .stage(pipe_sink, reg);
    let mut probe_tdag_lin3 = build_typed_linear_3(reg);
    let mut probe_tdag_dia2 = build_typed_diamond_2(reg);

    // reg goes dead here — world can be borrowed mutably below.

    // -- Benchmarks --

    let mut input = 1u32;

    print_header("Linear DAG (root → N-2 mid → sink)");

    bench_batched("linear 2 stages", || {
        input = input.wrapping_add(1);
        dag_lin2.run(&mut world, black_box(input));
    });
    bench_batched("linear 3 stages", || {
        input = input.wrapping_add(1);
        dag_lin3.run(&mut world, black_box(input));
    });
    bench_batched("linear 5 stages", || {
        input = input.wrapping_add(1);
        dag_lin5.run(&mut world, black_box(input));
    });
    bench_batched("linear 10 stages", || {
        input = input.wrapping_add(1);
        dag_lin10.run(&mut world, black_box(input));
    });

    println!();
    print_header("Fan-out (root → N sinks)");

    bench_batched("fan-out 2 (3 stages)", || {
        input = input.wrapping_add(1);
        dag_fan2.run(&mut world, black_box(input));
    });
    bench_batched("fan-out 4 (5 stages)", || {
        input = input.wrapping_add(1);
        dag_fan4.run(&mut world, black_box(input));
    });
    bench_batched("fan-out 8 (9 stages)", || {
        input = input.wrapping_add(1);
        dag_fan8.run(&mut world, black_box(input));
    });

    println!();
    print_header("Diamond (root → N middle → merge → sink)");

    bench_batched("diamond fan=2 (5 stages)", || {
        input = input.wrapping_add(1);
        dag_dia2.run(&mut world, black_box(input));
    });
    bench_batched("diamond fan=4 (7 stages)", || {
        input = input.wrapping_add(1);
        dag_dia4.run(&mut world, black_box(input));
    });

    println!();
    print_header("Complex Topologies");

    bench_batched("complex (fan+linear+merge, 5 stg)", || {
        input = input.wrapping_add(1);
        dag_complex.run(&mut world, black_box(input));
    });
    bench_batched("complex+Res<T> (5 stg, Param fetch)", || {
        input = input.wrapping_add(1);
        dag_complex_res.run(&mut world, black_box(input));
    });

    println!();
    print_header("Dyn Dispatch (Box<dyn Handler<u32>>)");

    bench_batched("linear 3 via Box<dyn Handler>", || {
        input = input.wrapping_add(1);
        dag_dyn_lin3.run(&mut world, black_box(input));
    });
    bench_batched("diamond-2 via Box<dyn Handler>", || {
        input = input.wrapping_add(1);
        dag_dyn_dia2.run(&mut world, black_box(input));
    });

    println!();
    print_header("Pipeline Comparison (monomorphized baseline)");

    bench_batched("pipeline 3-stage (monomorphized)", || {
        input = input.wrapping_add(1);
        pipe3.run(&mut world, black_box(input));
    });
    bench_batched("pipeline 5-stage (monomorphized)", || {
        input = input.wrapping_add(1);
        pipe5.run(&mut world, black_box(input));
    });
    bench_batched("pipeline 3-stage (boxed dyn)", || {
        input = input.wrapping_add(1);
        pipe3_boxed.run(&mut world, black_box(input));
    });

    println!();
    print_header("Typed DAG (monomorphized, zero vtable)");

    bench_batched("typed linear 3 stages", || {
        input = input.wrapping_add(1);
        tdag_lin3.run(&mut world, black_box(input));
    });
    bench_batched("typed linear 5 stages", || {
        input = input.wrapping_add(1);
        tdag_lin5.run(&mut world, black_box(input));
    });
    bench_batched("typed diamond fan=2 (5 stages)", || {
        input = input.wrapping_add(1);
        tdag_dia2.run(&mut world, black_box(input));
    });
    bench_batched("typed fan-out 2 (join)", || {
        input = input.wrapping_add(1);
        tdag_fan2.run(&mut world, black_box(input));
    });
    bench_batched("typed complex (fan+linear+merge)", || {
        input = input.wrapping_add(1);
        tdag_complex.run(&mut world, black_box(input));
    });
    bench_batched("typed complex+Res<T> (Param fetch)", || {
        input = input.wrapping_add(1);
        tdag_complex_res.run(&mut world, black_box(input));
    });

    // -- Codegen probes (exercised, not timed) --

    probe_dag_linear_3(&mut probe_lin3, &mut world, black_box(42));
    probe_dag_diamond(&mut probe_dia2, &mut world, black_box(42));
    probe_dag_dyn(&mut *probe_dyn, &mut world, black_box(42));
    probe_pipeline_linear_3(&mut probe_pipe, &mut world, black_box(42));
    probe_typed_dag_linear_3(&mut probe_tdag_lin3, &mut world, black_box(42));
    probe_typed_dag_diamond(&mut probe_tdag_dia2, &mut world, black_box(42));

    println!();
}
