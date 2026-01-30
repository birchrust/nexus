//! Access method latency comparison.
//!
//! Benchmarks access methods:
//! - Entry::get() - direct access (safe, Entry owns slot)
//! - get_by_key() - unsafe key-based access
//! - contains_key() - validity check
//!
//! Run with:
//!   cargo build --release --example perf_access_methods
//!   taskset -c 0 ./target/release/examples/perf_access_methods

use hdrhistogram::Histogram;
use std::hint::black_box;

use nexus_slab::{Key, bounded};

const CAPACITY: usize = 100_000;
const OPS: usize = 1_000_000;
const SEED: u64 = 0xCAFEBABE;

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
}

fn print_stats(name: &str, hist: &Histogram<u64>) {
    println!(
        "  {:30} p50={:>4}  p99={:>4}  p999={:>5}  avg={:>4.0}",
        name,
        hist.value_at_quantile(0.50),
        hist.value_at_quantile(0.99),
        hist.value_at_quantile(0.999),
        hist.mean()
    );
}

fn generate_random_indices(count: usize, max: usize, seed: u64) -> Vec<usize> {
    let mut rng = Xorshift::new(seed);
    (0..count).map(|_| (rng.next() as usize) % max).collect()
}

fn main() {
    let slab: bounded::Slab<u64> = bounded::Slab::with_capacity(CAPACITY);

    // Fill the slab - forget entries to keep them alive
    let keys: Vec<Key> = (0..CAPACITY as u64)
        .map(|i| slab.try_insert(i).unwrap().forget())
        .collect();

    let indices = generate_random_indices(OPS, CAPACITY, SEED);

    println!("ACCESS METHOD LATENCY (bounded::Slab, {} random ops)", OPS);
    println!("============================================================\n");

    // Warmup
    for &idx in indices.iter().take(10_000) {
        // SAFETY: key is valid
        black_box(unsafe { slab.get_by_key(keys[idx]) });
    }

    // 1. get_by_key - unsafe key-based access
    let mut hist = Histogram::<u64>::new(3).unwrap();
    for &idx in &indices {
        let key = keys[idx];
        let start = rdtscp();
        // SAFETY: key is valid
        black_box(unsafe { slab.get_by_key(key) });
        let end = rdtscp();
        let _ = hist.record(end.wrapping_sub(start));
    }
    print_stats("get_by_key() [unsafe]", &hist);

    // 2. contains_key - just validity check
    let mut hist = Histogram::<u64>::new(3).unwrap();
    for &idx in &indices {
        let key = keys[idx];
        let start = rdtscp();
        black_box(slab.contains_key(key));
        let end = rdtscp();
        let _ = hist.record(end.wrapping_sub(start));
    }
    print_stats("contains_key()", &hist);

    // 3. Entry::get() via re-acquiring entry
    // This is the recommended pattern: acquire entry, use it, let it drop
    let mut hist = Histogram::<u64>::new(3).unwrap();
    for &idx in &indices {
        let key = keys[idx];
        let start = rdtscp();
        // Re-acquire entry and access
        if let Some(entry) = slab.entry(key) {
            black_box(entry.get());
            entry.forget(); // Don't drop, keep data alive
        }
        let end = rdtscp();
        let _ = hist.record(end.wrapping_sub(start));
    }
    print_stats("entry(key)?.get() [safe]", &hist);

    println!();
    println!("------------------------------------------------------------");
    println!("Legend:");
    println!("  get_by_key() [unsafe]  - direct access via key, caller ensures validity");
    println!("  contains_key()         - validity check only, no value access");
    println!("  entry(key)?.get()      - safe pattern: acquire Entry, then direct access");
}
