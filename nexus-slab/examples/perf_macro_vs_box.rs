//! Full matrix: Macro/TLS Slab vs Box across sizes and percentiles.
//!
//! Run with:
//!   cargo build --release --example perf_macro_vs_box
//!   taskset -c 0 ./target/release/examples/perf_macro_vs_box

#![allow(clippy::large_stack_frames)]

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
// Macro allocators
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
        "  {:<22} {:>4} {:>5} {:>5} {:>6} {:>7} {:>7}",
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
        "  {:<22} {:>4} {:>5} {:>5} {:>6} {:>7} {:>7}",
        "", "p50", "p90", "p99", "p99.9", "p99.99", "max"
    );
}

// ============================================================================
// Unroll
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

const SAMPLES: usize = 10_000;

// ============================================================================
// Per-size benchmark
// ============================================================================

macro_rules! bench_size {
    ($name:expr, $pod:ty, $alloc_mod:ident) => {{
        // ── CHURN: alloc + deref + drop (LIFO single-slot) ──
        // This is the realistic pattern: create, use, destroy, repeat.
        // malloc gets its LIFO fast path here (same address reused).
        // Slab gets the same (same freelist head reused).
        {
            // Warmup both paths
            for _ in 0..1000 {
                let b = Box::new(<$pod>::default());
                black_box(&*b);
                drop(b);
            }
            for _ in 0..1000 {
                let s = $alloc_mod::BoxSlot::try_new(<$pod>::default()).unwrap();
                black_box(&*s);
                drop(s);
            }

            let mut box_samples = Vec::with_capacity(SAMPLES);
            let mut macro_samples = Vec::with_capacity(SAMPLES);

            // Interleaved to avoid ordering bias
            for i in 0..SAMPLES {
                if i % 2 == 0 {
                    let start = rdtsc_start();
                    unroll_100!({
                        let b = black_box(Box::new(<$pod>::default()));
                        black_box(b.data[0]);
                        drop(b);
                    });
                    let end = rdtsc_end();
                    box_samples.push((end - start) / 100);

                    let start = rdtsc_start();
                    unroll_100!({
                        let s = black_box($alloc_mod::BoxSlot::try_new(<$pod>::default()).unwrap());
                        black_box(s.data[0]);
                        drop(s);
                    });
                    let end = rdtsc_end();
                    macro_samples.push((end - start) / 100);
                } else {
                    let start = rdtsc_start();
                    unroll_100!({
                        let s = black_box($alloc_mod::BoxSlot::try_new(<$pod>::default()).unwrap());
                        black_box(s.data[0]);
                        drop(s);
                    });
                    let end = rdtsc_end();
                    macro_samples.push((end - start) / 100);

                    let start = rdtsc_start();
                    unroll_100!({
                        let b = black_box(Box::new(<$pod>::default()));
                        black_box(b.data[0]);
                        drop(b);
                    });
                    let end = rdtsc_end();
                    box_samples.push((end - start) / 100);
                }
            }

            println!("\n  ── {} CHURN (alloc+deref+drop, interleaved) ──", $name);
            print_header();
            print_row("Box", &mut box_samples);
            print_row("Macro/TLS", &mut macro_samples);
        }

        // ── ACCESS: random deref from pre-allocated pool ──
        // Both are pointer dereferences. Should be identical.
        {
            let count = 1000usize;
            let boxes: Vec<_> = (0..count).map(|_| Box::new(<$pod>::default())).collect();
            let slots: Vec<$alloc_mod::BoxSlot> = (0..count)
                .map(|_| $alloc_mod::BoxSlot::try_new(<$pod>::default()).unwrap())
                .collect();

            let mut rng = 12345u64;
            let mut box_samples = Vec::with_capacity(SAMPLES);
            for _ in 0..SAMPLES {
                rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                let base = (rng as usize) % (count - 100);
                let mut sum = 0u8;
                let mut idx = base;
                let start = rdtsc_start();
                unroll_100!({
                    sum = sum.wrapping_add(boxes[idx % count].data[0]);
                    idx += 1;
                });
                let end = rdtsc_end();
                black_box(sum);
                box_samples.push((end - start) / 100);
            }

            rng = 12345u64;
            let mut macro_samples = Vec::with_capacity(SAMPLES);
            for _ in 0..SAMPLES {
                rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                let base = (rng as usize) % (count - 100);
                let mut sum = 0u8;
                let mut idx = base;
                let start = rdtsc_start();
                unroll_100!({
                    sum = sum.wrapping_add(slots[idx % count].data[0]);
                    idx += 1;
                });
                let end = rdtsc_end();
                black_box(sum);
                macro_samples.push((end - start) / 100);
            }

            println!("\n  ── {} ACCESS (random deref from pool) ──", $name);
            print_header();
            print_row("Box deref", &mut box_samples);
            print_row("Macro deref", &mut macro_samples);

            drop(boxes);
            drop(slots);
        }
    }};
}

// ============================================================================
// Batch throughput (separate section, different workload)
// ============================================================================

