//! Benchmark: Macro-generated TLS allocator vs direct bounded::Slab
//!
//! Measures the actual cost of the TLS indirection in the macro path.
//!
//! The macro path (`alloc::Slot<A>`) hits TLS on alloc and drop.
//! Deref is identical (both are raw pointer dereferences).
//!
//! Run with: `cargo run --release --example tls_overhead`

use std::hint::black_box;

use nexus_slab::bounded::Slab as BoundedSlab;
use nexus_slab::unbounded::Slab as UnboundedSlab;

// Macro-generated bounded allocator
mod bounded_alloc {
    #[derive(Clone, Copy)]
    #[repr(C)]
    pub struct Pod64 {
        pub data: [u8; 64],
    }
    impl Default for Pod64 {
        fn default() -> Self { Self { data: [0; 64] } }
    }

    nexus_slab::bounded_allocator!(Pod64);
}

// Macro-generated unbounded allocator
mod unbounded_alloc {
    #[derive(Clone, Copy)]
    #[repr(C)]
    pub struct Pod64 {
        pub data: [u8; 64],
    }
    impl Default for Pod64 {
        fn default() -> Self { Self { data: [0; 64] } }
    }

    nexus_slab::unbounded_allocator!(Pod64);
}

// Direct slab types use the same pod
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Pod64 {
    pub data: [u8; 64],
}
impl Default for Pod64 {
    fn default() -> Self { Self { data: [0; 64] } }
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

fn print_stats(name: &str, samples: &mut [u64]) {
    samples.sort_unstable();
    println!(
        "  {:<35} p50={:>4}  p90={:>4}  p99={:>4}  p99.9={:>5}  max={:>6}",
        name,
        percentile(samples, 50.0),
        percentile(samples, 90.0),
        percentile(samples, 99.0),
        percentile(samples, 99.9),
        samples[samples.len() - 1]
    );
}

// ============================================================================
// Unroll macro
// ============================================================================

macro_rules! unroll_10 {
    ($op:expr) => {
        $op; $op; $op; $op; $op; $op; $op; $op; $op; $op;
    };
}

macro_rules! unroll_100 {
    ($op:expr) => {
        unroll_10!($op); unroll_10!($op); unroll_10!($op); unroll_10!($op); unroll_10!($op);
        unroll_10!($op); unroll_10!($op); unroll_10!($op); unroll_10!($op); unroll_10!($op);
    };
}

const SAMPLES: usize = 5000;

// ============================================================================
// BOUNDED: Direct vs Macro
// ============================================================================

fn bench_bounded() {
    println!("\nBOUNDED: Direct Slab vs Macro (TLS) Path");
    println!("─────────────────────────────────────────────────────────────────");

    let capacity = 20_000u32;
    let direct_slab = BoundedSlab::<Pod64>::new(capacity);
    bounded_alloc::Allocator::builder()
        .capacity(capacity as usize)
        .build()
        .expect("init bounded");

    let val = Pod64::default();

    // --- Alloc only ---
    println!("\n  ALLOC (cycles per alloc):");
    {
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = rdtsc_start();
            unroll_100!({
                let s = direct_slab.try_new_slot(val).unwrap();
                black_box(&s);
                drop(s);
            });
            let end = rdtsc_end();
            // This measures alloc+drop, we'll isolate below
            samples.push((end - start) / 100);
        }
        print_stats("Direct (alloc+drop cycle)", &mut samples);
    }
    {
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = rdtsc_start();
            unroll_100!({
                let s = bounded_alloc::Slot::new(bounded_alloc::Pod64::default());
                black_box(&*s);
                drop(s);
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("Macro/TLS (alloc+drop cycle)", &mut samples);
    }

    // --- Deref (should be identical) ---
    println!("\n  DEREF (cycles per deref, slots pre-allocated):");
    {
        // Pre-allocate a batch of direct slots
        let slots: Vec<_> = (0..100)
            .map(|_| direct_slab.try_new_slot(val).unwrap())
            .collect();

        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let mut sum = 0u8;
            let mut idx = 0usize;
            let start = rdtsc_start();
            unroll_100!({
                sum = sum.wrapping_add(slots[idx % 100].data[0]);
                idx += 1;
            });
            let end = rdtsc_end();
            black_box(sum);
            samples.push((end - start) / 100);
        }
        print_stats("Direct deref", &mut samples);
        drop(slots);
    }
    {
        let slots: Vec<_> = (0..100)
            .map(|_| bounded_alloc::Slot::new(bounded_alloc::Pod64::default()))
            .collect();

        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let mut sum = 0u8;
            let mut idx = 0usize;
            let start = rdtsc_start();
            unroll_100!({
                sum = sum.wrapping_add(slots[idx % 100].data[0]);
                idx += 1;
            });
            let end = rdtsc_end();
            black_box(sum);
            samples.push((end - start) / 100);
        }
        print_stats("Macro/TLS deref", &mut samples);
        drop(slots);
    }

