//! RcSlot benchmarks comparing against std::rc::Rc.
//!
//! Run with: cargo run --example bench_rc --release
//!
//! RcSlot is mechanically identical to std::rc::Rc — same Cell<u32> refcounts,
//! same clone/drop semantics. The difference is allocation strategy:
//!
//! - std::rc::Rc: Each allocation goes through the system allocator
//! - RcSlot: Allocations come from a pre-allocated SLUB-style slab via freelist
//!
//! This means RcSlot wins on allocation/deallocation (freelist pop/push vs malloc/free)
//! but refcount operations (clone, drop-not-last, deref) are essentially identical.
//!
//! Measures:
//! - Clone (hot path: increment strong count)
//! - Drop (hot path: decrement strong count, not last ref)
//! - Drop last (cold path: decrement to zero, dealloc)
//! - Deref (pointer chase)
//! - Downgrade + Upgrade cycle

use std::hint::black_box;
use std::rc::Rc;

// -----------------------------------------------------------------------------
// Test type
// -----------------------------------------------------------------------------

#[derive(Clone)]
pub struct Order {
    pub id: u64,
    pub price: f64,
    pub quantity: u64,
    pub flags: u64,
}

impl Order {
    fn new(id: u64) -> Self {
        Order {
            id,
            price: 100.0,
            quantity: 10,
            flags: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// RcSlot allocator
// -----------------------------------------------------------------------------

mod order_alloc {
    nexus_slab::bounded_rc_allocator!(super::Order);
}

fn init_allocator() {
    let _ = order_alloc::Allocator::builder().capacity(8192).build();
}

// -----------------------------------------------------------------------------
// Timing
// -----------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
fn rdtsc_start() -> u64 {
    unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    }
}

#[cfg(target_arch = "x86_64")]
fn rdtsc_end() -> u64 {
    unsafe {
        let tsc = core::arch::x86_64::__rdtscp(&mut 0u32 as *mut _);
        core::arch::x86_64::_mm_lfence();
        tsc
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn rdtsc_start() -> u64 {
    std::time::Instant::now().elapsed().as_nanos() as u64
}

#[cfg(not(target_arch = "x86_64"))]
fn rdtsc_end() -> u64 {
    std::time::Instant::now().elapsed().as_nanos() as u64
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

// -----------------------------------------------------------------------------
// Benchmarks
// -----------------------------------------------------------------------------

const WARMUP: usize = 1000;
const ITERATIONS: usize = 10000;

fn bench_clone_rcslot() -> Vec<u64> {
    init_allocator();

    let rc = order_alloc::RcSlot::new(Order::new(1));
    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for _ in 0..WARMUP {
        let cloned = black_box(&rc).clone();
        drop(black_box(cloned));
    }

    // Measure
    for _ in 0..ITERATIONS {
        let start = rdtsc_start();
        let cloned = black_box(&rc).clone();
        let end = rdtsc_end();
        drop(black_box(cloned));
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

fn bench_clone_std_rc() -> Vec<u64> {
    let rc = Rc::new(Order::new(1));
    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for _ in 0..WARMUP {
        let cloned = black_box(&rc).clone();
        drop(black_box(cloned));
    }

    // Measure
    for _ in 0..ITERATIONS {
        let start = rdtsc_start();
        let cloned = black_box(&rc).clone();
        let end = rdtsc_end();
        drop(black_box(cloned));
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

fn bench_drop_not_last_rcslot() -> Vec<u64> {
    init_allocator();

    let rc = order_alloc::RcSlot::new(Order::new(1));
    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for _ in 0..WARMUP {
        let cloned = rc.clone();
        let start = rdtsc_start();
        drop(black_box(cloned));
        let end = rdtsc_end();
        black_box(end - start);
    }

    // Measure
    for _ in 0..ITERATIONS {
        let cloned = rc.clone();
        let start = rdtsc_start();
        drop(black_box(cloned));
        let end = rdtsc_end();
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

fn bench_drop_not_last_std_rc() -> Vec<u64> {
    let rc = Rc::new(Order::new(1));
    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for _ in 0..WARMUP {
        let cloned = rc.clone();
        let start = rdtsc_start();
        drop(black_box(cloned));
        let end = rdtsc_end();
        black_box(end - start);
    }

    // Measure
    for _ in 0..ITERATIONS {
        let cloned = rc.clone();
        let start = rdtsc_start();
        drop(black_box(cloned));
        let end = rdtsc_end();
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

fn bench_drop_last_rcslot() -> Vec<u64> {
    init_allocator();

    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for i in 0..WARMUP {
        let rc = order_alloc::RcSlot::new(Order::new(i as u64));
        let start = rdtsc_start();
        drop(black_box(rc));
        let end = rdtsc_end();
        black_box(end - start);
    }

    // Measure
    for i in 0..ITERATIONS {
        let rc = order_alloc::RcSlot::new(Order::new(i as u64));
        let start = rdtsc_start();
        drop(black_box(rc));
        let end = rdtsc_end();
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

fn bench_drop_last_std_rc() -> Vec<u64> {
    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for i in 0..WARMUP {
        let rc = Rc::new(Order::new(i as u64));
        let start = rdtsc_start();
        drop(black_box(rc));
        let end = rdtsc_end();
        black_box(end - start);
    }

    // Measure
    for i in 0..ITERATIONS {
        let rc = Rc::new(Order::new(i as u64));
        let start = rdtsc_start();
        drop(black_box(rc));
        let end = rdtsc_end();
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

fn bench_deref_rcslot() -> Vec<u64> {
    init_allocator();

    let rc = order_alloc::RcSlot::new(Order::new(42));
    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for _ in 0..WARMUP {
        let id = black_box(&rc).id;
        black_box(id);
    }

    // Measure
    for _ in 0..ITERATIONS {
        let start = rdtsc_start();
        let id = black_box(&rc).id;
        let end = rdtsc_end();
        black_box(id);
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

fn bench_deref_std_rc() -> Vec<u64> {
    let rc = Rc::new(Order::new(42));
    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for _ in 0..WARMUP {
        let id = black_box(&rc).id;
        black_box(id);
    }

    // Measure
    for _ in 0..ITERATIONS {
        let start = rdtsc_start();
        let id = black_box(&rc).id;
        let end = rdtsc_end();
        black_box(id);
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

fn bench_downgrade_upgrade_rcslot() -> Vec<u64> {
    init_allocator();

    let rc = order_alloc::RcSlot::new(Order::new(1));
    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for _ in 0..WARMUP {
        let weak = black_box(&rc).downgrade();
        let upgraded = weak.upgrade().unwrap();
        drop(black_box(upgraded));
    }

    // Measure
    for _ in 0..ITERATIONS {
        let start = rdtsc_start();
        let weak = black_box(&rc).downgrade();
        let upgraded = weak.upgrade().unwrap();
        let end = rdtsc_end();
        drop(black_box(upgraded));
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

fn bench_downgrade_upgrade_std_rc() -> Vec<u64> {
    let rc = Rc::new(Order::new(1));
    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for _ in 0..WARMUP {
        let weak = Rc::downgrade(black_box(&rc));
        let upgraded = weak.upgrade().unwrap();
        drop(black_box(upgraded));
    }

    // Measure
    for _ in 0..ITERATIONS {
        let start = rdtsc_start();
        let weak = Rc::downgrade(black_box(&rc));
        let upgraded = weak.upgrade().unwrap();
        let end = rdtsc_end();
        drop(black_box(upgraded));
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

fn bench_new_rcslot() -> Vec<u64> {
    init_allocator();

    // Pre-warm the freelist by allocating and freeing
    for i in 0..WARMUP {
        let rc = order_alloc::RcSlot::new(Order::new(i as u64));
        drop(rc);
    }

    let mut samples = Vec::with_capacity(ITERATIONS);

    // Measure allocation from warm freelist
    for i in 0..ITERATIONS {
        let start = rdtsc_start();
        let rc = order_alloc::RcSlot::new(Order::new(i as u64));
        let end = rdtsc_end();
        samples.push(end - start);
        drop(black_box(rc)); // Return to freelist for next iteration
    }

    samples.sort_unstable();
    samples
}

fn bench_new_std_rc() -> Vec<u64> {
    let mut samples = Vec::with_capacity(ITERATIONS);

    // Warmup
    for i in 0..WARMUP {
        let rc = Rc::new(Order::new(i as u64));
        black_box(rc);
    }

    // Measure
    for i in 0..ITERATIONS {
        let start = rdtsc_start();
        let rc = Rc::new(Order::new(i as u64));
        let end = rdtsc_end();
        black_box(rc);
        samples.push(end - start);
    }

    samples.sort_unstable();
    samples
}

// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

fn print_comparison(name: &str, rcslot: &[u64], std_rc: &[u64]) {
    let rc_p50 = percentile(rcslot, 50.0);
    let rc_p99 = percentile(rcslot, 99.0);
    let rc_p999 = percentile(rcslot, 99.9);

    let std_p50 = percentile(std_rc, 50.0);
    let std_p99 = percentile(std_rc, 99.0);
    let std_p999 = percentile(std_rc, 99.9);

    let speedup_p50 = std_p50 as f64 / rc_p50 as f64;

    println!("{:<25} {:>8} {:>8} {:>8}    {:>8} {:>8} {:>8}    {:>5.2}x",
        name,
        rc_p50, rc_p99, rc_p999,
        std_p50, std_p99, std_p999,
        speedup_p50
    );
}

fn main() {
    println!("RcSlot vs std::rc::Rc Benchmark");
    println!("================================");
    println!("Type: Order (32 bytes)");
    println!("Iterations: {}", ITERATIONS);
    println!();
    println!("{:<25} {:>8} {:>8} {:>8}    {:>8} {:>8} {:>8}    {:>6}",
        "Operation", "p50", "p99", "p999", "p50", "p99", "p999", "Speedup");
    println!("{:<25} {:>8} {:>8} {:>8}    {:>8} {:>8} {:>8}    {:>6}",
        "", "RcSlot", "", "", "std::Rc", "", "", "(p50)");
    println!("{}", "-".repeat(95));

    let rcslot = bench_new_rcslot();
    let std_rc = bench_new_std_rc();
    print_comparison("new (from freelist)", &rcslot, &std_rc);

    let rcslot = bench_clone_rcslot();
    let std_rc = bench_clone_std_rc();
    print_comparison("clone", &rcslot, &std_rc);

    let rcslot = bench_drop_not_last_rcslot();
    let std_rc = bench_drop_not_last_std_rc();
    print_comparison("drop (not last)", &rcslot, &std_rc);

    let rcslot = bench_drop_last_rcslot();
    let std_rc = bench_drop_last_std_rc();
    print_comparison("drop (last) + dealloc", &rcslot, &std_rc);

    let rcslot = bench_deref_rcslot();
    let std_rc = bench_deref_std_rc();
    print_comparison("deref", &rcslot, &std_rc);

    let rcslot = bench_downgrade_upgrade_rcslot();
    let std_rc = bench_downgrade_upgrade_std_rc();
    print_comparison("downgrade + upgrade", &rcslot, &std_rc);

    println!();
    println!("All times in CPU cycles.");
}