macro_rules! bench_batch {
    ($name:expr, $pod:ty, $alloc_mod:ident) => {{
        // Batch alloc: 100 sequential allocations, no interleaved frees.
        // This is NOT the LIFO fast path — malloc must service 100
        // different addresses from the thread cache.
        {
            let mut box_samples = Vec::with_capacity(SAMPLES);
            for _ in 0..SAMPLES {
                let mut temp = Vec::with_capacity(100);
                let start = rdtsc_start();
                unroll_100!({
                    temp.push(black_box(Box::new(<$pod>::default())));
                });
                let end = rdtsc_end();
                box_samples.push((end - start) / 100);
                drop(temp);
            }

            let mut macro_samples = Vec::with_capacity(SAMPLES);
            for _ in 0..SAMPLES {
                let mut temp: Vec<$alloc_mod::BoxSlot> = Vec::with_capacity(100);
                let start = rdtsc_start();
                unroll_100!({
                    temp.push(black_box(
                        $alloc_mod::BoxSlot::try_new(<$pod>::default()).unwrap(),
                    ));
                });
                let end = rdtsc_end();
                macro_samples.push((end - start) / 100);
                drop(temp);
            }

            println!(
                "\n  ── {} BATCH ALLOC (100 sequential, no interleaved frees) ──",
                $name
            );
            print_header();
            print_row("Box::new()", &mut box_samples);
            print_row("Macro Slot::new() [TLS]", &mut macro_samples);
        }

        // Batch drop: pre-alloc 100, then free all.
        // malloc thread cache absorbs these; slab freelist absorbs them.
        {
            let mut box_samples = Vec::with_capacity(SAMPLES);
            for _ in 0..SAMPLES {
                let boxes: Vec<_> = (0..100).map(|_| Box::new(<$pod>::default())).collect();
                let mut iter = boxes.into_iter();
                let start = rdtsc_start();
                unroll_100!({
                    drop(black_box(iter.next()));
                });
                let end = rdtsc_end();
                box_samples.push((end - start) / 100);
            }

            let mut macro_samples = Vec::with_capacity(SAMPLES);
            for _ in 0..SAMPLES {
                let slots: Vec<$alloc_mod::BoxSlot> = (0..100)
                    .map(|_| $alloc_mod::BoxSlot::try_new(<$pod>::default()).unwrap())
                    .collect();
                let mut iter = slots.into_iter();
                let start = rdtsc_start();
                unroll_100!({
                    drop(black_box(iter.next()));
                });
                let end = rdtsc_end();
                macro_samples.push((end - start) / 100);
            }

            println!(
                "\n  ── {} BATCH DROP (pre-alloc 100, then free all) ──",
                $name
            );
            print_header();
            print_row("drop(Box)", &mut box_samples);
            print_row("drop(Slot) [TLS]", &mut macro_samples);
        }
    }};
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    let cap = 20_000;
    alloc_32::Allocator::builder()
        .capacity(cap)
        .build()
        .expect("init 32");
    alloc_64::Allocator::builder()
        .capacity(cap)
        .build()
        .expect("init 64");
    alloc_128::Allocator::builder()
        .capacity(cap)
        .build()
        .expect("init 128");
    alloc_256::Allocator::builder()
        .capacity(cap)
        .build()
        .expect("init 256");
    alloc_512::Allocator::builder()
        .capacity(cap)
        .build()
        .expect("init 512");
    alloc_1024::Allocator::builder()
        .capacity(cap)
        .build()
        .expect("init 1024");
    alloc_4096::Allocator::builder()
        .capacity(cap)
        .build()
        .expect("init 4096");

    println!("MACRO/TLS SLAB vs BOX — FULL SIZE × PERCENTILE MATRIX");
    println!("======================================================");
    println!(
        "Macro Slot: {} bytes | Box: {} bytes",
        std::mem::size_of::<alloc_64::BoxSlot>(),
        std::mem::size_of::<Box<Pod64>>()
    );
    println!("Samples: {}, 100 unrolled ops per sample", SAMPLES);
    println!("All times in CPU cycles (rdtsc)");

    // ── SECTION 1: Churn + Access per size ──
    // Churn uses LIFO single-slot pattern (alloc, use, free, repeat).
    // This is malloc's best case (same address reused from thread cache).
    println!("\n{}", "=".repeat(60));
    println!("CHURN + ACCESS (per size)");
    println!("{}", "=".repeat(60));

    bench_size!("32B", Pod32, alloc_32);
    bench_size!("64B", Pod64, alloc_64);
    bench_size!("128B", Pod128, alloc_128);
    bench_size!("256B", Pod256, alloc_256);
    bench_size!("512B", Pod512, alloc_512);
    bench_size!("1024B", Pod1024, alloc_1024);
    bench_size!("4096B", Pod4096, alloc_4096);

    // ── SECTION 2: Batch throughput ──
    // Different workload: allocate many without freeing, then free many.
    // These numbers do NOT compose with churn (different allocator behavior).
    println!("\n{}", "=".repeat(60));
    println!("BATCH THROUGHPUT (different workload from churn)");
    println!("{}", "=".repeat(60));
    println!("  Alloc: 100 sequential allocations, no interleaved frees.");
    println!("  Drop:  pre-alloc 100, then free all sequentially.");
    println!("  malloc must manage 100 concurrent allocations (not LIFO).");

    bench_batch!("32B", Pod32, alloc_32);
    bench_batch!("64B", Pod64, alloc_64);
    bench_batch!("128B", Pod128, alloc_128);
    bench_batch!("256B", Pod256, alloc_256);
    bench_batch!("512B", Pod512, alloc_512);
    bench_batch!("1024B", Pod1024, alloc_1024);
    bench_batch!("4096B", Pod4096, alloc_4096);

    println!("\n{}", "=".repeat(60));
    println!("Notes:");
    println!("  Churn = LIFO single-slot: alloc → use → free → repeat.");
    println!("    malloc reuses the same address from thread cache.");
    println!("    This is malloc's best case. Slab freelist is similar.");
    println!("  Batch = allocate/free many at once.");
    println!("    malloc must find N different addresses (no LIFO reuse).");
    println!("    These numbers do NOT sum to churn.");
    println!("  Access = pointer deref. Identical for Box and Slab.");
}
