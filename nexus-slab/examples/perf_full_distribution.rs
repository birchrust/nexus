//! Full latency distribution comparison: bounded vs unbounded vs slab crate.
//!
//! Shows p1 through p99.99 for insert, get (various access patterns), and remove.

use hdrhistogram::Histogram;
use std::hint::black_box;

use nexus_slab::{bounded, Key, Slab};

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
    // Key-based access (valid)
    get_by_key: Histogram<u64>,
    get_by_key_untracked: Histogram<u64>,
    get_by_key_unchecked: Histogram<u64>,
    // Key-based access (invalid/stale key)
    get_by_key_invalid: Histogram<u64>,
    // UntrackedAccessor indexing
    accessor_index: Histogram<u64>,
    // Entry-based access (valid)
    entry_get: Histogram<u64>,
    entry_get_untracked: Histogram<u64>,
    entry_get_unchecked: Histogram<u64>,
    // Entry-based access (invalid - slot removed via clone)
    entry_try_get_invalid: Histogram<u64>,
    // Remove
    remove_by_key: Histogram<u64>,
    entry_remove: Histogram<u64>,
    entry_remove_unchecked: Histogram<u64>,
}

impl Stats {
    fn new() -> Self {
        Self {
            insert: Histogram::new(3).unwrap(),
            get_by_key: Histogram::new(3).unwrap(),
            get_by_key_untracked: Histogram::new(3).unwrap(),
            get_by_key_unchecked: Histogram::new(3).unwrap(),
            get_by_key_invalid: Histogram::new(3).unwrap(),
            accessor_index: Histogram::new(3).unwrap(),
            entry_get: Histogram::new(3).unwrap(),
            entry_get_untracked: Histogram::new(3).unwrap(),
            entry_get_unchecked: Histogram::new(3).unwrap(),
            entry_try_get_invalid: Histogram::new(3).unwrap(),
            remove_by_key: Histogram::new(3).unwrap(),
            entry_remove: Histogram::new(3).unwrap(),
            entry_remove_unchecked: Histogram::new(3).unwrap(),
        }
    }
}

struct SlabStats {
    insert: Histogram<u64>,
    get: Histogram<u64>,
    get_invalid: Histogram<u64>,
    remove: Histogram<u64>,
}

impl SlabStats {
    fn new() -> Self {
        Self {
            insert: Histogram::new(3).unwrap(),
            get: Histogram::new(3).unwrap(),
            get_invalid: Histogram::new(3).unwrap(),
            remove: Histogram::new(3).unwrap(),
        }
    }
}

