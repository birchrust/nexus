//! Access method latency comparison.
//!
//! Benchmarks all access methods on BoundedSlab:
//! - get_unchecked() - no checks
//! - get_untracked() - validity check only
//! - get() - tracked with Ref guard
//! - UntrackedAccessor[key] - Index syntax
//! - contains_key() - validity check
//!
//! Run with:
//!   cargo build --release --example perf_access_methods
//!   taskset -c 0 ./target/release/examples/perf_access_methods

use hdrhistogram::Histogram;
use std::hint::black_box;

use nexus_slab::{BoundedSlab, Key};

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
    let slab: BoundedSlab<u64> = BoundedSlab::leak(CAPACITY);

    // Fill the slab
    let keys: Vec<Key> = (0..CAPACITY as u64)
        .map(|i| slab.try_insert(i).unwrap().leak())
        .collect();

    let indices = generate_random_indices(OPS, CAPACITY, SEED);

    println!("ACCESS METHOD LATENCY (BoundedSlab, {} random ops)", OPS);
    println!("============================================================\n");

    // Warmup
    for &idx in indices.iter().take(10_000) {
        black_box(unsafe { slab.get_unchecked(keys[idx]) });
    }

    // 1. get_unchecked - no checks at all
    let mut hist = Histogram::<u64>::new(3).unwrap();
    for &idx in &indices {
        let key = keys[idx];
        let start = rdtscp();
        black_box(unsafe { slab.get_unchecked(key) });
        let end = rdtscp();
        let _ = hist.record(end.wrapping_sub(start));
    }
    print_stats("get_unchecked()", &hist);

    // 2. get_untracked - validity check, no borrow tracking
    let mut hist = Histogram::<u64>::new(3).unwrap();
    for &idx in &indices {
        let key = keys[idx];
        let start = rdtscp();
        black_box(unsafe { slab.get_untracked(key) });
        let end = rdtscp();
        let _ = hist.record(end.wrapping_sub(start));
    }
    print_stats("get_untracked()", &hist);

    // 3. get - tracked, returns Ref<T>
    let mut hist = Histogram::<u64>::new(3).unwrap();
    for &idx in &indices {
        let key = keys[idx];
        let start = rdtscp();
        black_box(slab.get(key));
        let end = rdtscp();
        let _ = hist.record(end.wrapping_sub(start));
    }
    print_stats("get() -> Ref<T>", &hist);

    // 4. UntrackedAccessor[key] - Index syntax
    // SAFETY: No Entry operations during benchmark
    let accessor = unsafe { slab.untracked() };
    let mut hist = Histogram::<u64>::new(3).unwrap();
    for &idx in &indices {
        let key = keys[idx];
        let start = rdtscp();
        black_box(accessor[key]);
        let end = rdtscp();
        let _ = hist.record(end.wrapping_sub(start));
    }
    print_stats("UntrackedAccessor[key]", &hist);

    // 5. contains_key - just validity check
    let mut hist = Histogram::<u64>::new(3).unwrap();
    for &idx in &indices {
        let key = keys[idx];
        let start = rdtscp();
        black_box(slab.contains_key(key));
        let end = rdtscp();
        let _ = hist.record(end.wrapping_sub(start));
    }
    print_stats("contains_key()", &hist);

    println!();
    println!("------------------------------------------------------------");
    println!("Legend:");
    println!("  get_unchecked()      - unsafe, no validity/borrow checks");
    println!("  get_untracked()      - unsafe, validity check, no borrow");
    println!("  get() -> Ref<T>      - safe, validity + borrow tracking");
    println!("  UntrackedAccessor    - unsafe wrapper, uses get_unchecked");
    println!("  contains_key()       - validity check only, no value access");
}
