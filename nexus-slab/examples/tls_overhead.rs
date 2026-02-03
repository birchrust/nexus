//! Benchmark: TLS vs raw pointer access overhead
//!
//! Run with: cargo run --release --example tls_overhead

use std::cell::Cell;
use std::hint::black_box;

// Inline rdtsc for cycle counting
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

// Simple freelist-like structure
struct SlabInner {
    free_head: Cell<u32>,
    capacity: u32,
}

impl SlabInner {
    fn new(capacity: u32) -> Self {
        Self {
            free_head: Cell::new(0),
            capacity,
        }
    }

    #[inline(always)]
    fn alloc(&self) -> u32 {
        let head = self.free_head.get();
        self.free_head.set((head + 1) % self.capacity);
        head
    }

    #[inline(always)]
    fn free(&self, idx: u32) {
        self.free_head.set(idx);
    }
}

// Thread-local slab
thread_local! {
    static TLS_SLAB: SlabInner = SlabInner::new(1024);
}

fn main() {
    const WARMUP: usize = 10_000;
    const SAMPLES: usize = 100_000;
    const OPS_PER_SAMPLE: usize = 100;

    // Create a leaked pointer (simulates what we'd store in Slot)
    let raw_slab: &'static SlabInner = Box::leak(Box::new(SlabInner::new(1024)));
    let raw_ptr: *const SlabInner = raw_slab;

    println!("TLS vs Raw Pointer Overhead Benchmark");
    println!("======================================");
    println!("Measuring {} ops per sample, {} samples\n", OPS_PER_SAMPLE, SAMPLES);

    // Warmup
    for _ in 0..WARMUP {
        black_box(unsafe { (*raw_ptr).alloc() });
        TLS_SLAB.with(|s| black_box(s.alloc()));
    }

    // Benchmark 1: Raw pointer access
    let mut raw_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = rdtsc_start();
        for _ in 0..OPS_PER_SAMPLE {
            black_box(unsafe { (*raw_ptr).alloc() });
        }
        let end = rdtsc_end();
        raw_samples.push((end - start) / OPS_PER_SAMPLE as u64);
    }

    // Benchmark 2: TLS access via thread_local! macro
    let mut tls_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = rdtsc_start();
        for _ in 0..OPS_PER_SAMPLE {
            TLS_SLAB.with(|s| black_box(s.alloc()));
        }
        let end = rdtsc_end();
        tls_samples.push((end - start) / OPS_PER_SAMPLE as u64);
    }

    // Benchmark 3: TLS with cached reference (get ref once, use multiple times)
    let mut tls_cached_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = rdtsc_start();
        TLS_SLAB.with(|s| {
            for _ in 0..OPS_PER_SAMPLE {
                black_box(s.alloc());
            }
        });
        let end = rdtsc_end();
        tls_cached_samples.push((end - start) / OPS_PER_SAMPLE as u64);
    }

    // Benchmark 4: Passed reference (inlined, fair comparison)
    let mut passed_ptr_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let slab_ref: &SlabInner = raw_slab;  // simulate receiving a reference
        let start = rdtsc_start();
        for _ in 0..OPS_PER_SAMPLE {
            black_box(slab_ref.alloc());
        }
        let end = rdtsc_end();
        passed_ptr_samples.push((end - start) / OPS_PER_SAMPLE as u64);
    }

    // Benchmark 5: Simulate Slot pattern - pointer stored in struct
    struct FakeSlot {
        slab: *const SlabInner,
    }

    let fake_slot = FakeSlot { slab: raw_ptr };
    let mut slot_pattern_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = rdtsc_start();
        for _ in 0..OPS_PER_SAMPLE {
            black_box(unsafe { (*fake_slot.slab).alloc() });
        }
        let end = rdtsc_end();
        slot_pattern_samples.push((end - start) / OPS_PER_SAMPLE as u64);
    }

    // Sort and compute percentiles
    fn percentiles(samples: &mut [u64]) -> (u64, u64, u64, u64, u64) {
        samples.sort_unstable();
        let p50 = samples[samples.len() * 50 / 100];
        let p90 = samples[samples.len() * 90 / 100];
        let p99 = samples[samples.len() * 99 / 100];
        let p999 = samples[samples.len() * 999 / 1000];
        let max = *samples.last().unwrap();
        (p50, p90, p99, p999, max)
    }

    let (r50, r90, r99, r999, rmax) = percentiles(&mut raw_samples);
    let (t50, t90, t99, t999, tmax) = percentiles(&mut tls_samples);
    let (tc50, tc90, tc99, tc999, tcmax) = percentiles(&mut tls_cached_samples);
    let (p50, p90, p99, p999, pmax) = percentiles(&mut passed_ptr_samples);
    let (s50, s90, s99, s999, smax) = percentiles(&mut slot_pattern_samples);

    println!("Results (cycles per operation):");
    println!("─────────────────────────────────────────────────────────────────");
    println!("                          p50    p90    p99   p99.9    max");
    println!("─────────────────────────────────────────────────────────────────");
    println!("Raw pointer (*ptr)       {:4}   {:4}   {:4}   {:5}  {:5}", r50, r90, r99, r999, rmax);
    println!("Passed reference (&ref)  {:4}   {:4}   {:4}   {:5}  {:5}", p50, p90, p99, p999, pmax);
    println!("Slot pattern (ptr field) {:4}   {:4}   {:4}   {:5}  {:5}", s50, s90, s99, s999, smax);
    println!("TLS cached (with once)   {:4}   {:4}   {:4}   {:5}  {:5}", tc50, tc90, tc99, tc999, tcmax);
    println!("TLS each op (with each)  {:4}   {:4}   {:4}   {:5}  {:5}", t50, t90, t99, t999, tmax);
    println!("─────────────────────────────────────────────────────────────────");
    println!();
    println!("TLS overhead per access:  {} cycles", t50 as i64 - r50 as i64);
    println!("TLS vs slot pattern:      {} cycles", t50 as i64 - s50 as i64);
}