fn bench_bounded() -> Stats {
    let slab = bounded::Slab::<u64>::leak(CAPACITY);
    let mut stats = Stats::new();
    let mut entries: Vec<bounded::Entry<u64>> = Vec::with_capacity(CAPACITY);
    let mut keys: Vec<Key> = Vec::with_capacity(CAPACITY);
    let mut stale_keys: Vec<Key> = Vec::with_capacity(1000);
    let mut stale_entries: Vec<bounded::Entry<u64>> = Vec::with_capacity(1000);

    // Warmup
    for i in 0..1000u64 {
        let entry = slab.try_insert(i).unwrap();
        keys.push(entry.leak());
    }
    for key in keys.drain(..) {
        let _ = slab.remove_by_key(key);
    }

    // Measured operations
    for i in 0..OPS as u64 {
        // Insert
        let start = rdtscp();
        let entry = slab.try_insert(i).unwrap();
        let end = rdtscp();
        let _ = stats.insert.record(end.wrapping_sub(start));

        // Decide: keep as entry or leak to key (alternate)
        if i % 2 == 0 {
            entries.push(entry);
        } else {
            keys.push(entry.leak());
        }

        // Key-based gets (valid)
        if !keys.is_empty() {
            let idx = (i as usize * 7) % keys.len();
            let key = keys[idx];

            // Checked (returns Option<Ref<T>>)
            let start = rdtscp();
            let val = slab.get(key);
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_by_key.record(end.wrapping_sub(start));

            // Untracked (unsafe, no borrow tracking)
            let start = rdtscp();
            let val = unsafe { slab.get_untracked(key) };
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_by_key_untracked.record(end.wrapping_sub(start));

            // Unchecked (unsafe, no validity check)
            let start = rdtscp();
            let val = unsafe { slab.get_unchecked(key) };
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_by_key_unchecked.record(end.wrapping_sub(start));

            // UntrackedAccessor indexing (unsafe to create, then raw indexing)
            let start = rdtscp();
            let accessor = unsafe { slab.untracked() };
            let val = &accessor[key];
            black_box(val);
            let end = rdtscp();
            let _ = stats.accessor_index.record(end.wrapping_sub(start));
        }

        // Key-based gets (invalid/stale key)
        if !stale_keys.is_empty() {
            let idx = (i as usize * 13) % stale_keys.len();
            let stale_key = stale_keys[idx];

            let start = rdtscp();
            let val = slab.get(stale_key); // Should return None
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_by_key_invalid.record(end.wrapping_sub(start));
        }

        // Entry-based gets (valid)
        if !entries.is_empty() {
            let idx = (i as usize * 11) % entries.len();
            let entry = &entries[idx];

            // Checked (returns Ref<T>)
            let start = rdtscp();
            let val = entry.get();
            black_box(&*val);
            drop(val);
            let end = rdtscp();
            let _ = stats.entry_get.record(end.wrapping_sub(start));

            // Untracked (unsafe, no borrow tracking)
            let start = rdtscp();
            let val = unsafe { entry.get_untracked() };
            black_box(val);
            let end = rdtscp();
            let _ = stats.entry_get_untracked.record(end.wrapping_sub(start));

            // Unchecked (unsafe, no validity/borrow check)
            let start = rdtscp();
            let val = unsafe { entry.get_unchecked() };
            black_box(val);
            let end = rdtscp();
            let _ = stats.entry_get_unchecked.record(end.wrapping_sub(start));
        }

        // Entry-based gets (invalid - slot was removed)
        if !stale_entries.is_empty() {
            let idx = (i as usize * 17) % stale_entries.len();
            let stale_entry = &stale_entries[idx];

            let start = rdtscp();
            let val = stale_entry.try_get(); // Should return None
            black_box(val);
            let end = rdtscp();
            let _ = stats.entry_try_get_invalid.record(end.wrapping_sub(start));
        }

        // Remove periodically to maintain steady state
        if entries.len() + keys.len() > CAPACITY / 2 {
            // Cycle through: entry.remove(), remove_by_key(), entry.remove_unchecked()
            match i % 3 {
                0 if !entries.is_empty() => {
                    let entry = entries.pop().unwrap();

                    // Clone entry before removing to create a stale entry
                    if stale_entries.len() < 1000 {
                        stale_entries.push(entry.clone());
                    }

                    let start = rdtscp();
                    let val = entry.remove();
                    black_box(val);
                    let end = rdtscp();
                    let _ = stats.entry_remove.record(end.wrapping_sub(start));
                }
                1 if !keys.is_empty() => {
                    let key = keys.pop().unwrap();

                    // Save the key as stale before removing
                    if stale_keys.len() < 1000 {
                        stale_keys.push(key);
                    }

                    let start = rdtscp();
                    let val = slab.remove_by_key(key);
                    black_box(val);
                    let end = rdtscp();
                    let _ = stats.remove_by_key.record(end.wrapping_sub(start));
                }
                2 if !entries.is_empty() => {
                    let entry = entries.pop().unwrap();

                    let start = rdtscp();
                    let val = unsafe { entry.remove_unchecked() };
                    black_box(val);
                    let end = rdtscp();
                    let _ = stats.entry_remove_unchecked.record(end.wrapping_sub(start));
                }
                _ => {
                    // Fallback: remove whatever is available
                    if !entries.is_empty() {
                        let entry = entries.pop().unwrap();
                        black_box(entry.remove());
                    } else if !keys.is_empty() {
                        let key = keys.pop().unwrap();
                        black_box(slab.remove_by_key(key));
                    }
                }
            }
        }
    }

    stats
}