    // --- Drop only (pre-allocate, then time the drops) ---
    println!("\n  DROP (cycles per drop, pre-allocated batch):");
    {
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            // Pre-allocate 100 slots (not timed)
            let slots: Vec<_> = (0..100)
                .map(|_| direct_slab.try_new_slot(val).unwrap())
                .collect();

            let mut iter = slots.into_iter();
            let start = rdtsc_start();
            unroll_100!({
                drop(black_box(iter.next()));
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("Direct drop", &mut samples);
    }
    {
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let slots: Vec<_> = (0..100)
                .map(|_| bounded_alloc::Slot::new(bounded_alloc::Pod64::default()))
                .collect();

            let mut iter = slots.into_iter();
            let start = rdtsc_start();
            unroll_100!({
                drop(black_box(iter.next()));
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("Macro/TLS drop", &mut samples);
    }
}

// ============================================================================
// UNBOUNDED: Direct vs Macro
// ============================================================================

fn bench_unbounded() {
    println!("\n\nUNBOUNDED: Direct Slab vs Macro (TLS) Path");
    println!("─────────────────────────────────────────────────────────────────");

    let chunk_size = 4096u32;
    let direct_slab = UnboundedSlab::<Pod64>::new(chunk_size);
    unbounded_alloc::Allocator::builder()
        .chunk_size(chunk_size as usize)
        .build()
        .expect("init unbounded");

    let val = Pod64::default();

    // --- Churn (alloc + deref + drop) ---
    println!("\n  CHURN - alloc+deref+drop (cycles per full cycle):");
    {
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = rdtsc_start();
            unroll_100!({
                let s = direct_slab.new_slot(val);
                black_box(s.data[0]);
                drop(s);
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("Direct (alloc+deref+drop)", &mut samples);
    }
    {
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = rdtsc_start();
            unroll_100!({
                let s = unbounded_alloc::Slot::new(unbounded_alloc::Pod64::default());
                black_box(s.data[0]);
                drop(s);
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("Macro/TLS (alloc+deref+drop)", &mut samples);
    }

    // --- Drop only ---
    println!("\n  DROP (cycles per drop, pre-allocated batch):");
    {
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let slots: Vec<_> = (0..100)
                .map(|_| direct_slab.new_slot(val))
                .collect();

            let mut iter = slots.into_iter();
            let start = rdtsc_start();
            unroll_100!({
                drop(black_box(iter.next()));
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("Direct drop", &mut samples);
    }
    {
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let slots: Vec<_> = (0..100)
                .map(|_| unbounded_alloc::Slot::new(unbounded_alloc::Pod64::default()))
                .collect();

            let mut iter = slots.into_iter();
            let start = rdtsc_start();
            unroll_100!({
                drop(black_box(iter.next()));
            });
            let end = rdtsc_end();
            samples.push((end - start) / 100);
        }
        print_stats("Macro/TLS drop", &mut samples);
    }
}

// ============================================================================
// Summary
// ============================================================================

fn bench_side_by_side() {
    println!("\n\nSIDE-BY-SIDE: Bounded Churn (the number that matters)");
    println!("─────────────────────────────────────────────────────────────────");
    println!("  Pattern: alloc Pod64, read one byte, drop. 100 unrolled ops/sample.");

    let capacity = 20_000u32;

    // Direct path uses the already-created slab from bench_bounded,
    // but we create a fresh one to be fair
    let direct_slab = BoundedSlab::<Pod64>::new(capacity);
    let val = Pod64::default();

    // Interleaved to avoid ordering bias
    let mut direct_samples = Vec::with_capacity(SAMPLES);
    let mut macro_samples = Vec::with_capacity(SAMPLES);

    for i in 0..SAMPLES {
        if i % 2 == 0 {
            // Direct first
            let start = rdtsc_start();
            unroll_100!({
                let s = direct_slab.try_new_slot(val).unwrap();
                black_box(s.data[0]);
                drop(s);
            });
            let end = rdtsc_end();
            direct_samples.push((end - start) / 100);

            let start = rdtsc_start();
            unroll_100!({
                let s = bounded_alloc::Slot::new(bounded_alloc::Pod64::default());
                black_box(s.data[0]);
                drop(s);
            });
            let end = rdtsc_end();
            macro_samples.push((end - start) / 100);
        } else {
            // Macro first
            let start = rdtsc_start();
            unroll_100!({
                let s = bounded_alloc::Slot::new(bounded_alloc::Pod64::default());
                black_box(s.data[0]);
                drop(s);
            });
            let end = rdtsc_end();
            macro_samples.push((end - start) / 100);

            let start = rdtsc_start();
            unroll_100!({
                let s = direct_slab.try_new_slot(val).unwrap();
                black_box(s.data[0]);
                drop(s);
            });
            let end = rdtsc_end();
            direct_samples.push((end - start) / 100);
        }
    }

    print_stats("Direct bounded churn", &mut direct_samples);
    print_stats("Macro/TLS bounded churn", &mut macro_samples);

    // Compute delta
    direct_samples.sort_unstable();
    macro_samples.sort_unstable();
    let d50 = percentile(&direct_samples, 50.0);
    let m50 = percentile(&macro_samples, 50.0);
    let d99 = percentile(&direct_samples, 99.0);
    let m99 = percentile(&macro_samples, 99.0);
    println!();
    println!("  TLS tax (p50):  {} cycles/op  ({} direct → {} macro)", m50 as i64 - d50 as i64, d50, m50);
    println!("  TLS tax (p99):  {} cycles/op  ({} direct → {} macro)", m99 as i64 - d99 as i64, d99, m99);
    println!("  Note: TLS is hit twice per churn cycle (alloc + drop)");
}

fn main() {
    println!("MACRO (TLS) vs DIRECT SLAB — OVERHEAD MEASUREMENT");
    println!("===================================================");
    println!("Pod size: 64 bytes");
    println!("Samples: {}, 100 unrolled ops per sample", SAMPLES);
    println!("All times in CPU cycles (lfence+rdtsc)");

    bench_bounded();
    bench_unbounded();
    bench_side_by_side();

    println!("\n===================================================");
    println!("Legend:");
    println!("  Direct      = bounded::Slab / unbounded::Slab (no TLS)");
    println!("  Macro/TLS   = bounded_allocator!() / unbounded_allocator!()");
    println!("  TLS is hit on alloc() and drop() — NOT on deref");
}
