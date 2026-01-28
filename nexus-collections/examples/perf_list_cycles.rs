//! List benchmark: measure push, pop, remove, and link operations.
//!
//! Run with:
//!   cargo build --release --example perf_list_cycles
//!   taskset -c 0 ./target/release/examples/perf_list_cycles

use hdrhistogram::Histogram;
use std::hint::black_box;

use nexus_collections::{BoxedListStorage, List};

const CAPACITY: usize = 100_000;
const OPERATIONS: usize = 500_000;
const SEED: u64 = 0xDEADBEEF;

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
    push_back: Histogram<u64>,
    push_front: Histogram<u64>,
    pop_front: Histogram<u64>,
    pop_back: Histogram<u64>,
    remove: Histogram<u64>,
    get: Histogram<u64>,
}

impl Stats {
    fn new() -> Self {
        Self {
            push_back: Histogram::new(3).unwrap(),
            push_front: Histogram::new(3).unwrap(),
            pop_front: Histogram::new(3).unwrap(),
            pop_back: Histogram::new(3).unwrap(),
            remove: Histogram::new(3).unwrap(),
            get: Histogram::new(3).unwrap(),
        }
    }

    fn print(&self, name: &str) {
        println!("{}:", name);
        println!(
            "  PUSH_BACK:  p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.push_back.value_at_quantile(0.50),
            self.push_back.value_at_quantile(0.99),
            self.push_back.value_at_quantile(0.999),
            self.push_back.max(),
            self.push_back.len()
        );
        println!(
            "  PUSH_FRONT: p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.push_front.value_at_quantile(0.50),
            self.push_front.value_at_quantile(0.99),
            self.push_front.value_at_quantile(0.999),
            self.push_front.max(),
            self.push_front.len()
        );
        println!(
            "  POP_FRONT:  p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.pop_front.value_at_quantile(0.50),
            self.pop_front.value_at_quantile(0.99),
            self.pop_front.value_at_quantile(0.999),
            self.pop_front.max(),
            self.pop_front.len()
        );
        println!(
            "  POP_BACK:   p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.pop_back.value_at_quantile(0.50),
            self.pop_back.value_at_quantile(0.99),
            self.pop_back.value_at_quantile(0.999),
            self.pop_back.max(),
            self.pop_back.len()
        );
        println!(
            "  REMOVE:     p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.remove.value_at_quantile(0.50),
            self.remove.value_at_quantile(0.99),
            self.remove.value_at_quantile(0.999),
            self.remove.max(),
            self.remove.len()
        );
        println!(
            "  GET:        p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.get.value_at_quantile(0.50),
            self.get.value_at_quantile(0.99),
            self.get.value_at_quantile(0.999),
            self.get.max(),
            self.get.len()
        );
    }
}

fn bench_list_individual_ops() -> Stats {
    let mut storage: BoxedListStorage<u64> = BoxedListStorage::with_capacity(CAPACITY);
    let mut list: List<u64, BoxedListStorage<u64>, usize> = List::new();

    let mut stats = Stats::new();

    // Benchmark push_back
    for i in 0..OPERATIONS {
        let start = rdtscp();
        let _ = list.try_push_back(&mut storage, i as u64);
        let end = rdtscp();
        let _ = stats.push_back.record(end.wrapping_sub(start));

        // Keep list at reasonable size
        if list.len() > CAPACITY / 2 {
            list.pop_front(&mut storage);
        }
    }

    // Reset
    while list.pop_front(&mut storage).is_some() {}

    // Benchmark push_front
    for i in 0..OPERATIONS {
        let start = rdtscp();
        let _ = list.try_push_front(&mut storage, i as u64);
        let end = rdtscp();
        let _ = stats.push_front.record(end.wrapping_sub(start));

        if list.len() > CAPACITY / 2 {
            list.pop_back(&mut storage);
        }
    }

    // Fill list for pop/remove benchmarks
    let mut keys = Vec::with_capacity(CAPACITY / 2);
    for i in 0..(CAPACITY / 2) {
        if let Ok(key) = list.try_push_back(&mut storage, i as u64) {
            keys.push(key);
        }
    }

    // Benchmark pop_front
    for _ in 0..OPERATIONS.min(keys.len()) {
        // Ensure list has elements
        if list.is_empty() {
            for i in 0..1000 {
                let _ = list.try_push_back(&mut storage, i as u64);
            }
        }

        let start = rdtscp();
        black_box(list.pop_front(&mut storage));
        let end = rdtscp();
        let _ = stats.pop_front.record(end.wrapping_sub(start));
    }

    // Refill for pop_back
    while list.pop_front(&mut storage).is_some() {}
    for i in 0..(CAPACITY / 2) {
        let _ = list.try_push_back(&mut storage, i as u64);
    }

    // Benchmark pop_back
    for _ in 0..OPERATIONS.min(CAPACITY / 2) {
        if list.is_empty() {
            for i in 0..1000 {
                let _ = list.try_push_back(&mut storage, i as u64);
            }
        }

        let start = rdtscp();
        black_box(list.pop_back(&mut storage));
        let end = rdtscp();
        let _ = stats.pop_back.record(end.wrapping_sub(start));
    }

    stats
}