fn bench_unbounded() -> Stats {
    let slab = Slab::<u64>::with_capacity(CAPACITY);
    let mut stats = Stats::new();
    let mut entries: Vec<nexus_slab::unbounded::Entry<u64>> = Vec::with_capacity(CAPACITY);
    let mut keys: Vec<Key> = Vec::with_capacity(CAPACITY);
    let mut stale_keys: Vec<Key> = Vec::with_capacity(1000);
    let mut stale_entries: Vec<nexus_slab::unbounded::Entry<u64>> = Vec::with_capacity(1000);

    // Warmup
    for i in 0..1000u64 {
        let entry = slab.insert(i);
        keys.push(entry.leak());
    }
    for key in keys.drain(..) {
        let _ = slab.remove_by_key(key);
    }

    // Measured operations
    for i in 0..OPS as u64 {
        // Insert
        let start = rdtscp();
        let entry = slab.insert(i);
        let end = rdtscp();
        let _ = stats.insert.record(end.wrapping_sub(start));

        // Decide: keep as entry or leak to key (alternate)
        if i % 2 == 0 {
            entries.push(entry);
        } else {
            keys.push(entry.leak());
        }

        // Key-based gets (valid)
        if !keys.is_empty() {
            let idx = (i as usize * 7) % keys.len();
            let key = keys[idx];

            // Checked (returns Option<Ref<T>>)
            let start = rdtscp();
            let val = slab.get(key);
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_by_key.record(end.wrapping_sub(start));

            // Untracked (unsafe, no borrow tracking)
            let start = rdtscp();
            let val = unsafe { slab.get_untracked(key) };
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_by_key_untracked.record(end.wrapping_sub(start));

            // Unchecked (unsafe, no validity check)
            let start = rdtscp();
            let val = unsafe { slab.get_unchecked(key) };
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_by_key_unchecked.record(end.wrapping_sub(start));

            // UntrackedAccessor indexing (unsafe to create, then raw indexing)
            let start = rdtscp();
            let accessor = unsafe { slab.untracked() };
            let val = &accessor[key];
            black_box(val);
            let end = rdtscp();
            let _ = stats.accessor_index.record(end.wrapping_sub(start));
        }

        // Key-based gets (invalid/stale key)
        if !stale_keys.is_empty() {
            let idx = (i as usize * 13) % stale_keys.len();
            let stale_key = stale_keys[idx];

            let start = rdtscp();
            let val = slab.get(stale_key); // Should return None
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_by_key_invalid.record(end.wrapping_sub(start));
        }

        // Entry-based gets (valid)
        if !entries.is_empty() {
            let idx = (i as usize * 11) % entries.len();
            let entry = &entries[idx];

            // Checked (returns Ref<T>)
            let start = rdtscp();
            let val = entry.get();
            black_box(&*val);
            drop(val);
            let end = rdtscp();
            let _ = stats.entry_get.record(end.wrapping_sub(start));

            // Untracked (unsafe, no borrow tracking)
            let start = rdtscp();
            let val = unsafe { entry.get_untracked() };
            black_box(val);
            let end = rdtscp();
            let _ = stats.entry_get_untracked.record(end.wrapping_sub(start));

            // Unchecked (unsafe, no validity/borrow check)
            let start = rdtscp();
            let val = unsafe { entry.get_unchecked() };
            black_box(val);
            let end = rdtscp();
            let _ = stats.entry_get_unchecked.record(end.wrapping_sub(start));
        }

        // Entry-based gets (invalid - slot was removed)
        if !stale_entries.is_empty() {
            let idx = (i as usize * 17) % stale_entries.len();
            let stale_entry = &stale_entries[idx];

            let start = rdtscp();
            let val = stale_entry.try_get(); // Should return None
            black_box(val);
            let end = rdtscp();
            let _ = stats.entry_try_get_invalid.record(end.wrapping_sub(start));
        }

        // Remove periodically to maintain steady state
        if entries.len() + keys.len() > CAPACITY / 2 {
            // Cycle through: entry.remove(), remove_by_key(), entry.remove_unchecked()
            match i % 3 {
                0 if !entries.is_empty() => {
                    let entry = entries.pop().unwrap();

                    // Clone entry before removing to create a stale entry
                    if stale_entries.len() < 1000 {
                        stale_entries.push(entry.clone());
                    }

                    let start = rdtscp();
                    let val = entry.remove();
                    black_box(val);
                    let end = rdtscp();
                    let _ = stats.entry_remove.record(end.wrapping_sub(start));
                }
                1 if !keys.is_empty() => {
                    let key = keys.pop().unwrap();

                    // Save the key as stale before removing
                    if stale_keys.len() < 1000 {
                        stale_keys.push(key);
                    }

                    let start = rdtscp();
                    let val = slab.remove_by_key(key);
                    black_box(val);
                    let end = rdtscp();
                    let _ = stats.remove_by_key.record(end.wrapping_sub(start));
                }
                2 if !entries.is_empty() => {
                    let entry = entries.pop().unwrap();

                    let start = rdtscp();
                    let val = unsafe { entry.remove_unchecked() };
                    black_box(val);
                    let end = rdtscp();
                    let _ = stats.entry_remove_unchecked.record(end.wrapping_sub(start));
                }
                _ => {
                    // Fallback: remove whatever is available
                    if !entries.is_empty() {
                        let entry = entries.pop().unwrap();
                        black_box(entry.remove());
                    } else if !keys.is_empty() {
                        let key = keys.pop().unwrap();
                        black_box(slab.remove_by_key(key));
                    }
                }
            }
        }
    }

    stats
}

