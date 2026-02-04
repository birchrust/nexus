//! Isolated benchmark: Box vs Slot across sizes and percentiles.
//!
//! Runs EITHER Box or Slot, never both in the same process. This avoids
//! cache and allocator state pollution between the two.
//!
//! Usage:
//!   cargo build --release --example bench_isolated
//!   taskset -c 0 ./target/release/examples/bench_isolated box
//!   taskset -c 0 ./target/release/examples/bench_isolated slot

use std::hint::black_box;

// ============================================================================
// Pod types
// ============================================================================

macro_rules! define_pod {
    ($name:ident, $size:expr) => {
        #[derive(Clone)]
        #[repr(C)]
        pub struct $name {
            pub data: [u8; $size],
        }
        impl Default for $name {
            fn default() -> Self {
                Self { data: [0; $size] }
            }
        }
    };
}

define_pod!(Pod32, 32);
define_pod!(Pod64, 64);
define_pod!(Pod128, 128);
define_pod!(Pod256, 256);
define_pod!(Pod512, 512);
define_pod!(Pod1024, 1024);
define_pod!(Pod4096, 4096);

// ============================================================================
// Macro allocators (only touched in slot mode)
// ============================================================================

mod alloc_32 {
    use super::Pod32;
    nexus_slab::bounded_allocator!(Pod32);
}
mod alloc_64 {
    use super::Pod64;
    nexus_slab::bounded_allocator!(Pod64);
}
mod alloc_128 {
    use super::Pod128;
    nexus_slab::bounded_allocator!(Pod128);
}
mod alloc_256 {
    use super::Pod256;
    nexus_slab::bounded_allocator!(Pod256);
}
mod alloc_512 {
    use super::Pod512;
    nexus_slab::bounded_allocator!(Pod512);
}
mod alloc_1024 {
    use super::Pod1024;
    nexus_slab::bounded_allocator!(Pod1024);
}
mod alloc_4096 {
    use super::Pod4096;
    nexus_slab::bounded_allocator!(Pod4096);
}

// ============================================================================
// Timing
// ============================================================================

#[inline(always)]
fn rdtsc_start() -> u64 {
    unsafe {
        std::arch::x86_64::_mm_lfence();
        std::arch::x86_64::_rdtsc()
    }
}

