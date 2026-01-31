//! Latency distribution benchmarks for nexus-slab vs slab crate.
//!
//! Uses unrolled loops (100 ops) to eliminate loop overhead and measure
//! actual CPU cycles per operation.
//!
//! Run with:
//!   cargo build --release --example perf_full_distribution
//!   taskset -c 0 ./target/release/examples/perf_full_distribution

use hdrhistogram::Histogram;
use seq_macro::seq;
use std::hint::black_box;

use nexus_slab::bounded;

const NUM_SLOTS: usize = 10_000;
const OPS: usize = 500_000;
const BATCH_SIZE: u64 = 100;

/// Unroll N iterations of an expression - no loop overhead, just straight-line code.
macro_rules! unroll {
    ($n:literal, $op:expr) => {
        seq!(_ in 0..$n { $op; })
    };
}

#[inline(always)]
fn rdtsc_start() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_mm_lfence();
        std::arch::x86_64::_rdtsc()
    }
    #[cfg(not(target_arch = "x86_64"))]
    panic!("rdtsc only supported on x86_64")
}

#[inline(always)]
fn rdtsc_end() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        std::arch::x86_64::_mm_lfence();
        std::arch::x86_64::_rdtsc()
    }
    #[cfg(not(target_arch = "x86_64"))]
    panic!("rdtsc only supported on x86_64")
}

fn print_hist(name: &str, hist: &Histogram<u64>) {
    println!(
        "  {:28} p50={:>3}  p90={:>3}  p99={:>3}  p99.9={:>4}  max={:>6}",
        name,
        hist.value_at_quantile(0.50),
        hist.value_at_quantile(0.90),
        hist.value_at_quantile(0.99),
        hist.value_at_quantile(0.999),
        hist.max()
    );
}

// =============================================================================
// GET Benchmarks
// =============================================================================

fn bench_get() {
    println!("GET (cycles per operation)");
    println!("--------------------------");

    let mut entry_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut entry_hot_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut key_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut slab_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut slab_hot_hist: Histogram<u64> = Histogram::new(3).unwrap();

    // Setup bounded slab
    let bounded_slab = bounded::Slab::<u64>::with_capacity(NUM_SLOTS);
    let entries: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bounded_slab.try_insert(i).unwrap())
        .collect();

    // Setup slab crate
    let mut slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.insert(i)).collect();

    // entry.get() - random access
    let mut idx = 0usize;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let entry = black_box(&entries[idx % NUM_SLOTS]);
            black_box(entry.get());
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = entry_hist.record((end - start) / BATCH_SIZE);
    }

    // entry.get() - hot (same slot)
    let hot_entry = &entries[0];
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let entry = black_box(hot_entry);
            black_box(entry.get());
        });
        let end = rdtsc_end();
        let _ = entry_hot_hist.record((end - start) / BATCH_SIZE);
    }

    // get_by_key() - random access
    let bounded_slab2 = bounded::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys2: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bounded_slab2.try_insert(i).unwrap().leak())
        .collect();
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&bounded_slab2);
            let key = black_box(keys2[idx % NUM_SLOTS]);
            black_box(unsafe { slab_ref.get_by_key(key) });
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = key_hist.record((end - start) / BATCH_SIZE);
    }
    // Cleanup
    for key in keys2 {
        unsafe { bounded_slab2.remove_by_key(key) };
    }

    // slab.get() - random access
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&slab);
            let key = black_box(keys[idx % NUM_SLOTS]);
            black_box(slab_ref.get(key));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
    }

    // slab.get() - hot (same key)
    let hot_key = keys[0];
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&slab);
            let key = black_box(hot_key);
            black_box(slab_ref.get(key));
        });
        let end = rdtsc_end();
        let _ = slab_hot_hist.record((end - start) / BATCH_SIZE);
    }

    print_hist("entry.get() random", &entry_hist);
    print_hist("entry.get() hot", &entry_hot_hist);
    print_hist("get_by_key() [unsafe]", &key_hist);
    print_hist("slab.get() random", &slab_hist);
    print_hist("slab.get() hot", &slab_hot_hist);
    println!();
}

