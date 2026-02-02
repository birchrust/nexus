//! Minimal isolated contention test - run this ALONE for accurate numbers

use std::hint::black_box;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Pod64 { data: [u8; 64] }
impl Default for Pod64 { fn default() -> Self { Self { data: [0; 64] } } }

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Pod256 { data: [u8; 256] }
impl Default for Pod256 { fn default() -> Self { Self { data: [0; 256] } } }

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Pod1024 { data: [u8; 1024] }
impl Default for Pod1024 { fn default() -> Self { Self { data: [0; 1024] } } }

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Pod4096 { data: [u8; 4096] }
impl Default for Pod4096 { fn default() -> Self { Self { data: [0; 4096] } } }

nexus_slab::create_allocator!(alloc64, crate::Pod64);
nexus_slab::create_allocator!(alloc256, crate::Pod256);
nexus_slab::create_allocator!(alloc1024, crate::Pod1024);
nexus_slab::create_allocator!(alloc4096, crate::Pod4096);

const SAMPLES: usize = 5000;
const BATCH: usize = 100;

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
    println!("  {} p25={:3} p50={:3} p75={:3} p90={:3} p99={:4} p99.9={:5} p99.99={:6} max={:6}",
        name,
        percentile(samples, 25.0),
        percentile(samples, 50.0),
        percentile(samples, 75.0),
        percentile(samples, 90.0),
        percentile(samples, 99.0),
        percentile(samples, 99.9),
        percentile(samples, 99.99),
        samples.last().unwrap());
}

macro_rules! contention_test {
    ($name:literal, $pod:ty, $alloc:ident) => {{
        println!("\n  ── {} ──", $name);
        
        $alloc::init().bounded(SAMPLES * 2).build();
        
        let mut rng = 12345u64;
        let mut box_samples = Vec::with_capacity(SAMPLES);
        let mut slab_samples = Vec::with_capacity(SAMPLES);
        
        // Box with contention
        for _ in 0..SAMPLES {
            // Random noise to global allocator (NOT timed)
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let noise_count = 20 + (rng % 60) as usize;
            let mut noise: Vec<Box<[u8]>> = Vec::with_capacity(noise_count);
            for _ in 0..noise_count {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let size = 32 << (rng % 6);
                noise.push(vec![0u8; size as usize].into_boxed_slice());
            }
            // Swiss cheese
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let keep = 50 + (rng % 25) as usize;
            while noise.len() > keep { 
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let idx = (rng as usize) % noise.len();
                noise.swap_remove(idx); 
            }
            
            // Time batch
            let start = rdtsc_start();
            for _ in 0..BATCH {
                let item = Box::new(<$pod>::default());
                black_box(&*item);
                drop(item);
            }
            let elapsed = rdtsc_end() - start;
            box_samples.push(elapsed / BATCH as u64);
            drop(noise);
        }
        
        // Reset RNG for same noise pattern
        rng = 12345u64;
        
        // Slab with same noise
        for _ in 0..SAMPLES {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let noise_count = 20 + (rng % 60) as usize;
            let mut noise: Vec<Box<[u8]>> = Vec::with_capacity(noise_count);
            for _ in 0..noise_count {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let size = 32 << (rng % 6);
                noise.push(vec![0u8; size as usize].into_boxed_slice());
            }
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let keep = 50 + (rng % 25) as usize;
            while noise.len() > keep { 
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let idx = (rng as usize) % noise.len();
                noise.swap_remove(idx); 
            }
            
            let start = rdtsc_start();
            for _ in 0..BATCH {
                let slot = $alloc::insert(<$pod>::default());
                black_box(&*slot);
                drop(slot);
            }
            let elapsed = rdtsc_end() - start;
            slab_samples.push(elapsed / BATCH as u64);
            drop(noise);
        }
        
        print_stats(&format!("Box  ({})", $name), &mut box_samples);
        print_stats(&format!("Slab ({})", $name), &mut slab_samples);
        
        let _ = $alloc::shutdown();
    }};
}

fn main() {
    println!("ISOLATED CONTENTION TEST");
    println!("========================");
    
    contention_test!("64B", Pod64, alloc64);
    contention_test!("256B", Pod256, alloc256);
    contention_test!("1024B", Pod1024, alloc1024);
    contention_test!("4096B", Pod4096, alloc4096);
}
