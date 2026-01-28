//! Heap benchmark: measure push, pop, peek, remove, and decrease_key operations.
//!
//! Run with:
//!   cargo build --release --example perf_heap_cycles
//!   taskset -c 0 ./target/release/examples/perf_heap_cycles

use hdrhistogram::Histogram;
use std::hint::black_box;

use nexus_collections::{BoxedHeapStorage, Heap};

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
    push: Histogram<u64>,
    pop: Histogram<u64>,
    peek: Histogram<u64>,
    remove: Histogram<u64>,
    decrease_key: Histogram<u64>,
}

impl Stats {
    fn new() -> Self {
        Self {
            push: Histogram::new(3).unwrap(),
            pop: Histogram::new(3).unwrap(),
            peek: Histogram::new(3).unwrap(),
            remove: Histogram::new(3).unwrap(),
            decrease_key: Histogram::new(3).unwrap(),
        }
    }

    fn print(&self, name: &str) {
        println!("{}:", name);
        println!(
            "  PUSH:         p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.push.value_at_quantile(0.50),
            self.push.value_at_quantile(0.99),
            self.push.value_at_quantile(0.999),
            self.push.max(),
            self.push.len()
        );
        println!(
            "  POP:          p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.pop.value_at_quantile(0.50),
            self.pop.value_at_quantile(0.99),
            self.pop.value_at_quantile(0.999),
            self.pop.max(),
            self.pop.len()
        );
        println!(
            "  PEEK:         p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.peek.value_at_quantile(0.50),
            self.peek.value_at_quantile(0.99),
            self.peek.value_at_quantile(0.999),
            self.peek.max(),
            self.peek.len()
        );
        println!(
            "  REMOVE:       p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.remove.value_at_quantile(0.50),
            self.remove.value_at_quantile(0.99),
            self.remove.value_at_quantile(0.999),
            self.remove.max(),
            self.remove.len()
        );
        println!(
            "  DECREASE_KEY: p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
            self.decrease_key.value_at_quantile(0.50),
            self.decrease_key.value_at_quantile(0.99),
            self.decrease_key.value_at_quantile(0.999),
            self.decrease_key.max(),
            self.decrease_key.len()
        );
    }
}

fn bench_heap_individual_ops() -> Stats {
    let mut storage: BoxedHeapStorage<u64> = BoxedHeapStorage::with_capacity(CAPACITY);
    let mut heap: Heap<u64, BoxedHeapStorage<u64>, usize> = Heap::new();

    let mut stats = Stats::new();
    let mut rng = Xorshift::new(SEED);

    // Benchmark push (O(log n))
    for _ in 0..OPERATIONS {
        let value = rng.next();
        let start = rdtscp();
        let _ = heap.try_push(&mut storage, value);
        let end = rdtscp();
        let _ = stats.push.record(end.wrapping_sub(start));

        // Keep heap at reasonable size
        if heap.len() > CAPACITY / 2 {
            heap.pop(&mut storage);
        }
    }

    // Reset and fill for pop benchmark
    while heap.pop(&mut storage).is_some() {}
    for _ in 0..(CAPACITY / 2) {
        let _ = heap.try_push(&mut storage, rng.next());
    }

    // Benchmark pop (O(log n))
    for _ in 0..OPERATIONS {
        if heap.is_empty() {
            // Refill
            for _ in 0..1000 {
                let _ = heap.try_push(&mut storage, rng.next());
            }
        }

        let start = rdtscp();
        black_box(heap.pop(&mut storage));
        let end = rdtscp();
        let _ = stats.pop.record(end.wrapping_sub(start));
    }

    // Refill for peek benchmark
    while heap.pop(&mut storage).is_some() {}
    for _ in 0..(CAPACITY / 2) {
        let _ = heap.try_push(&mut storage, rng.next());
    }

    // Benchmark peek (O(1))
    for _ in 0..OPERATIONS {
        let start = rdtscp();
        black_box(heap.peek(&storage));
        let end = rdtscp();
        let _ = stats.peek.record(end.wrapping_sub(start));
    }

    stats
}

