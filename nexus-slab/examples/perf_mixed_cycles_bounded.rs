//! Bounded slab benchmark: compare BoundedSlab vs slab crate
//!
//! Both pre-allocated to same capacity - pure operation comparison.
//! Tests insert/get/remove latency without growth overhead.

use hdrhistogram::Histogram;
use std::hint::black_box;

const CAPACITY: usize = 100_000;
const OPERATIONS: usize = 1_000_000;
const SEED: u64 = 0xDEADBEEF;

#[inline(always)]
fn rdtscp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let mut aux: u32 = 0;
        std::arch::x86_64::__rdtscp(&mut aux)
    }
    #[cfg(not(target_arch = "x86_64"))]
    panic!("rdtscp only supported on x86_64");
}

struct Xorshift {
    state: u64,
}

impl Xorshift {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    fn next_usize(&mut self, max: usize) -> usize {
        (self.next() as usize) % max
    }
}

struct Stats {
    insert: Histogram<u64>,
    get: Histogram<u64>,
    remove: Histogram<u64>,
}

impl Stats {
    fn new() -> Self {
        Self {
            insert: Histogram::new(3).unwrap(),
            get: Histogram::new(3).unwrap(),
            remove: Histogram::new(3).unwrap(),
        }
    }

    fn print(&self, name: &str) {
        println!("{}:", name);
        println!(
            "  INSERT:  p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.insert.value_at_quantile(0.50),
            self.insert.value_at_quantile(0.99),
            self.insert.value_at_quantile(0.999),
            self.insert.max(),
            self.insert.len()
        );
        println!(
            "  GET:     p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.get.value_at_quantile(0.50),
            self.get.value_at_quantile(0.99),
            self.get.value_at_quantile(0.999),
            self.get.max(),
            self.get.len()
        );
        println!(
            "  REMOVE:  p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.remove.value_at_quantile(0.50),
            self.remove.value_at_quantile(0.99),
            self.remove.value_at_quantile(0.999),
            self.remove.max(),
            self.remove.len()
        );
    }
}

fn bench_bounded_slab() -> Stats {
    let mut slab: nexus_slab::BoundedSlab<u64> = nexus_slab::BoundedSlab::with_capacity(CAPACITY);

    let mut stats = Stats::new();
    let mut rng = Xorshift::new(SEED);
    let mut keys: Vec<nexus_slab::Key> = Vec::with_capacity(CAPACITY);

    // Warm up: fill to ~50% capacity
    let warmup_target = CAPACITY / 2;
    for i in 0..warmup_target {
        let key = slab.try_insert(i as u64).unwrap();
        keys.push(key);
    }

    // Mixed operations
    for _ in 0..OPERATIONS {
        let op = rng.next() % 10;

        if op < 3 && slab.len() < CAPACITY - 1000 {
            // Insert (30%)
            let start = rdtscp();
            let key = slab.try_insert(rng.next()).unwrap();
            let end = rdtscp();
            let _ = stats.insert.record(end.wrapping_sub(start));
            keys.push(key);
        } else if op < 8 && !keys.is_empty() {
            // Get (50%)
            let idx = rng.next_usize(keys.len());
            let key = keys[idx];
            let start = rdtscp();
            black_box(slab.get(key));
            let end = rdtscp();
            let _ = stats.get.record(end.wrapping_sub(start));
        } else if !keys.is_empty() {
            // Remove (20%)
            let idx = rng.next_usize(keys.len());
            let key = keys.swap_remove(idx);
            let start = rdtscp();
            black_box(slab.remove(key));
            let end = rdtscp();
            let _ = stats.remove.record(end.wrapping_sub(start));
        }
    }

    stats
}

fn bench_bounded_slab_unchecked() -> Stats {
    let mut slab: nexus_slab::BoundedSlab<u64> = nexus_slab::BoundedSlab::with_capacity(CAPACITY);

    let mut stats = Stats::new();
    let mut rng = Xorshift::new(SEED);
    let mut keys: Vec<nexus_slab::Key> = Vec::with_capacity(CAPACITY);

    // Warm up: fill to ~50% capacity
    let warmup_target = CAPACITY / 2;
    for i in 0..warmup_target {
        let key = slab.try_insert(i as u64).unwrap();
        keys.push(key);
    }

    // Mixed operations - using unchecked where possible
    for _ in 0..OPERATIONS {
        let op = rng.next() % 10;

        if op < 3 && slab.len() < CAPACITY - 1000 {
            // Insert (30%)
            let start = rdtscp();
            let key = slab.try_insert(rng.next()).unwrap();
            let end = rdtscp();
            let _ = stats.insert.record(end.wrapping_sub(start));
            keys.push(key);
        } else if op < 8 && !keys.is_empty() {
            // Get unchecked (50%)
            let idx = rng.next_usize(keys.len());
            let key = keys[idx];
            let start = rdtscp();
            black_box(unsafe { slab.get_unchecked(key) });
            let end = rdtscp();
            let _ = stats.get.record(end.wrapping_sub(start));
        } else if !keys.is_empty() {
            // Remove unchecked (20%)
            let idx = rng.next_usize(keys.len());
            let key = keys.swap_remove(idx);
            let start = rdtscp();
            black_box(unsafe { slab.remove_unchecked(key) });
            let end = rdtscp();
            let _ = stats.remove.record(end.wrapping_sub(start));
        }
    }

    stats
}