#[inline(always)]
fn rdtsc_end() -> u64 {
    unsafe {
        let tsc = std::arch::x86_64::__rdtscp(&mut 0u32 as *mut _);
        std::arch::x86_64::_mm_lfence();
        tsc
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn print_row(label: &str, samples: &mut [u64]) {
    samples.sort_unstable();
    println!(
        "  {:<14} {:>5} {:>5} {:>5} {:>6} {:>7} {:>7}",
        label,
        percentile(samples, 50.0),
        percentile(samples, 90.0),
        percentile(samples, 99.0),
        percentile(samples, 99.9),
        percentile(samples, 99.99),
        samples[samples.len() - 1],
    );
}

fn print_header() {
    println!(
        "  {:<14} {:>5} {:>5} {:>5} {:>6} {:>7} {:>7}",
        "", "p50", "p90", "p99", "p99.9", "p99.99", "max"
    );
}

const SAMPLES: usize = 50_000;
const WARMUP: usize = 5_000;

// ============================================================================
// Unroll
// ============================================================================

macro_rules! unroll_10 {
    ($op:expr) => { $op; $op; $op; $op; $op; $op; $op; $op; $op; $op; };
}

macro_rules! unroll_100 {
    ($op:expr) => {
        unroll_10!($op); unroll_10!($op); unroll_10!($op); unroll_10!($op); unroll_10!($op);
        unroll_10!($op); unroll_10!($op); unroll_10!($op); unroll_10!($op); unroll_10!($op);
    };
}

// ============================================================================
// Box benchmarks
// ============================================================================

macro_rules! box_churn {
    ($name:expr, $pod:ty) => {{
        let val = <$pod>::default();

        for _ in 0..WARMUP {
            let b = Box::new(val.clone());
            black_box(b.data[0]);
            drop(b);
        }

        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = rdtsc_start();
            unroll_100!({
                let b = black_box(Box::new(val.clone()));
                black_box(b.data[0]);
                drop(b);
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_row($name, &mut samples);
    }};
}

macro_rules! box_batch_alloc {
    ($name:expr, $pod:ty) => {{
        let val = <$pod>::default();

        for _ in 0..WARMUP / 10 {
            let temp: Vec<Box<$pod>> = (0..100).map(|_| Box::new(val.clone())).collect();
            drop(temp);
        }

        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let mut temp: Vec<Box<$pod>> = Vec::with_capacity(100);
            let start = rdtsc_start();
            unroll_100!({
                temp.push(black_box(Box::new(val.clone())));
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
            drop(temp);
        }
        print_row($name, &mut samples);
    }};
}

macro_rules! box_batch_drop {
    ($name:expr, $pod:ty) => {{
        let val = <$pod>::default();

        for _ in 0..WARMUP / 10 {
            let temp: Vec<Box<$pod>> = (0..100).map(|_| Box::new(val.clone())).collect();
            drop(temp);
        }

        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let boxes: Vec<Box<$pod>> = (0..100).map(|_| Box::new(val.clone())).collect();
            let mut iter = boxes.into_iter();
            let start = rdtsc_start();
            unroll_100!({
                drop(black_box(iter.next()));
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_row($name, &mut samples);
    }};
}

macro_rules! box_access {
    ($name:expr, $pod:ty) => {{
        let val = <$pod>::default();
        let pool: Vec<Box<$pod>> = (0..1000).map(|_| Box::new(val.clone())).collect();

        // Warmup: touch every element
        for p in &pool {
            black_box(p.data[0]);
        }

        let mut rng = 67890u64;
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let base = (rng as usize) % 900;
            let mut idx = base;
            let mut sum = 0u8;
            let start = rdtsc_start();
            unroll_100!({
                sum = sum.wrapping_add(pool[idx % 1000].data[0]);
                idx += 1;
            });
            let end = rdtsc_end();
            black_box(sum);
            samples.push((end - start) / 100);
        }
        print_row($name, &mut samples);
        drop(pool);
    }};
}

// ============================================================================
// Slot benchmarks
// ============================================================================

macro_rules! slot_churn {
    ($name:expr, $pod:ty, $alloc:ident) => {{
        let val = <$pod>::default();

        for _ in 0..WARMUP {
            let s = $alloc::Slot::new(val.clone());
            black_box(s.data[0]);
            drop(s);
        }

        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = rdtsc_start();
            unroll_100!({
                let s = black_box($alloc::Slot::new(val.clone()));
                black_box(s.data[0]);
                drop(s);
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_row($name, &mut samples);
    }};
}

macro_rules! slot_batch_alloc {
    ($name:expr, $pod:ty, $alloc:ident) => {{
        let val = <$pod>::default();

        for _ in 0..WARMUP / 10 {
            let temp: Vec<$alloc::Slot> = (0..100).map(|_| $alloc::Slot::new(val.clone())).collect();
            drop(temp);
        }

        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let mut temp: Vec<$alloc::Slot> = Vec::with_capacity(100);
            let start = rdtsc_start();
            unroll_100!({
                temp.push(black_box($alloc::Slot::new(val.clone())));
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
            drop(temp);
        }
        print_row($name, &mut samples);
    }};
}

macro_rules! slot_batch_drop {
    ($name:expr, $pod:ty, $alloc:ident) => {{
        let val = <$pod>::default();

        for _ in 0..WARMUP / 10 {
            let temp: Vec<$alloc::Slot> = (0..100).map(|_| $alloc::Slot::new(val.clone())).collect();
            drop(temp);
        }

        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let slots: Vec<$alloc::Slot> =
                (0..100).map(|_| $alloc::Slot::new(val.clone())).collect();
            let mut iter = slots.into_iter();
            let start = rdtsc_start();
            unroll_100!({
                drop(black_box(iter.next()));
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_row($name, &mut samples);
    }};
}

macro_rules! slot_access {
    ($name:expr, $pod:ty, $alloc:ident) => {{
        let val = <$pod>::default();
        let pool: Vec<$alloc::Slot> = (0..1000).map(|_| $alloc::Slot::new(val.clone())).collect();

        // Warmup: touch every element
        for p in &pool {
            black_box(p.data[0]);
        }

        let mut rng = 67890u64;
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let base = (rng as usize) % 900;
            let mut idx = base;
            let mut sum = 0u8;
            let start = rdtsc_start();
            unroll_100!({
                sum = sum.wrapping_add(pool[idx % 1000].data[0]);
                idx += 1;
            });
            let end = rdtsc_end();
            black_box(sum);
            samples.push((end - start) / 100);
        }
        print_row($name, &mut samples);
        drop(pool);
    }};
}

// ============================================================================
// Cold benchmarks (cache-evicted between every single measurement)
// ============================================================================

const COLD_SAMPLES: usize = 10_000;
const POLLUTER_SIZE: usize = 8 * 1024 * 1024; // 8MB > most L3 caches

#[inline(never)]
fn evict_cache(polluter: &[u8]) {
    let ptr = polluter.as_ptr();
    let len = polluter.len();
    for i in (0..len).step_by(64) {
        unsafe { std::ptr::read_volatile(ptr.add(i)); }
    }
    unsafe { std::arch::x86_64::_mm_lfence(); }
}

macro_rules! box_cold_churn {
    ($name:expr, $pod:ty) => {{
        let val = <$pod>::default();
        let polluter = vec![0u8; POLLUTER_SIZE];

        // Warmup
        for _ in 0..100 {
            evict_cache(&polluter);
            let b = Box::new(val.clone());
            black_box(b.data[0]);
            drop(b);
        }

        let mut samples = Vec::with_capacity(COLD_SAMPLES);
        for _ in 0..COLD_SAMPLES {
            evict_cache(&polluter);

            let start = rdtsc_start();
            let b = black_box(Box::new(val.clone()));
            black_box(b.data[0]);
            drop(b);
            let end = rdtsc_end();
            samples.push(end - start);
        }
        print_row($name, &mut samples);
    }};
}

macro_rules! slot_cold_churn {
    ($name:expr, $pod:ty, $alloc:ident) => {{
        let val = <$pod>::default();
        let polluter = vec![0u8; POLLUTER_SIZE];

        // Warmup
        for _ in 0..100 {
            evict_cache(&polluter);
            let s = $alloc::Slot::new(val.clone());
            black_box(s.data[0]);
            drop(s);
        }

        let mut samples = Vec::with_capacity(COLD_SAMPLES);
        for _ in 0..COLD_SAMPLES {
            evict_cache(&polluter);

            let start = rdtsc_start();
            let s = black_box($alloc::Slot::new(val.clone()));
            black_box(s.data[0]);
            drop(s);
            let end = rdtsc_end();
            samples.push(end - start);
        }
        print_row($name, &mut samples);
    }};
}

// ============================================================================
// Runners
// ============================================================================

macro_rules! run_all_sizes_box {
    ($bench_macro:ident, $label:expr) => {{
        println!("\n{}", $label);
        print_header();
        $bench_macro!("32B", Pod32);
        $bench_macro!("64B", Pod64);
        $bench_macro!("128B", Pod128);
        $bench_macro!("256B", Pod256);
        $bench_macro!("512B", Pod512);
        $bench_macro!("1024B", Pod1024);
        $bench_macro!("4096B", Pod4096);
    }};
}

macro_rules! run_all_sizes_slot {
    ($bench_macro:ident, $label:expr) => {{
        println!("\n{}", $label);
        print_header();
        $bench_macro!("32B", Pod32, alloc_32);
        $bench_macro!("64B", Pod64, alloc_64);
        $bench_macro!("128B", Pod128, alloc_128);
        $bench_macro!("256B", Pod256, alloc_256);
        $bench_macro!("512B", Pod512, alloc_512);
        $bench_macro!("1024B", Pod1024, alloc_1024);
        $bench_macro!("4096B", Pod4096, alloc_4096);
    }};
}

fn run_box() {
    println!("TARGET: Box (heap allocation via malloc)");
    println!("{}", "=".repeat(66));

    run_all_sizes_box!(box_churn, "CHURN (alloc + deref + drop, LIFO single-slot)");
    run_all_sizes_box!(box_batch_alloc, "BATCH ALLOC (100 sequential, no interleaved frees)");
    run_all_sizes_box!(box_batch_drop, "BATCH DROP (pre-alloc 100, then free all)");
    run_all_sizes_box!(box_access, "ACCESS (random deref from pool of 1000)");
    run_all_sizes_box!(box_cold_churn, "COLD CHURN (cache-evicted between each op, single alloc+deref+drop)");
}

fn run_slot() {
    let cap = 20_000;
    alloc_32::Allocator::builder().capacity(cap).build().expect("init");
    alloc_64::Allocator::builder().capacity(cap).build().expect("init");
    alloc_128::Allocator::builder().capacity(cap).build().expect("init");
    alloc_256::Allocator::builder().capacity(cap).build().expect("init");
    alloc_512::Allocator::builder().capacity(cap).build().expect("init");
    alloc_1024::Allocator::builder().capacity(cap).build().expect("init");
    alloc_4096::Allocator::builder().capacity(cap).build().expect("init");

    println!("TARGET: Slot (bounded_allocator!, TLS-backed slab)");
    println!("Slot handle: {} bytes", std::mem::size_of::<alloc_64::Slot>());
    println!("{}", "=".repeat(66));

    run_all_sizes_slot!(slot_churn, "CHURN (alloc + deref + drop, LIFO single-slot)");
    run_all_sizes_slot!(slot_batch_alloc, "BATCH ALLOC (100 sequential, no interleaved frees)");
    run_all_sizes_slot!(slot_batch_drop, "BATCH DROP (pre-alloc 100, then free all)");
    run_all_sizes_slot!(slot_access, "ACCESS (random deref from pool of 1000)");
    run_all_sizes_slot!(slot_cold_churn, "COLD CHURN (cache-evicted between each op, single alloc+deref+drop)");
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() != 2 || !matches!(args[1].as_str(), "box" | "slot") {
        eprintln!("Usage: {} <box|slot>", args[0]);
        eprintln!("  Runs Box or Slot benchmarks in complete isolation.");
        std::process::exit(1);
    }

    println!("ISOLATED BENCHMARK — nexus-slab (SLUB-style union SlotCell)");
    println!("Samples: {SAMPLES}, 100 unrolled ops per sample, {WARMUP} warmup iterations");
    println!("All times in CPU cycles (rdtsc)\n");

    match args[1].as_str() {
        "box" => run_box(),
        "slot" => run_slot(),
        _ => unreachable!(),
    }
}