fn bench_heap_mixed() -> Stats {
    let mut storage: BoxedHeapStorage<u64> = BoxedHeapStorage::with_capacity(CAPACITY);
    let mut heap: Heap<u64, BoxedHeapStorage<u64>, usize> = Heap::new();

    let mut stats = Stats::new();
    let mut rng = Xorshift::new(SEED);
    let mut keys: Vec<usize> = Vec::with_capacity(CAPACITY);

    // Warm up: fill to ~50% capacity
    for _ in 0..(CAPACITY / 2) {
        if let Ok(key) = heap.try_push(&mut storage, rng.next() % 1_000_000) {
            keys.push(key);
        }
    }

    // Mixed operations
    for _ in 0..OPERATIONS {
        let op = rng.next() % 100;

        if op < 25 && heap.len() < CAPACITY - 1000 {
            // Push (25%)
            let value = rng.next() % 1_000_000;
            let start = rdtscp();
            if let Ok(key) = heap.try_push(&mut storage, value) {
                let end = rdtscp();
                let _ = stats.push.record(end.wrapping_sub(start));
                keys.push(key);
            }
        } else if op < 45 && !heap.is_empty() {
            // Pop (20%)
            let start = rdtscp();
            if heap.pop(&mut storage).is_some() {
                let end = rdtscp();
                let _ = stats.pop.record(end.wrapping_sub(start));
                // Note: We don't track which key was popped, so keys vec becomes stale
                // This is fine for benchmarking purposes
            }
        } else if op < 65 {
            // Peek (20%)
            let start = rdtscp();
            black_box(heap.peek(&storage));
            let end = rdtscp();
            let _ = stats.peek.record(end.wrapping_sub(start));
        } else if op < 80 && keys.len() > 10 {
            // Remove by key (15%)
            let idx = rng.next_usize(keys.len());
            let key = keys.swap_remove(idx);
            let start = rdtscp();
            if heap.remove(&mut storage, key).is_some() {
                let end = rdtscp();
                let _ = stats.remove.record(end.wrapping_sub(start));
            }
        } else if !keys.is_empty() {
            // Decrease key (20%) - call decrease_key (measures sift-up overhead)
            // Note: We can't actually modify HeapNode.data from outside the crate,
            // so this measures the decrease_key operation when no reordering is needed.
            let idx = rng.next_usize(keys.len());
            let key = keys[idx];
            let start = rdtscp();
            heap.decrease_key(&mut storage, key);
            let end = rdtscp();
            let _ = stats.decrease_key.record(end.wrapping_sub(start));
        }
    }

    stats
}

fn main() {
    println!("HEAP BENCHMARK (BoxedStorage)");
    println!("Capacity: {}, Operations: {}", CAPACITY, OPERATIONS);
    println!("================================================================\n");

    let individual = bench_heap_individual_ops();
    individual.print("Individual Operations");
    println!();

    let mixed = bench_heap_mixed();
    mixed.print("Mixed Workload");
    println!();

    println!("================================================================");
    println!("SUMMARY (p50 cycles):");
    println!("----------------------------------------------------------------");
    println!(
        "  PUSH:         {:>4}  (individual)  {:>4}  (mixed)   O(log n)",
        individual.push.value_at_quantile(0.50),
        mixed.push.value_at_quantile(0.50)
    );
    println!(
        "  POP:          {:>4}  (individual)  {:>4}  (mixed)   O(log n)",
        individual.pop.value_at_quantile(0.50),
        mixed.pop.value_at_quantile(0.50)
    );
    println!(
        "  PEEK:         {:>4}  (individual)  {:>4}  (mixed)   O(1)",
        individual.peek.value_at_quantile(0.50),
        mixed.peek.value_at_quantile(0.50)
    );
    println!(
        "  REMOVE:                         {:>4}  (mixed)   O(log n)",
        mixed.remove.value_at_quantile(0.50)
    );
    println!(
        "  DECREASE_KEY:                   {:>4}  (mixed)   O(log n)",
        mixed.decrease_key.value_at_quantile(0.50)
    );
}
