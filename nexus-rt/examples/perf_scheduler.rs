//! Scheduler dispatch latency benchmark.
//!
//! Measures the cost of `SystemScheduler::run()` with various DAG
//! topologies and system counts. All systems do trivial work (single
//! wrapping_add) to isolate scheduler overhead from system body cost.
//!
//! Run with:
//! ```bash
//! taskset -c 0 cargo run --release -p nexus-rt --example perf_scheduler
//! ```

use std::hint::black_box;

use nexus_rt::scheduler::SchedulerInstaller;
use nexus_rt::{ResMut, WorldBuilder, new_resource};

new_resource!(ResU64(u64));

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
// Trivial systems — isolate scheduler overhead from system body cost
// =============================================================================

fn sys_true(mut val: ResMut<ResU64>) -> bool {
    val.0 = val.0.wrapping_add(1);
    true
}

fn sys_false(mut val: ResMut<ResU64>) -> bool {
    val.0 = val.0.wrapping_add(1);
    false
}

// =============================================================================
// DAG builders
// =============================================================================

/// N independent roots (no edges). All always run.
fn build_flat(n: usize) -> (WorldBuilder, SchedulerInstaller) {
    let mut wb = WorldBuilder::new();
    wb.register(ResU64(0));
    let mut installer = SchedulerInstaller::new();
    for _ in 0..n {
        installer.add(sys_true, wb.registry());
    }
    (wb, installer)
}

/// Linear chain: A → B → C → ... All propagate true.
fn build_chain(n: usize) -> (WorldBuilder, SchedulerInstaller) {
    let mut wb = WorldBuilder::new();
    wb.register(ResU64(0));
    let mut installer = SchedulerInstaller::new();
    let mut prev = installer.add(sys_true, wb.registry());
    for _ in 1..n {
        let cur = installer.add(sys_true, wb.registry());
        installer.after(cur, prev);
        prev = cur;
    }
    (wb, installer)
}

/// Diamond fan-out/fan-in: root → N middle → sink.
/// Total systems = N + 2.
fn build_diamond(fan: usize) -> (WorldBuilder, SchedulerInstaller) {
    let mut wb = WorldBuilder::new();
    wb.register(ResU64(0));
    let mut installer = SchedulerInstaller::new();
    let root = installer.add(sys_true, wb.registry());
    let mut middles = Vec::with_capacity(fan);
    for _ in 0..fan {
        let m = installer.add(sys_true, wb.registry());
        installer.after(m, root);
        middles.push(m);
    }
    let sink = installer.add(sys_true, wb.registry());
    for &m in &middles {
        installer.after(sink, m);
    }
    (wb, installer)
}

/// Chain where root returns false — downstream skipped.
fn build_chain_skipped(n: usize) -> (WorldBuilder, SchedulerInstaller) {
    let mut wb = WorldBuilder::new();
    wb.register(ResU64(0));
    let mut installer = SchedulerInstaller::new();
    let mut prev = installer.add(sys_false, wb.registry());
    for _ in 1..n {
        let cur = installer.add(sys_true, wb.registry());
        installer.after(cur, prev);
        prev = cur;
    }
    (wb, installer)
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    println!("SCHEDULER DISPATCH LATENCY BENCHMARK");
    println!("====================================\n");
    println!("Iterations: {ITERATIONS}, Warmup: {WARMUP}, Batch: {BATCH}");
    println!("All times in CPU cycles\n");

    // -- Flat (independent roots) --

    print_header("Flat DAG (independent roots, all run)");

    for n in [1, 4, 8, 16, 32] {
        let (mut wb, installer) = build_flat(n);
        let mut scheduler = wb.install_driver(installer);
        let mut world = wb.build();
        bench_batched(&format!("flat {n} systems"), || {
            black_box(scheduler.run(&mut world));
        });
    }

    // -- Linear chain (all propagate) --

    println!();
    print_header("Linear Chain (all propagate true)");

    for n in [1, 4, 8, 16, 32] {
        let (mut wb, installer) = build_chain(n);
        let mut scheduler = wb.install_driver(installer);
        let mut world = wb.build();
        bench_batched(&format!("chain {n} systems"), || {
            black_box(scheduler.run(&mut world));
        });
    }

    // -- Diamond (fan-out/fan-in) --

    println!();
    print_header("Diamond (root → N middle → sink)");

    for fan in [2, 4, 8, 16] {
        let (mut wb, installer) = build_diamond(fan);
        let total = fan + 2;
        let mut scheduler = wb.install_driver(installer);
        let mut world = wb.build();
        bench_batched(&format!("diamond fan={fan} ({total} systems)"), || {
            black_box(scheduler.run(&mut world));
        });
    }

    // -- Skipped chain (root returns false) --

    println!();
    print_header("Skipped Chain (root=false, downstream skipped)");

    for n in [4, 8, 16, 32] {
        let (mut wb, installer) = build_chain_skipped(n);
        let mut scheduler = wb.install_driver(installer);
        let mut world = wb.build();
        bench_batched(
            &format!("skipped chain {n} (1 runs, {} skip)", n - 1),
            || {
                black_box(scheduler.run(&mut world));
            },
        );
    }

    println!();
}
