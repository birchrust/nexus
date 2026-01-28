//! SkipList benchmark: measure insert, get, remove, first, last operations.
//!
//! Run with:
//!   cargo build --release --example perf_skiplist_cycles
//!   taskset -c 0 ./target/release/examples/perf_skiplist_cycles

use hdrhistogram::Histogram;
use std::hint::black_box;

use nexus_collections::{BoxedSkipStorage, SkipList};
use rand::rngs::SmallRng;
use rand::SeedableRng;

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
    insert: Histogram<u64>,
    get: Histogram<u64>,
    remove: Histogram<u64>,
    first: Histogram<u64>,
    last: Histogram<u64>,
    pop_first: Histogram<u64>,
}

impl Stats {
    fn new() -> Self {
        Self {
            insert: Histogram::new(3).unwrap(),
            get: Histogram::new(3).unwrap(),
            remove: Histogram::new(3).unwrap(),
            first: Histogram::new(3).unwrap(),
            last: Histogram::new(3).unwrap(),
            pop_first: Histogram::new(3).unwrap(),
        }
    }

    fn print(&self, name: &str) {
        println!("{}:", name);
        println!(
            "  INSERT:    p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.insert.value_at_quantile(0.50),
            self.insert.value_at_quantile(0.99),
            self.insert.value_at_quantile(0.999),
            self.insert.max(),
            self.insert.len()
        );
        println!(
            "  GET:       p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.get.value_at_quantile(0.50),
            self.get.value_at_quantile(0.99),
            self.get.value_at_quantile(0.999),
            self.get.max(),
            self.get.len()
        );
        println!(
            "  REMOVE:    p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.remove.value_at_quantile(0.50),
            self.remove.value_at_quantile(0.99),
            self.remove.value_at_quantile(0.999),
            self.remove.max(),
            self.remove.len()
        );
        println!(
            "  FIRST:     p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.first.value_at_quantile(0.50),
            self.first.value_at_quantile(0.99),
            self.first.value_at_quantile(0.999),
            self.first.max(),
            self.first.len()
        );
        println!(
            "  LAST:      p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.last.value_at_quantile(0.50),
            self.last.value_at_quantile(0.99),
            self.last.value_at_quantile(0.999),
            self.last.max(),
            self.last.len()
        );
        println!(
            "  POP_FIRST: p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.pop_first.value_at_quantile(0.50),
            self.pop_first.value_at_quantile(0.99),
            self.pop_first.value_at_quantile(0.999),
            self.pop_first.max(),
            self.pop_first.len()
        );
    }
}

fn bench_skiplist_individual_ops() -> Stats {
    let mut storage: BoxedSkipStorage<u64, u64> = BoxedSkipStorage::with_capacity(CAPACITY);
    let rng = SmallRng::seed_from_u64(SEED);
    let mut skiplist: SkipList<u64, u64, BoxedSkipStorage<u64, u64>, usize, SmallRng, 16> =
        SkipList::new(rng);

    let mut stats = Stats::new();
    let mut data_rng = Xorshift::new(SEED);

    // Benchmark insert (O(log n))
    for _ in 0..OPERATIONS {
        let key = data_rng.next();
        let value = data_rng.next();
        let start = rdtscp();
        let _ = skiplist.try_insert(&mut storage, key, value);
        let end = rdtscp();
        let _ = stats.insert.record(end.wrapping_sub(start));

        // Keep skiplist at reasonable size
        if skiplist.len() > CAPACITY / 2 {
            skiplist.pop_first(&mut storage);
        }
    }

    // Reset and fill for get/first/last benchmarks
    while skiplist.pop_first(&mut storage).is_some() {}
    let mut keys: Vec<u64> = Vec::with_capacity(CAPACITY / 2);
    for _ in 0..(CAPACITY / 2) {
        let key = data_rng.next();
        if skiplist.try_insert(&mut storage, key, data_rng.next()).is_ok() {
            keys.push(key);
        }
    }

    // Benchmark get (O(log n))
    for _ in 0..OPERATIONS {
        let idx = data_rng.next_usize(keys.len());
        let key = keys[idx];
        let start = rdtscp();
        black_box(skiplist.get(&storage, &key));
        let end = rdtscp();
        let _ = stats.get.record(end.wrapping_sub(start));
    }

    // Benchmark first (O(1))
    for _ in 0..OPERATIONS {
        let start = rdtscp();
        black_box(skiplist.first(&storage));
        let end = rdtscp();
        let _ = stats.first.record(end.wrapping_sub(start));
    }

    // Benchmark last (O(1))
    for _ in 0..OPERATIONS {
        let start = rdtscp();
        black_box(skiplist.last(&storage));
        let end = rdtscp();
        let _ = stats.last.record(end.wrapping_sub(start));
    }

    // Benchmark pop_first (O(1))
    for _ in 0..OPERATIONS {
        if skiplist.is_empty() {
            // Refill
            for _ in 0..1000 {
                let _ = skiplist.try_insert(&mut storage, data_rng.next(), data_rng.next());
            }
        }

        let start = rdtscp();
        black_box(skiplist.pop_first(&mut storage));
        let end = rdtscp();
        let _ = stats.pop_first.record(end.wrapping_sub(start));
    }

    stats
}

