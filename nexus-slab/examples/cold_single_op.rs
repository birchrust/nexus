//! Cold cache test - SINGLE OPERATION per eviction
//!
//! Measures truly cold first operation, not amortized over batch

use nexus_slab::bounded::Slab as BoundedSlab;
use std::hint::black_box;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Pod64 {
    data: [u8; 64],
}
impl Default for Pod64 {
    fn default() -> Self {
        Self { data: [0; 64] }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Pod256 {
    data: [u8; 256],
}
impl Default for Pod256 {
    fn default() -> Self {
        Self { data: [0; 256] }
    }
}

const SAMPLES: usize = 2000; // Fewer samples since we evict each time

use std::cell::UnsafeCell;
struct EvictBuffer(UnsafeCell<[u8; 24 * 1024 * 1024]>);
unsafe impl Sync for EvictBuffer {}
static EVICT_BUFFER: EvictBuffer = EvictBuffer(UnsafeCell::new([0u8; 24 * 1024 * 1024]));

#[inline(never)]
fn evict_cache() {
    unsafe {
        let buf = &mut *EVICT_BUFFER.0.get();
        let len = buf.len();
        let stride = 4097;
        let mut i = 0;
        while i < len {
            buf[i] = buf[i].wrapping_add(1);
            i += stride;
        }
        i = 64;
        while i < len {
            buf[i] = buf[i].wrapping_add(1);
            i += stride;
        }
        black_box(buf);
    }
}

#[inline(never)]
fn rdtsc_start() -> u64 {
    unsafe {
        core::arch::x86_64::_mm_lfence();
        core::arch::x86_64::_rdtsc()
    }
}

#[inline(never)]
fn rdtsc_end() -> u64 {
    let mut aux: u32 = 0;
    unsafe {
        let t = core::arch::x86_64::__rdtscp(&mut aux);
        core::arch::x86_64::_mm_lfence();
        t
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn print_stats(name: &str, samples: &mut [u64]) {
    samples.sort_unstable();
    println!(
        "  {} p25={:3} p50={:3} p75={:3} p90={:3} p99={:4}",
        name,
        percentile(samples, 25.0),
        percentile(samples, 50.0),
        percentile(samples, 75.0),
        percentile(samples, 90.0),
        percentile(samples, 99.0)
    );
}

fn main() {
    println!("COLD CACHE - SINGLE OP (one alloc+free per eviction)");
    println!("=====================================================");
    println!("  Note: rdtsc overhead (~20-25 cycles) included in measurements");

    // 64B test
    {
        println!("\n  -- 64B SINGLE OP --");
        // SAFETY: slab outlives all slots
        let slab = unsafe { BoundedSlab::<Pod64>::new((SAMPLES * 4) as u32) };

        let mut box_samples = Vec::with_capacity(SAMPLES);
        let mut slab_samples = Vec::with_capacity(SAMPLES);

        // Interleaved to avoid ordering bias
        for i in 0..SAMPLES {
            if i % 2 == 0 {
                // Box first
                evict_cache();
                let start = rdtsc_start();
                let item = Box::new(Pod64::default());
                black_box(&*item);
                drop(item);
                let elapsed = rdtsc_end() - start;
                box_samples.push(elapsed);

                evict_cache();
                let start = rdtsc_start();
                let slot = slab.alloc(Pod64::default());
                black_box(&*slot);
                drop(slot);
                let elapsed = rdtsc_end() - start;
                slab_samples.push(elapsed);
            } else {
                // Slab first
                evict_cache();
                let start = rdtsc_start();
                let slot = slab.alloc(Pod64::default());
                black_box(&*slot);
                drop(slot);
                let elapsed = rdtsc_end() - start;
                slab_samples.push(elapsed);

                evict_cache();
                let start = rdtsc_start();
                let item = Box::new(Pod64::default());
                black_box(&*item);
                drop(item);
                let elapsed = rdtsc_end() - start;
                box_samples.push(elapsed);
            }
        }

        print_stats("Box  (alloc+free)", &mut box_samples);
        print_stats("Slab (alloc+free)", &mut slab_samples);
    }

    // 256B test
    {
        println!("\n  -- 256B SINGLE OP --");
        // SAFETY: slab outlives all slots
        let slab = unsafe { BoundedSlab::<Pod256>::new((SAMPLES * 4) as u32) };

        let mut box_samples = Vec::with_capacity(SAMPLES);
        let mut slab_samples = Vec::with_capacity(SAMPLES);

        for i in 0..SAMPLES {
            if i % 2 == 0 {
                evict_cache();
                let start = rdtsc_start();
                let item = Box::new(Pod256::default());
                black_box(&*item);
                drop(item);
                let elapsed = rdtsc_end() - start;
                box_samples.push(elapsed);

                evict_cache();
                let start = rdtsc_start();
                let slot = slab.alloc(Pod256::default());
                black_box(&*slot);
                drop(slot);
                let elapsed = rdtsc_end() - start;
                slab_samples.push(elapsed);
            } else {
                evict_cache();
                let start = rdtsc_start();
                let slot = slab.alloc(Pod256::default());
                black_box(&*slot);
                drop(slot);
                let elapsed = rdtsc_end() - start;
                slab_samples.push(elapsed);

                evict_cache();
                let start = rdtsc_start();
                let item = Box::new(Pod256::default());
                black_box(&*item);
                drop(item);
                let elapsed = rdtsc_end() - start;
                box_samples.push(elapsed);
            }
        }

        print_stats("Box  (alloc+free)", &mut box_samples);
        print_stats("Slab (alloc+free)", &mut slab_samples);
    }
}