fn bench_slab_crate() -> Stats {
    let mut slab = slab::Slab::<u64>::with_capacity(CAPACITY);

    let mut stats = Stats::new();
    let mut rng = Xorshift::new(SEED);
    let mut keys: Vec<usize> = Vec::with_capacity(CAPACITY);

    // Warm up: fill to ~50% capacity
    let warmup_target = CAPACITY / 2;
    for i in 0..warmup_target {
        let key = slab.insert(i as u64);
        keys.push(key);
    }

    // Mixed operations
    for _ in 0..OPERATIONS {
        let op = rng.next() % 10;

        if op < 3 && slab.len() < CAPACITY - 1000 {
            // Insert (30%)
            let start = rdtscp();
            let key = slab.insert(rng.next());
            let end = rdtscp();
            let _ = stats.insert.record(end.wrapping_sub(start));
            keys.push(key);
        } else if op < 8 && !keys.is_empty() {
            // Get (50%)
            let idx = rng.next_usize(keys.len());
            let key = keys[idx];
            let start = rdtscp();
            black_box(slab.get(key));
            let end = rdtscp();
            let _ = stats.get.record(end.wrapping_sub(start));
        } else if !keys.is_empty() {
            // Remove (20%)
            let idx = rng.next_usize(keys.len());
            let key = keys.swap_remove(idx);
            let start = rdtscp();
            black_box(slab.remove(key));
            let end = rdtscp();
            let _ = stats.remove.record(end.wrapping_sub(start));
        }
    }

    stats
}

fn main() {
    println!("BOUNDED SLAB BENCHMARK");
    println!("Capacity: {}, Operations: {}", CAPACITY, OPERATIONS);
    println!("Both slabs pre-allocated - pure operation comparison");
    println!("================================================================\n");

    let bounded = bench_bounded_slab();
    let bounded_unchecked = bench_bounded_slab_unchecked();
    let slab = bench_slab_crate();

    bounded.print("nexus BoundedSlab (checked)");
    println!();
    bounded_unchecked.print("nexus BoundedSlab (unchecked)");
    println!();
    slab.print("slab crate");
    println!();

    println!("================================================================");
    println!("COMPARISON (cycles):");
    println!("----------------------------------------------------------------");
    println!("              bounded    unchecked      slab");
    println!(
        "  INSERT p50:   {:>4}        {:>4}        {:>4}",
        bounded.insert.value_at_quantile(0.50),
        bounded_unchecked.insert.value_at_quantile(0.50),
        slab.insert.value_at_quantile(0.50)
    );
    println!(
        "  INSERT p99:   {:>4}        {:>4}        {:>4}",
        bounded.insert.value_at_quantile(0.99),
        bounded_unchecked.insert.value_at_quantile(0.99),
        slab.insert.value_at_quantile(0.99)
    );
    println!(
        "  GET p50:      {:>4}        {:>4}        {:>4}",
        bounded.get.value_at_quantile(0.50),
        bounded_unchecked.get.value_at_quantile(0.50),
        slab.get.value_at_quantile(0.50)
    );
    println!(
        "  GET p99:      {:>4}        {:>4}        {:>4}",
        bounded.get.value_at_quantile(0.99),
        bounded_unchecked.get.value_at_quantile(0.99),
        slab.get.value_at_quantile(0.99)
    );
    println!(
        "  REMOVE p50:   {:>4}        {:>4}        {:>4}",
        bounded.remove.value_at_quantile(0.50),
        bounded_unchecked.remove.value_at_quantile(0.50),
        slab.remove.value_at_quantile(0.50)
    );
    println!(
        "  REMOVE p99:   {:>4}        {:>4}        {:>4}",
        bounded.remove.value_at_quantile(0.99),
        bounded_unchecked.remove.value_at_quantile(0.99),
        slab.remove.value_at_quantile(0.99)
    );

    println!();
    println!("NOTE: BoundedSlab has generational keys (ABA protection)");
    println!("      slab crate does not - stale keys return wrong values");
}