fn bench_slab_crate() -> SlabStats {
    let mut slab = slab::Slab::<u64>::with_capacity(CAPACITY);
    let mut stats = SlabStats::new();
    let mut keys = Vec::with_capacity(CAPACITY);
    let mut stale_keys: Vec<usize> = Vec::with_capacity(1000);

    // Warmup
    for i in 0..1000u64 {
        keys.push(slab.insert(i));
    }
    for key in keys.drain(..) {
        let _ = slab.remove(key);
    }

    // Measured operations
    for i in 0..OPS as u64 {
        // Insert
        let start = rdtscp();
        let key = slab.insert(i);
        let end = rdtscp();
        let _ = stats.insert.record(end.wrapping_sub(start));

        keys.push(key);

        // Get (valid key)
        if !keys.is_empty() {
            let idx = (i as usize * 7) % keys.len();
            let key = keys[idx];
            let start = rdtscp();
            let val = slab.get(key);
            black_box(val);
            let end = rdtscp();
            let _ = stats.get.record(end.wrapping_sub(start));
        }

        // Get (invalid/stale key) - slab returns None for out-of-bounds or vacant
        if !stale_keys.is_empty() {
            let idx = (i as usize * 13) % stale_keys.len();
            let stale_key = stale_keys[idx];
            let start = rdtscp();
            let val = slab.get(stale_key); // Returns None (or panics if contains checks fail)
            black_box(val);
            let end = rdtscp();
            let _ = stats.get_invalid.record(end.wrapping_sub(start));
        }

        // Remove periodically to maintain steady state
        if keys.len() > CAPACITY / 2 {
            let key = keys.pop().unwrap();

            // Save as stale
            if stale_keys.len() < 1000 {
                stale_keys.push(key);
            }

            let start = rdtscp();
            let val = slab.remove(key);
            black_box(val);
            let end = rdtscp();
            let _ = stats.remove.record(end.wrapping_sub(start));
        }
    }

    stats
}

