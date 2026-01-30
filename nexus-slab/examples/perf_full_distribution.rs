//! Full latency distribution comparison: bounded vs unbounded vs slab crate.
//!
//! Shows p1 through p99.99 for insert, get, and remove operations.
//!
//! Run with:
//!   cargo build --release --example perf_full_distribution
//!   taskset -c 0 ./target/release/examples/perf_full_distribution

use hdrhistogram::Histogram;
use std::hint::black_box;

use nexus_slab::{Key, bounded, unbounded};

const CAPACITY: usize = 100_000;
const OPS: usize = 500_000;

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

struct Stats {
    insert: Histogram<u64>,
    // Entry-based access (safe, owns slot)
    entry_get: Histogram<u64>,
    // Key-based access (unsafe)
    get_by_key: Histogram<u64>,
    // Validity check
    contains_key: Histogram<u64>,
    // Remove operations
    entry_remove: Histogram<u64>,
    remove_by_key: Histogram<u64>,
}

impl Stats {
    fn new() -> Self {
        Self {
            insert: Histogram::new(3).unwrap(),
            entry_get: Histogram::new(3).unwrap(),
            get_by_key: Histogram::new(3).unwrap(),
            contains_key: Histogram::new(3).unwrap(),
            entry_remove: Histogram::new(3).unwrap(),
            remove_by_key: Histogram::new(3).unwrap(),
        }
    }
}

struct SlabStats {
    insert: Histogram<u64>,
    get: Histogram<u64>,
    remove: Histogram<u64>,
}

impl SlabStats {
    fn new() -> Self {
        Self {
            insert: Histogram::new(3).unwrap(),
            get: Histogram::new(3).unwrap(),
            remove: Histogram::new(3).unwrap(),
        }
    }
}

fn bench_bounded() -> Stats {
    let slab = bounded::Slab::<u64>::with_capacity(CAPACITY);
    let mut stats = Stats::new();
    let mut entries: Vec<bounded::Entry<u64>> = Vec::with_capacity(CAPACITY);
    let mut keys: Vec<Key> = Vec::with_capacity(CAPACITY);

    // Warmup
    for i in 0..1000u64 {
        let entry = slab.try_insert(i).unwrap();
        keys.push(entry.forget());
    }
    for key in keys.drain(..) {
        // SAFETY: key is valid
        let _ = unsafe { slab.remove_by_key(key) };
    }

    // Measured operations
    for i in 0..OPS as u64 {
        // Insert
        let start = rdtscp();
        let entry = slab.try_insert(i).unwrap();
        let end = rdtscp();
        let _ = stats.insert.record(end.wrapping_sub(start));

        // Decide: keep as entry or forget to key (alternate)
        if i % 2 == 0 {
            entries.push(entry);
        } else {
            keys.push(entry.forget());
        }

        // Key-based gets (unsafe)
        if !keys.is_empty() {
            let idx = (i as usize * 7) % keys.len();
            let key = keys[idx];

            let start = rdtscp();
            // SAFETY: key is valid
            let val = unsafe { slab.get_by_key(key) };
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_by_key.record(end.wrapping_sub(start));

            // contains_key check
            let start = rdtscp();
            let valid = slab.contains_key(key);
            black_box(valid);
            let end = rdtscp();
            let _ = stats.contains_key.record(end.wrapping_sub(start));
        }

        // Entry-based gets (safe)
        if !entries.is_empty() {
            let idx = (i as usize * 11) % entries.len();
            let entry = &entries[idx];

            let start = rdtscp();
            let val = entry.get();
            black_box(val);
            let end = rdtscp();
            let _ = stats.entry_get.record(end.wrapping_sub(start));
        }

        // Remove periodically to maintain steady state
        if entries.len() + keys.len() > CAPACITY / 2 {
            match i % 2 {
                0 if !entries.is_empty() => {
                    let entry = entries.pop().unwrap();

                    let start = rdtscp();
                    let val = entry.remove();
                    black_box(val);
                    let end = rdtscp();
                    let _ = stats.entry_remove.record(end.wrapping_sub(start));
                }
                1 if !keys.is_empty() => {
                    let key = keys.pop().unwrap();

                    let start = rdtscp();
                    // SAFETY: key is valid
                    let val = unsafe { slab.remove_by_key(key) };
                    black_box(val);
                    let end = rdtscp();
                    let _ = stats.remove_by_key.record(end.wrapping_sub(start));
                }
                _ => {}
            }
        }
    }

    // Cleanup
    for entry in entries {
        entry.remove();
    }
    for key in keys {
        // SAFETY: key is valid
        unsafe { slab.remove_by_key(key) };
    }

    stats
}

