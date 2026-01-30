//! Focused GET comparison: bounded vs slab crate
//!
//! Fair comparison - same fill level, same access patterns, isolated measurements.

use hdrhistogram::Histogram;
use std::hint::black_box;

use nexus_slab::{bounded, Key};

const CAPACITY: usize = 100_000;
const OPS: usize = 1_000_000;

#[inline(always)]
fn rdtscp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let mut aux: u32 = 0;
        std::arch::x86_64::__rdtscp(&mut aux)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        panic!("rdtscp only supported on x86_64");
    }
}

struct BoundedStats {
    get_by_key: Histogram<u64>,
    entry_get: Histogram<u64>,
    entry_get_unchecked: Histogram<u64>,
}

fn bench_bounded_by_key() -> Histogram<u64> {
    let slab = bounded::Slab::<u64>::leak(CAPACITY);
    let mut hist = Histogram::<u64>::new(3).unwrap();
    let mut keys: Vec<Key> = Vec::with_capacity(CAPACITY);

    // Fill to 50% capacity, all as keys
    for i in 0..(CAPACITY / 2) as u64 {
        let entry = slab.try_insert(i).unwrap();
        keys.push(entry.leak());
    }

    // Warmup
    for i in 0..10_000 {
        let idx = (i * 7) % keys.len();
        black_box(slab.get(keys[idx]));
    }

    // Measured gets
    for i in 0..OPS {
        let idx = (i * 7) % keys.len();
        let key = keys[idx];

        let start = rdtscp();
        let val = slab.get(key);
        black_box(val);
        let end = rdtscp();

        let _ = hist.record(end.wrapping_sub(start));
    }

    hist
}

fn bench_bounded_by_entry() -> Histogram<u64> {
    let slab = bounded::Slab::<u64>::leak(CAPACITY);
    let mut hist = Histogram::<u64>::new(3).unwrap();
    let mut entries: Vec<bounded::Entry<u64>> = Vec::with_capacity(CAPACITY);

    // Fill to 50% capacity, all as entries
    for i in 0..(CAPACITY / 2) as u64 {
        let entry = slab.try_insert(i).unwrap();
        entries.push(entry);
    }

    // Warmup
    for i in 0..10_000 {
        let idx = (i * 7) % entries.len();
        black_box(entries[idx].get());
    }

    // Measured gets
    for i in 0..OPS {
        let idx = (i * 7) % entries.len();
        let entry = &entries[idx];

        let start = rdtscp();
        let val = entry.get();
        black_box(&*val);
        drop(val);
        let end = rdtscp();

        let _ = hist.record(end.wrapping_sub(start));
    }

    hist
}

fn bench_bounded_by_entry_unchecked() -> Histogram<u64> {
    let slab = bounded::Slab::<u64>::leak(CAPACITY);
    let mut hist = Histogram::<u64>::new(3).unwrap();
    let mut entries: Vec<bounded::Entry<u64>> = Vec::with_capacity(CAPACITY);

    // Fill to 50% capacity, all as entries
    for i in 0..(CAPACITY / 2) as u64 {
        let entry = slab.try_insert(i).unwrap();
        entries.push(entry);
    }

    // Warmup
    for i in 0..10_000 {
        let idx = (i * 7) % entries.len();
        black_box(unsafe { entries[idx].get_unchecked() });
    }

    // Measured gets
    for i in 0..OPS {
        let idx = (i * 7) % entries.len();
        let entry = &entries[idx];

        let start = rdtscp();
        let val = unsafe { entry.get_unchecked() };
        black_box(val);
        let end = rdtscp();

        let _ = hist.record(end.wrapping_sub(start));
    }

    hist
}

fn bench_slab() -> Histogram<u64> {
    let mut slab = slab::Slab::<u64>::with_capacity(CAPACITY);
    let mut hist = Histogram::<u64>::new(3).unwrap();
    let mut keys: Vec<usize> = Vec::with_capacity(CAPACITY);

    // Fill to 50% capacity
    for i in 0..(CAPACITY / 2) as u64 {
        keys.push(slab.insert(i));
    }

    // Warmup
    for i in 0..10_000 {
        let idx = (i * 7) % keys.len();
        black_box(slab.get(keys[idx]));
    }

    // Measured gets
    for i in 0..OPS {
        let idx = (i * 7) % keys.len();
        let key = keys[idx];

        let start = rdtscp();
        let val = slab.get(key);
        black_box(val);
        let end = rdtscp();

        let _ = hist.record(end.wrapping_sub(start));
    }

    hist
}

fn print_hist(name: &str, hist: &Histogram<u64>) {
    println!(
        "{:>24}: p1={:>4} p5={:>4} p25={:>4} p50={:>4} p75={:>4} p90={:>4} p99={:>4} p99.9={:>5} p99.99={:>6} max={:>7}",
        name,
        hist.value_at_quantile(0.01),
        hist.value_at_quantile(0.05),
        hist.value_at_quantile(0.25),
        hist.value_at_quantile(0.50),
        hist.value_at_quantile(0.75),
        hist.value_at_quantile(0.90),
        hist.value_at_quantile(0.99),
        hist.value_at_quantile(0.999),
        hist.value_at_quantile(0.9999),
        hist.max(),
    );
}

fn main() {
    println!("GET comparison - fair isolated benchmarks");
    println!("Capacity: {}, Fill: 50%, Ops: {}", CAPACITY, OPS);
    println!("Each benchmark: fill slab, warmup, measure only get()");
    println!("================================================================\n");

    for run in 1..=5 {
        println!("Run {}:", run);

        let bounded_key = bench_bounded_by_key();
        let bounded_entry = bench_bounded_by_entry();
        let bounded_entry_unchecked = bench_bounded_by_entry_unchecked();
        let slab = bench_slab();

        print_hist("bounded get(key)", &bounded_key);
        print_hist("bounded entry.get()", &bounded_entry);
        print_hist("bounded entry.unchecked()", &bounded_entry_unchecked);
        print_hist("slab get(key)", &slab);
        println!();
    }
}