fn print_distribution(name: &str, hist: &Histogram<u64>) {
    println!(
        "{:>32}: p1={:>4} p5={:>4} p25={:>4} p50={:>4} p75={:>4} p90={:>4} p99={:>4} p99.9={:>5} p99.99={:>6} max={:>7}",
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
    println!("FULL LATENCY DISTRIBUTION (cycles)");
    println!("Capacity: {}, Operations: {}", CAPACITY, OPS);
    println!("All slabs pre-allocated to same capacity");
    println!("================================================================\n");

    let bounded = bench_bounded();
    let unbounded = bench_unbounded();
    let slab = bench_slab_crate();

    println!("INSERT:");
    print_distribution("bounded", &bounded.insert);
    print_distribution("unbounded", &unbounded.insert);
    print_distribution("slab", &slab.insert);
    println!();

    println!("GET BY KEY - valid (checked, borrow-tracked):");
    print_distribution("bounded", &bounded.get_by_key);
    print_distribution("unbounded", &unbounded.get_by_key);
    print_distribution("slab", &slab.get);
    println!();

    println!("GET BY KEY - STALE/INVALID (safe check):");
    print_distribution("bounded", &bounded.get_by_key_invalid);
    print_distribution("unbounded", &unbounded.get_by_key_invalid);
    print_distribution("slab", &slab.get_invalid);
    println!();

    println!("GET BY KEY - valid (untracked):");
    print_distribution("bounded", &bounded.get_by_key_untracked);
    print_distribution("unbounded", &unbounded.get_by_key_untracked);
    println!("                            slab: n/a");
    println!();

    println!("GET BY KEY - valid (unchecked):");
    print_distribution("bounded", &bounded.get_by_key_unchecked);
    print_distribution("unbounded", &unbounded.get_by_key_unchecked);
    println!("                            slab: n/a");
    println!();

    println!("UNTRACKED ACCESSOR[key]:");
    print_distribution("bounded", &bounded.accessor_index);
    print_distribution("unbounded", &unbounded.accessor_index);
    println!("                            slab: n/a");
    println!();

    println!("ENTRY.get() - valid (checked, borrow-tracked):");
    print_distribution("bounded", &bounded.entry_get);
    print_distribution("unbounded", &unbounded.entry_get);
    println!("                            slab: n/a (no Entry API)");
    println!();

    println!("ENTRY.try_get() - STALE/INVALID (safe check):");
    print_distribution("bounded", &bounded.entry_try_get_invalid);
    print_distribution("unbounded", &unbounded.entry_try_get_invalid);
    println!("                            slab: n/a (no Entry API)");
    println!();

    println!("ENTRY.get_untracked() - valid:");
    print_distribution("bounded", &bounded.entry_get_untracked);
    print_distribution("unbounded", &unbounded.entry_get_untracked);
    println!("                            slab: n/a (no Entry API)");
    println!();

    println!("ENTRY.get_unchecked() - valid:");
    print_distribution("bounded", &bounded.entry_get_unchecked);
    print_distribution("unbounded", &unbounded.entry_get_unchecked);
    println!("                            slab: n/a (no Entry API)");
    println!();

    println!("REMOVE BY KEY:");
    print_distribution("bounded", &bounded.remove_by_key);
    print_distribution("unbounded", &unbounded.remove_by_key);
    print_distribution("slab", &slab.remove);
    println!();

    println!("ENTRY.remove():");
    print_distribution("bounded", &bounded.entry_remove);
    print_distribution("unbounded", &unbounded.entry_remove);
    println!("                            slab: n/a (no Entry API)");
    println!();

    println!("ENTRY.remove_unchecked():");
    print_distribution("bounded", &bounded.entry_remove_unchecked);
    print_distribution("unbounded", &unbounded.entry_remove_unchecked);
    println!("                            slab: n/a (no Entry API)");
}