// =============================================================================
// GET_MUT Benchmarks
// =============================================================================

fn bench_get_mut() {
    println!("GET_MUT (cycles per operation)");
    println!("------------------------------");

    let mut entry_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut key_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut slab_hist: Histogram<u64> = Histogram::new(3).unwrap();

    // Setup bounded slab
    let bounded_slab = bounded::Slab::<u64>::with_capacity(NUM_SLOTS);
    let mut entries: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bounded_slab.try_insert(i).unwrap())
        .collect();

    // Setup slab crate
    let mut slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.insert(i)).collect();

    // entry.get_mut() - random access
    let mut idx = 0usize;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let entry = black_box(&mut entries[idx % NUM_SLOTS]);
            black_box(entry.get_mut());
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = entry_hist.record((end - start) / BATCH_SIZE);
    }

    // get_by_key_mut() - random access
    let bounded_slab2 = bounded::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys2: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bounded_slab2.try_insert(i).unwrap().leak())
        .collect();
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&bounded_slab2);
            let key = black_box(keys2[idx % NUM_SLOTS]);
            black_box(unsafe { slab_ref.get_by_key_mut(key) });
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = key_hist.record((end - start) / BATCH_SIZE);
    }
    for key in keys2 {
        unsafe { bounded_slab2.remove_by_key(key) };
    }

    // slab.get_mut() - random access
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&mut slab);
            let key = black_box(keys[idx % NUM_SLOTS]);
            black_box(slab_ref.get_mut(key));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
    }

    print_hist("entry.get_mut()", &entry_hist);
    print_hist("get_by_key_mut() [unsafe]", &key_hist);
    print_hist("slab.get_mut()", &slab_hist);
    println!();
}

// =============================================================================
// CONTAINS Benchmarks
// =============================================================================

fn bench_contains() {
    println!("CONTAINS (cycles per operation)");
    println!("-------------------------------");

    let mut entry_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut key_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut slab_hist: Histogram<u64> = Histogram::new(3).unwrap();

    // Setup bounded slab
    let bounded_slab = bounded::Slab::<u64>::with_capacity(NUM_SLOTS);
    let entries: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bounded_slab.try_insert(i).unwrap())
        .collect();

    // Setup slab crate
    let mut slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.insert(i)).collect();

    // entry.is_valid() - random access
    let mut idx = 0usize;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let entry = black_box(&entries[idx % NUM_SLOTS]);
            black_box(entry.is_valid());
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = entry_hist.record((end - start) / BATCH_SIZE);
    }

    // contains_key() - random access
    let bounded_slab2 = bounded::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys2: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bounded_slab2.try_insert(i).unwrap().leak())
        .collect();
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&bounded_slab2);
            let key = black_box(keys2[idx % NUM_SLOTS]);
            black_box(slab_ref.contains_key(key));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = key_hist.record((end - start) / BATCH_SIZE);
    }
    for key in keys2 {
        unsafe { bounded_slab2.remove_by_key(key) };
    }

    // slab.contains() - random access
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&slab);
            let key = black_box(keys[idx % NUM_SLOTS]);
            black_box(slab_ref.contains(key));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
    }

    print_hist("entry.is_valid()", &entry_hist);
    print_hist("contains_key()", &key_hist);
    print_hist("slab.contains()", &slab_hist);
    println!();
}

// =============================================================================
// INSERT Benchmarks
// =============================================================================