fn bench_list_mixed() -> Stats {
    let mut storage: BoxedListStorage<u64> = BoxedListStorage::with_capacity(CAPACITY);
    let mut list: List<u64, BoxedListStorage<u64>, usize> = List::new();

    let mut stats = Stats::new();
    let mut rng = Xorshift::new(SEED);
    let mut keys: Vec<usize> = Vec::with_capacity(CAPACITY);

    // Warm up: fill to ~50% capacity
    for i in 0..(CAPACITY / 2) {
        if let Ok(key) = list.try_push_back(&mut storage, i as u64) {
            keys.push(key);
        }
    }

    // Mixed operations
    for _ in 0..OPERATIONS {
        let op = rng.next() % 100;

        if op < 20 && list.len() < CAPACITY - 1000 {
            // Push back (20%)
            let start = rdtscp();
            if let Ok(key) = list.try_push_back(&mut storage, rng.next()) {
                let end = rdtscp();
                let _ = stats.push_back.record(end.wrapping_sub(start));
                keys.push(key);
            }
        } else if op < 25 && list.len() < CAPACITY - 1000 {
            // Push front (5%)
            let start = rdtscp();
            if let Ok(key) = list.try_push_front(&mut storage, rng.next()) {
                let end = rdtscp();
                let _ = stats.push_front.record(end.wrapping_sub(start));
                keys.push(key);
            }
        } else if op < 40 && !list.is_empty() {
            // Pop front (15%)
            let start = rdtscp();
            if let Some(_) = list.pop_front(&mut storage) {
                let end = rdtscp();
                let _ = stats.pop_front.record(end.wrapping_sub(start));
                // Remove corresponding key (it's the first one that's still valid)
                if !keys.is_empty() {
                    keys.remove(0);
                }
            }
        } else if op < 50 && !list.is_empty() {
            // Pop back (10%)
            let start = rdtscp();
            if let Some(_) = list.pop_back(&mut storage) {
                let end = rdtscp();
                let _ = stats.pop_back.record(end.wrapping_sub(start));
                keys.pop();
            }
        } else if op < 65 && keys.len() > 1 {
            // Remove from middle (15%)
            let idx = rng.next_usize(keys.len().saturating_sub(2)) + 1; // Avoid first/last
            let key = keys[idx];
            let start = rdtscp();
            if list.remove(&mut storage, key).is_some() {
                let end = rdtscp();
                let _ = stats.remove.record(end.wrapping_sub(start));
                keys.remove(idx);
            }
        } else if !keys.is_empty() {
            // Get (35%)
            let idx = rng.next_usize(keys.len());
            let key = keys[idx];
            let start = rdtscp();
            black_box(list.get(&storage, key));
            let end = rdtscp();
            let _ = stats.get.record(end.wrapping_sub(start));
        }
    }

    stats
}

fn main() {
    println!("LIST BENCHMARK (BoxedStorage)");
    println!("Capacity: {}, Operations: {}", CAPACITY, OPERATIONS);
    println!("================================================================\n");

    let individual = bench_list_individual_ops();
    individual.print("Individual Operations");
    println!();

    let mixed = bench_list_mixed();
    mixed.print("Mixed Workload");
    println!();

    println!("================================================================");
    println!("SUMMARY (p50 cycles):");
    println!("----------------------------------------------------------------");
    println!(
        "  PUSH_BACK:  {:>4}  (individual)  {:>4}  (mixed)",
        individual.push_back.value_at_quantile(0.50),
        mixed.push_back.value_at_quantile(0.50)
    );
    println!(
        "  PUSH_FRONT: {:>4}  (individual)  {:>4}  (mixed)",
        individual.push_front.value_at_quantile(0.50),
        mixed.push_front.value_at_quantile(0.50)
    );
    println!(
        "  POP_FRONT:  {:>4}  (individual)  {:>4}  (mixed)",
        individual.pop_front.value_at_quantile(0.50),
        mixed.pop_front.value_at_quantile(0.50)
    );
    println!(
        "  POP_BACK:   {:>4}  (individual)  {:>4}  (mixed)",
        individual.pop_back.value_at_quantile(0.50),
        mixed.pop_back.value_at_quantile(0.50)
    );
    println!(
        "  REMOVE:     {:>4}  (mixed only)",
        mixed.remove.value_at_quantile(0.50)
    );
    println!(
        "  GET:        {:>4}  (mixed only)",
        mixed.get.value_at_quantile(0.50)
    );
}