fn bench_skiplist_mixed() -> Stats {
    let mut storage: BoxedSkipStorage<u64, u64> = BoxedSkipStorage::with_capacity(CAPACITY);
    let rng = SmallRng::seed_from_u64(SEED);
    let mut skiplist: SkipList<u64, u64, BoxedSkipStorage<u64, u64>, usize, SmallRng, 16> =
        SkipList::new(rng);

    let mut stats = Stats::new();
    let mut data_rng = Xorshift::new(SEED);
    let mut keys: Vec<u64> = Vec::with_capacity(CAPACITY);

    // Warm up: fill to ~50% capacity
    for _ in 0..(CAPACITY / 2) {
        let key = data_rng.next() % 10_000_000;
        if skiplist.try_insert(&mut storage, key, data_rng.next()).is_ok() {
            keys.push(key);
        }
    }

    // Mixed operations
    for _ in 0..OPERATIONS {
        let op = data_rng.next() % 100;

        if op < 20 && skiplist.len() < CAPACITY - 1000 {
            // Insert (20%)
            let key = data_rng.next() % 10_000_000;
            let value = data_rng.next();
            let start = rdtscp();
            if skiplist.try_insert(&mut storage, key, value).is_ok() {
                let end = rdtscp();
                let _ = stats.insert.record(end.wrapping_sub(start));
                keys.push(key);
            }
        } else if op < 50 && !keys.is_empty() {
            // Get (30%)
            let idx = data_rng.next_usize(keys.len());
            let key = keys[idx];
            let start = rdtscp();
            black_box(skiplist.get(&storage, &key));
            let end = rdtscp();
            let _ = stats.get.record(end.wrapping_sub(start));
        } else if op < 65 {
            // First (15%)
            let start = rdtscp();
            black_box(skiplist.first(&storage));
            let end = rdtscp();
            let _ = stats.first.record(end.wrapping_sub(start));
        } else if op < 80 {
            // Last (15%)
            let start = rdtscp();
            black_box(skiplist.last(&storage));
            let end = rdtscp();
            let _ = stats.last.record(end.wrapping_sub(start));
        } else if op < 90 && !keys.is_empty() {
            // Remove by key (10%)
            let idx = data_rng.next_usize(keys.len());
            let key = keys.swap_remove(idx);
            let start = rdtscp();
            if skiplist.remove(&mut storage, &key).is_some() {
                let end = rdtscp();
                let _ = stats.remove.record(end.wrapping_sub(start));
            }
        } else if !skiplist.is_empty() {
            // Pop first (10%)
            let start = rdtscp();
            if let Some((k, _)) = skiplist.pop_first(&mut storage) {
                let end = rdtscp();
                let _ = stats.pop_first.record(end.wrapping_sub(start));
                // Remove from keys tracking
                if let Some(pos) = keys.iter().position(|&x| x == k) {
                    keys.swap_remove(pos);
                }
            }
        }
    }

    stats
}

fn main() {
    println!("SKIPLIST BENCHMARK (BoxedStorage)");
    println!("Capacity: {}, Operations: {}", CAPACITY, OPERATIONS);
    println!("================================================================\n");

    let individual = bench_skiplist_individual_ops();
    individual.print("Individual Operations");
    println!();

    let mixed = bench_skiplist_mixed();
    mixed.print("Mixed Workload");
    println!();

    println!("================================================================");
    println!("SUMMARY (p50 cycles):");
    println!("----------------------------------------------------------------");
    println!(
        "  INSERT:    {:>4}  (individual)  {:>4}  (mixed)   O(log n)",
        individual.insert.value_at_quantile(0.50),
        mixed.insert.value_at_quantile(0.50)
    );
    println!(
        "  GET:       {:>4}  (individual)  {:>4}  (mixed)   O(log n)",
        individual.get.value_at_quantile(0.50),
        mixed.get.value_at_quantile(0.50)
    );
    println!(
        "  REMOVE:                       {:>4}  (mixed)   O(log n)",
        mixed.remove.value_at_quantile(0.50)
    );
    println!(
        "  FIRST:     {:>4}  (individual)  {:>4}  (mixed)   O(1)",
        individual.first.value_at_quantile(0.50),
        mixed.first.value_at_quantile(0.50)
    );
    println!(
        "  LAST:      {:>4}  (individual)  {:>4}  (mixed)   O(1)",
        individual.last.value_at_quantile(0.50),
        mixed.last.value_at_quantile(0.50)
    );
    println!(
        "  POP_FIRST: {:>4}  (individual)  {:>4}  (mixed)   O(1)",
        individual.pop_first.value_at_quantile(0.50),
        mixed.pop_first.value_at_quantile(0.50)
    );
}