fn bench_insert() {
    println!("INSERT (cycles per operation)");
    println!("-----------------------------");

    let mut entry_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut slab_hist: Histogram<u64> = Histogram::new(3).unwrap();

    // bounded::Slab insert - need to remove after each batch to make room
    let bounded_slab = bounded::Slab::<u64>::with_capacity(NUM_SLOTS);
    let mut temp_entries: Vec<bounded::Slot<u64>> = Vec::with_capacity(BATCH_SIZE as usize);

    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&bounded_slab);
            let entry = slab_ref.try_insert(black_box(42u64)).unwrap();
            temp_entries.push(entry);
        });
        let end = rdtsc_end();
        let _ = entry_hist.record((end - start) / BATCH_SIZE);

        // Cleanup batch
        for entry in temp_entries.drain(..) {
            entry.into_inner();
        }
    }

    // slab crate insert
    let mut slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let mut temp_keys: Vec<usize> = Vec::with_capacity(BATCH_SIZE as usize);

    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&mut slab);
            let key = slab_ref.insert(black_box(42u64));
            temp_keys.push(key);
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);

        // Cleanup batch
        for key in temp_keys.drain(..) {
            slab.remove(key);
        }
    }

    print_hist("bounded::Slab insert", &entry_hist);
    print_hist("slab crate insert", &slab_hist);
    println!();
}

// =============================================================================
// REMOVE Benchmarks
// =============================================================================

fn bench_remove() {
    println!("REMOVE (cycles per operation)");
    println!("-----------------------------");

    let mut entry_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut key_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut slab_hist: Histogram<u64> = Histogram::new(3).unwrap();

    // entry.into_inner() - insert batch, then remove batch
    let bounded_slab = bounded::Slab::<u64>::with_capacity(NUM_SLOTS);

    for _ in 0..OPS / BATCH_SIZE as usize {
        // Insert batch
        let mut temp_entries: Vec<bounded::Slot<u64>> = Vec::with_capacity(BATCH_SIZE as usize);
        for _ in 0..BATCH_SIZE {
            temp_entries.push(bounded_slab.try_insert(42u64).unwrap());
        }

        // Time the removes
        let mut idx = 0usize;
        let start = rdtsc_start();
        unroll!(100, {
            let entry = black_box(std::mem::replace(
                &mut temp_entries[idx],
                bounded_slab.try_insert(0).unwrap(),
            )); // placeholder
            black_box(entry.into_inner());
            idx += 1;
        });
        let end = rdtsc_end();
        let _ = entry_hist.record((end - start) / BATCH_SIZE);

        // Cleanup placeholders
        for entry in temp_entries {
            entry.into_inner();
        }
    }

    // remove_by_key()
    let bounded_slab2 = bounded::Slab::<u64>::with_capacity(NUM_SLOTS);

    for _ in 0..OPS / BATCH_SIZE as usize {
        // Insert batch
        let mut temp_keys: Vec<_> = (0..BATCH_SIZE)
            .map(|_| bounded_slab2.try_insert(42u64).unwrap().leak())
            .collect();

        let mut idx = 0usize;
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&bounded_slab2);
            let key = black_box(temp_keys[idx]);
            black_box(unsafe { slab_ref.remove_by_key(key) });
            idx += 1;
        });
        let end = rdtsc_end();
        let _ = key_hist.record((end - start) / BATCH_SIZE);
    }

    // slab.into_inner()
    let mut slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);

    for _ in 0..OPS / BATCH_SIZE as usize {
        // Insert batch
        let temp_keys: Vec<_> = (0..BATCH_SIZE).map(|_| slab.insert(42u64)).collect();

        let mut idx = 0usize;
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&mut slab);
            let key = black_box(temp_keys[idx]);
            black_box(slab_ref.remove(key));
            idx += 1;
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
    }

    print_hist("entry.into_inner()", &entry_hist);
    print_hist("remove_by_key() [unsafe]", &key_hist);
    print_hist("slab.into_inner()", &slab_hist);
    println!();
}

// =============================================================================
// REPLACE Benchmarks
// =============================================================================