fn bench_unbounded() -> Stats {
    let slab = unbounded::Slab::<u64>::with_capacity(CAPACITY);
    let mut stats = Stats::new();
    let mut entries: Vec<unbounded::Entry<u64>> = Vec::with_capacity(CAPACITY);
    let mut keys: Vec<Key> = Vec::with_capacity(CAPACITY);

    // Warmup
    for i in 0..1000u64 {
        let entry = slab.insert(i);
        keys.push(entry.forget());
    }
    for key in keys.drain(..) {
        // SAFETY: key is valid
        let _ = unsafe { slab.remove_by_key(key) };
    }

    // Measured operations
    for i in 0..OPS as u64 {
        // Insert
        let start = rdtscp();
        let entry = slab.insert(i);
        let end = rdtscp();
        let _ = stats.insert.record(end.wrapping_sub(start));

        if i % 2 == 0 {
            entries.push(entry);
        } else {
            keys.push(entry.forget());
        }

        // Key-based gets (unsafe)
        if !keys.is_empty() {
            let idx = (i as usize * 7) % keys.len();
            let key = keys[idx];

            let start = rdtscp();
            // SAFETY: key is valid
            let val = unsafe { slab.get_by_key(key) };
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_by_key.record(end.wrapping_sub(start));

            // contains_key check
            let start = rdtscp();
            let valid = slab.contains_key(key);
            black_box(valid);
            let end = rdtscp();
            let _ = stats.contains_key.record(end.wrapping_sub(start));
        }

        // Entry-based gets (safe)
        if !entries.is_empty() {
            let idx = (i as usize * 11) % entries.len();
            let entry = &entries[idx];

            let start = rdtscp();
            let val = entry.get();
            black_box(val);
            let end = rdtscp();
            let _ = stats.entry_get.record(end.wrapping_sub(start));
        }

        // Remove periodically
        if entries.len() + keys.len() > CAPACITY / 2 {
            match i % 2 {
                0 if !entries.is_empty() => {
                    let entry = entries.pop().unwrap();

                    let start = rdtscp();
                    let val = entry.remove();
                    black_box(val);
                    let end = rdtscp();
                    let _ = stats.entry_remove.record(end.wrapping_sub(start));
                }
                1 if !keys.is_empty() => {
                    let key = keys.pop().unwrap();

                    let start = rdtscp();
                    // SAFETY: key is valid
                    let val = unsafe { slab.remove_by_key(key) };
                    black_box(val);
                    let end = rdtscp();
                    let _ = stats.remove_by_key.record(end.wrapping_sub(start));
                }
                _ => {}
            }
        }
    }

    // Cleanup
    for entry in entries {
        entry.remove();
    }
    for key in keys {
        // SAFETY: key is valid
        unsafe { slab.remove_by_key(key) };
    }

    stats
}

fn bench_slab_crate() -> SlabStats {
    let mut slab = slab::Slab::<u64>::with_capacity(CAPACITY);
    let mut stats = SlabStats::new();
    let mut keys: Vec<usize> = Vec::with_capacity(CAPACITY);

    // Warmup
    for i in 0..1000u64 {
        keys.push(slab.insert(i));
    }
    for key in keys.drain(..) {
        slab.remove(key);
    }

    // Measured operations
    for i in 0..OPS as u64 {
        // Insert
        let start = rdtscp();
        let key = slab.insert(i);
        let end = rdtscp();
        let _ = stats.insert.record(end.wrapping_sub(start));
        keys.push(key);

        // Get
        if !keys.is_empty() {
            let idx = (i as usize * 7) % keys.len();
            let key = keys[idx];

            let start = rdtscp();
            let val = slab.get(key);
            black_box(val);
            let end = rdtscp();
            let _ = stats.get.record(end.wrapping_sub(start));
        }

        // Remove periodically
        if keys.len() > CAPACITY / 2 {
            let key = keys.pop().unwrap();

            let start = rdtscp();
            let val = slab.remove(key);
            black_box(val);
            let end = rdtscp();
            let _ = stats.remove.record(end.wrapping_sub(start));
        }
    }

    stats
}

fn print_hist(name: &str, hist: &Histogram<u64>) {
    println!(
        "  {:25} p50={:>4}  p99={:>4}  p999={:>5}  max={:>6}",
        name,
        hist.value_at_quantile(0.50),
        hist.value_at_quantile(0.99),
        hist.value_at_quantile(0.999),
        hist.max()
    );
}

fn main() {
    println!("FULL LATENCY DISTRIBUTION ({} ops)", OPS);
    println!("===========================================\n");

    println!("bounded::Slab");
    println!("-------------");
    let bounded_stats = bench_bounded();
    print_hist("insert", &bounded_stats.insert);
    print_hist("entry.get() [safe]", &bounded_stats.entry_get);
    print_hist("get_by_key() [unsafe]", &bounded_stats.get_by_key);
    print_hist("contains_key()", &bounded_stats.contains_key);
    print_hist("entry.remove()", &bounded_stats.entry_remove);
    print_hist("remove_by_key() [unsafe]", &bounded_stats.remove_by_key);
    println!();

    println!("unbounded::Slab");
    println!("---------------");
    let unbounded_stats = bench_unbounded();
    print_hist("insert", &unbounded_stats.insert);
    print_hist("entry.get() [safe]", &unbounded_stats.entry_get);
    print_hist("get_by_key() [unsafe]", &unbounded_stats.get_by_key);
    print_hist("contains_key()", &unbounded_stats.contains_key);
    print_hist("entry.remove()", &unbounded_stats.entry_remove);
    print_hist("remove_by_key() [unsafe]", &unbounded_stats.remove_by_key);
    println!();

    println!("slab crate");
    println!("----------");
    let slab_stats = bench_slab_crate();
    print_hist("insert", &slab_stats.insert);
    print_hist("get", &slab_stats.get);
    print_hist("remove", &slab_stats.remove);
    println!();

    println!("===========================================");
    println!("Legend:");
    println!("  entry.get() [safe]     - direct access via Entry (owns slot)");
    println!("  get_by_key() [unsafe]  - key-based access (caller ensures validity)");
    println!("  contains_key()         - validity check only");
}
