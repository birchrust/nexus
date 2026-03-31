//! Benchmark: nexus-slab Slot vs Box
//!
//! Compares allocation patterns between pre-allocated slab and heap allocation.
//!
//! Run with: `taskset -c 0 ./target/release/examples/perf_vs_box`

#![allow(clippy::items_after_statements)]

use nexus_slab::SlotPtr;
use nexus_slab::bounded::Slab as BoundedSlab;
use std::hint::black_box;

// ============================================================================
// Timing Infrastructure
// ============================================================================

#[inline(always)]
fn rdtsc_start() -> u64 {
    unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    }
}

#[inline(always)]
fn rdtsc_end() -> u64 {
    unsafe {
        let tsc = core::arch::x86_64::__rdtscp(&mut 0u32 as *mut _);
        core::arch::x86_64::_mm_lfence();
        tsc
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn print_stats(name: &str, samples: &mut [u64]) {
    samples.sort_unstable();
    println!(
        "  {:<30} p50={:>4}  p90={:>4}  p99={:>4}  p99.9={:>5}  max={:>6}",
        name,
        percentile(samples, 50.0),
        percentile(samples, 90.0),
        percentile(samples, 99.0),
        percentile(samples, 99.9),
        samples[samples.len() - 1]
    );
}

// ============================================================================
// Test Types
// ============================================================================

const SEPARATOR: &str = "----------------------------------------------------------------------";

/// Simulates a realistic struct (e.g., an Order)
#[derive(Clone)]
#[allow(dead_code)]
pub struct TestValue {
    id: u64,
    price: u64,
    quantity: u64,
    flags: u64,
}

impl TestValue {
    fn new(id: u64) -> Self {
        Self {
            id,
            price: id * 100,
            quantity: id * 10,
            flags: 0,
        }
    }
}

// ============================================================================
// Macros for unrolled timing
// ============================================================================

macro_rules! unroll_10 {
    ($op:expr) => {
        $op;
        $op;
        $op;
        $op;
        $op;
        $op;
        $op;
        $op;
        $op;
        $op;
    };
}

macro_rules! unroll_100 {
    ($op:expr) => {
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
        unroll_10!($op);
    };
}

// ============================================================================
// Benchmarks
// ============================================================================

const SAMPLES: usize = 5000;
const POOL_SIZE: usize = 10_000;

fn bench_allocation() {
    println!("\nALLOCATION (cycles per operation)");
    println!("{}", SEPARATOR);

    // --- Box allocation ---
    {
        let mut samples = Vec::with_capacity(SAMPLES);

        for i in 0..SAMPLES {
            let val = TestValue::new(i as u64);
            let start = rdtsc_start();
            unroll_100!({
                let b = Box::new(val.clone());
                black_box(b);
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("Box::new()", &mut samples);
    }

    // --- Slab allocation ---
    {
        let alloc: BoundedSlab<TestValue> = unsafe { BoundedSlab::with_capacity(POOL_SIZE * 2) };

        let mut samples = Vec::with_capacity(SAMPLES);

        for i in 0..SAMPLES {
            // Pre-clear to avoid "full" state
            let val = TestValue::new(i as u64);
            let start = rdtsc_start();
            unroll_100!({
                let s = alloc.alloc(val.clone());
                black_box(&*s);
                // SAFETY: slot was allocated from this slab
                alloc.free(s);
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("slab::insert()", &mut samples);
    }
}

fn bench_deallocation() {
    println!("\nDEALLOCATION (cycles per operation)");
    println!("{}", SEPARATOR);

    // --- Box deallocation ---
    {
        let mut samples = Vec::with_capacity(SAMPLES);

        for i in 0..SAMPLES {
            #[allow(clippy::needless_collect)]
            let boxes: Vec<_> = (0..100)
                .map(|j| Box::new(TestValue::new((i * 100 + j) as u64)))
                .collect();
            let mut iter = boxes.into_iter();
            let start = rdtsc_start();
            unroll_100!({
                drop(black_box(iter.next()));
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("drop(Box)", &mut samples);
    }

    // --- Slab deallocation ---
    {
        let alloc: BoundedSlab<TestValue> = unsafe { BoundedSlab::with_capacity(POOL_SIZE * 2) };

        let mut samples = Vec::with_capacity(SAMPLES);

        for i in 0..SAMPLES {
            // Pre-allocate slots
            let slots: Vec<SlotPtr<TestValue>> = (0..100)
                .map(|j| alloc.alloc(TestValue::new((i * 100 + j) as u64)))
                .collect();

            let mut iter = slots.into_iter();
            let start = rdtsc_start();
            unroll_100!({
                let slot = iter.next().unwrap();
                // SAFETY: slot was allocated from this slab
                alloc.free(slot);
                black_box(());
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("slab.free(SlotPtr)", &mut samples);
    }
}

fn bench_access() {
    println!("\nACCESS (cycles per operation)");
    println!("{}", SEPARATOR);

    // --- Box access ---
    {
        let boxes: Vec<_> = (0..POOL_SIZE)
            .map(|i| Box::new(TestValue::new(i as u64)))
            .collect();

        let mut samples = Vec::with_capacity(SAMPLES);
        let mut rng_state = 12345u64;

        for _ in 0..SAMPLES {
            // LCG for deterministic "random" access
            rng_state = rng_state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let base_idx = (rng_state as usize) % (POOL_SIZE - 100);

            let mut sum = 0u64;
            let mut idx = base_idx;
            let start = rdtsc_start();
            unroll_100!({
                sum += boxes[idx % POOL_SIZE].id;
                idx += 1;
            });
            let end = rdtsc_end();
            black_box(sum);
            samples.push((end - start) / 100);
        }
        print_stats("Box deref (random)", &mut samples);
    }

    // --- Slab access ---
    {
        let alloc: BoundedSlab<TestValue> = unsafe { BoundedSlab::with_capacity(POOL_SIZE) };

        let slots: Vec<SlotPtr<TestValue>> = (0..POOL_SIZE)
            .map(|i| alloc.alloc(TestValue::new(i as u64)))
            .collect();

        let mut samples = Vec::with_capacity(SAMPLES);
        let mut rng_state = 12345u64;

        for _ in 0..SAMPLES {
            rng_state = rng_state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let base_idx = (rng_state as usize) % (POOL_SIZE - 100);

            let mut sum = 0u64;
            let mut idx = base_idx;
            let start = rdtsc_start();
            unroll_100!({
                sum += slots[idx % POOL_SIZE].id;
                idx += 1;
            });
            let end = rdtsc_end();
            black_box(sum);
            samples.push((end - start) / 100);
        }
        print_stats("SlotPtr deref (random)", &mut samples);

        // Cleanup
        for slot in slots {
            // SAFETY: slot was allocated from this slab
            alloc.free(slot);
        }
    }
}

fn bench_churn() {
    println!("\nCHURN - insert/remove cycles (cycles per insert+remove pair)");
    println!("{}", SEPARATOR);

    // --- Box churn ---
    {
        let mut samples = Vec::with_capacity(SAMPLES);

        // Pre-warm the allocator
        let warmup: Vec<_> = (0..POOL_SIZE)
            .map(|i| Box::new(TestValue::new(i as u64)))
            .collect();
        drop(warmup);

        for i in 0..SAMPLES {
            let val = TestValue::new(i as u64);
            let start = rdtsc_start();
            unroll_100!({
                let b = Box::new(val.clone());
                black_box(&*b);
                drop(b);
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("Box new+drop", &mut samples);
    }

    // --- Slab churn ---
    {
        let alloc: BoundedSlab<TestValue> = unsafe { BoundedSlab::with_capacity(POOL_SIZE) };

        // Pre-warm
        let warmup: Vec<SlotPtr<TestValue>> = (0..POOL_SIZE / 2)
            .map(|i| alloc.alloc(TestValue::new(i as u64)))
            .collect();
        for slot in warmup {
            // SAFETY: slot was allocated from this slab
            alloc.free(slot);
        }

        let mut samples = Vec::with_capacity(SAMPLES);

        for i in 0..SAMPLES {
            let val = TestValue::new(i as u64);
            let start = rdtsc_start();
            unroll_100!({
                let s = alloc.alloc(val.clone());
                black_box(&*s);
                // SAFETY: slot was allocated from this slab
                alloc.free(s);
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("SlotPtr insert+free", &mut samples);
    }
}

fn bench_realistic_workload() {
    println!("\nREALISTIC WORKLOAD - mixed operations");
    println!("{}", SEPARATOR);
    println!("  Pattern: 60% access, 20% insert, 20% remove (steady state)");

    const WORKING_SET: usize = 1000;
    const OPS: usize = 100_000;

    // --- Box workload ---
    {
        let mut boxes: Vec<Option<Box<TestValue>>> = (0..WORKING_SET)
            .map(|i| Some(Box::new(TestValue::new(i as u64))))
            .collect();

        let mut rng = 12345u64;
        let mut next_id = WORKING_SET as u64;

        let start = rdtsc_start();
        for _ in 0..OPS {
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let idx = (rng as usize) % WORKING_SET;
            let action = (rng >> 32) % 10;

            match action {
                0..=5 => {
                    // Access (60%)
                    if let Some(ref b) = boxes[idx] {
                        black_box(b.id);
                    }
                }
                6..=7 => {
                    // Insert (20%)
                    if boxes[idx].is_none() {
                        boxes[idx] = Some(Box::new(TestValue::new(next_id)));
                        next_id += 1;
                    }
                }
                _ => {
                    // Remove (20%)
                    if boxes[idx].is_some() {
                        black_box(boxes[idx].take());
                    }
                }
            }
        }
        let end = rdtsc_end();

        let cycles_per_op = (end - start) / OPS as u64;
        println!("  Box workload:  {} cycles/op average", cycles_per_op);

        drop(boxes);
    }

    // --- Slab workload ---
    {
        let alloc: BoundedSlab<TestValue> = unsafe { BoundedSlab::with_capacity(WORKING_SET * 2) };

        let mut slots: Vec<Option<SlotPtr<TestValue>>> = (0..WORKING_SET)
            .map(|i| Some(alloc.alloc(TestValue::new(i as u64))))
            .collect();

        let mut rng = 12345u64;
        let mut next_id = WORKING_SET as u64;

        let start = rdtsc_start();
        for _ in 0..OPS {
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let idx = (rng as usize) % WORKING_SET;
            let action = (rng >> 32) % 10;

            match action {
                0..=5 => {
                    // Access (60%)
                    if let Some(ref s) = slots[idx] {
                        black_box(s.id);
                    }
                }
                6..=7 => {
                    // Insert (20%)
                    if slots[idx].is_none() {
                        slots[idx] = Some(alloc.alloc(TestValue::new(next_id)));
                        next_id += 1;
                    }
                }
                _ => {
                    // Remove (20%)
                    if let Some(slot) = slots[idx].take() {
                        // SAFETY: slot was allocated from this slab
                        alloc.free(slot);
                    }
                }
            }
        }
        let end = rdtsc_end();

        let cycles_per_op = (end - start) / OPS as u64;
        println!("  Slab workload: {} cycles/op average", cycles_per_op);

        // Cleanup remaining slots
        for slot in slots.into_iter().flatten() {
            // SAFETY: slot was allocated from this slab
            alloc.free(slot);
        }
    }
}

fn main() {
    println!("NEXUS-SLAB vs BOX - ALLOCATION COMPARISON");
    println!("==========================================");
    println!(
        "Value size: {} bytes (TestValue)",
        std::mem::size_of::<TestValue>()
    );
    println!("Pool size: {} items", POOL_SIZE);
    println!();

    bench_allocation();
    bench_deallocation();
    bench_access();
    bench_churn();
    bench_realistic_workload();

    println!("\n==========================================");
    println!("Note: Box uses the system allocator (malloc/free).");
    println!("Slab uses pre-allocated memory with freelist management.");
    println!("Slab wins on allocation/deallocation; access should be equal.");
}