fn bench_replace() {
    println!("REPLACE (cycles per operation)");
    println!("------------------------------");

    let mut entry_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut slab_hist: Histogram<u64> = Histogram::new(3).unwrap();

    // Setup bounded slab
    let bounded_slab = bounded::Slab::<u64>::with_capacity(NUM_SLOTS);
    let mut entries: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bounded_slab.try_insert(i).unwrap())
        .collect();

    // Setup slab crate
    let mut slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.insert(i)).collect();

    // entry.replace() - random access
    let mut idx = 0usize;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let entry = black_box(&mut entries[idx % NUM_SLOTS]);
            black_box(entry.replace(black_box(999u64)));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = entry_hist.record((end - start) / BATCH_SIZE);
    }

    // slab get_mut + replace
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&mut slab);
            let key = black_box(keys[idx % NUM_SLOTS]);
            if let Some(v) = slab_ref.get_mut(key) {
                black_box(std::mem::replace(v, black_box(999u64)));
            }
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
    }

    print_hist("entry.replace()", &entry_hist);
    print_hist("slab get_mut+replace", &slab_hist);
    println!();
}

// =============================================================================
// TAKE Benchmarks
// =============================================================================

fn bench_take() {
    println!("TAKE (cycles per operation)");
    println!("---------------------------");

    let mut entry_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut slab_hist: Histogram<u64> = Histogram::new(3).unwrap();

    // entry.take() - take and re-insert
    let bounded_slab = bounded::Slab::<u64>::with_capacity(BATCH_SIZE as usize + 10);

    for _ in 0..OPS / BATCH_SIZE as usize {
        // Setup: insert batch
        let mut entries: Vec<_> = (0..BATCH_SIZE)
            .map(|_| bounded_slab.try_insert(42u64).unwrap())
            .collect();

        let mut idx = 0usize;
        let start = rdtsc_start();
        unroll!(100, {
            let entry = std::mem::replace(&mut entries[idx], bounded_slab.try_insert(0).unwrap()); // placeholder
            let entry = black_box(entry);
            let (val, vacant) = entry.take();
            black_box(val);
            entries[idx] = vacant.insert(42u64);
            idx += 1;
        });
        let end = rdtsc_end();
        let _ = entry_hist.record((end - start) / BATCH_SIZE);

        // Cleanup
        for entry in entries {
            entry.into_inner();
        }
    }

    // slab.into_inner() (no vacant entry concept)
    let mut slab = slab::Slab::<u64>::with_capacity(BATCH_SIZE as usize + 10);

    for _ in 0..OPS / BATCH_SIZE as usize {
        // Setup
        let mut keys: Vec<_> = (0..BATCH_SIZE).map(|_| slab.insert(42u64)).collect();

        let mut idx = 0usize;
        let start = rdtsc_start();
        unroll!(100, {
            let slab_ref = black_box(&mut slab);
            let key = black_box(keys[idx]);
            let val = slab_ref.remove(key);
            black_box(val);
            keys[idx] = slab_ref.insert(42u64); // re-insert
            idx += 1;
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);

        // Cleanup
        for key in keys {
            slab.remove(key);
        }
    }

    print_hist("entry.take()", &entry_hist);
    print_hist("slab remove+insert", &slab_hist);
    println!();
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    println!("NEXUS-SLAB vs SLAB CRATE - ACTUAL CYCLE COUNTS");
    println!("===============================================");
    println!(
        "Unrolled {} ops per sample, {} total ops per benchmark",
        BATCH_SIZE, OPS
    );
    println!("All times in CPU cycles (lfence+rdtsc, loop overhead eliminated)\n");

    bench_get();
    bench_get_mut();
    bench_contains();
    bench_insert();
    bench_remove();
    bench_replace();
    bench_take();

    println!("===============================================");
    println!("Legend:");
    println!("  entry.*()           Slot-based API (safe, owns slot)");
    println!("  *_by_key() [unsafe] Key-based API (caller ensures validity)");
    println!("  slab.*()            slab crate (key + bounds checking)");
}
