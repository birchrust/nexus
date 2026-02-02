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

use nexus_slab::create_allocator;

const NUM_SLOTS: usize = 10_000;
const OPS: usize = 500_000;
const BATCH_SIZE: u64 = 100;

// Create macro-based allocator
create_allocator!(bench_slab, u64);

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

    // Setup macro-based slab
    bench_slab::init().bounded(NUM_SLOTS).build();
    let entries: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bench_slab::insert(i))
        .collect();

    // Setup slab crate
    let mut slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.insert(i)).collect();

    // slot.get() - random access
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

    // slot.get() - hot (same slot)
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

    // get_unchecked() - random access (need separate allocator to leak keys)
    create_allocator!(key_bench, u64);
    key_bench::init().bounded(NUM_SLOTS).build();
    let keys2: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| key_bench::insert(i).leak())
        .collect();
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let key = black_box(keys2[idx % NUM_SLOTS]);
            black_box(unsafe { key_bench::get_unchecked(key) });
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = key_hist.record((end - start) / BATCH_SIZE);
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

    print_hist("slot.get() random", &entry_hist);
    print_hist("slot.get() hot", &entry_hot_hist);
    print_hist("get_unchecked() [unsafe]", &key_hist);
    print_hist("slab.get() random", &slab_hist);
    print_hist("slab.get() hot", &slab_hot_hist);
    println!();

    // Cleanup
    drop(entries);
    let _ = bench_slab::shutdown();
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

    // Setup macro-based slab
    bench_slab::init().bounded(NUM_SLOTS).build();
    let mut entries: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bench_slab::insert(i))
        .collect();

    // Setup slab crate
    let mut slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.insert(i)).collect();

    // slot.get_mut() - random access
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

    // get_unchecked_mut() - random access
    create_allocator!(key_mut_bench, u64);
    key_mut_bench::init().bounded(NUM_SLOTS).build();
    let keys2: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| key_mut_bench::insert(i).leak())
        .collect();
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let key = black_box(keys2[idx % NUM_SLOTS]);
            black_box(unsafe { key_mut_bench::get_unchecked_mut(key) });
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = key_hist.record((end - start) / BATCH_SIZE);
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

    print_hist("slot.get_mut()", &entry_hist);
    print_hist("get_unchecked_mut() [unsafe]", &key_hist);
    print_hist("slab.get_mut()", &slab_hist);
    println!();

    // Cleanup
    drop(entries);
    let _ = bench_slab::shutdown();
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

    // Setup macro-based slab
    bench_slab::init().bounded(NUM_SLOTS).build();
    let entries: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bench_slab::insert(i))
        .collect();

    // Setup slab crate
    let mut slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.insert(i)).collect();

    // slot.is_valid() - random access
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
    create_allocator!(contains_bench, u64);
    contains_bench::init().bounded(NUM_SLOTS).build();
    let keys2: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| contains_bench::insert(i).leak())
        .collect();
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let key = black_box(keys2[idx % NUM_SLOTS]);
            black_box(contains_bench::contains_key(key));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = key_hist.record((end - start) / BATCH_SIZE);
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

    print_hist("slot.is_valid()", &entry_hist);
    print_hist("contains_key()", &key_hist);
    print_hist("slab.contains()", &slab_hist);
    println!();

    // Cleanup
    drop(entries);
    let _ = bench_slab::shutdown();
}

// =============================================================================
// INSERT Benchmarks
// =============================================================================

fn bench_insert() {
    println!("INSERT (cycles per operation)");
    println!("-----------------------------");

    let mut entry_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut slab_hist: Histogram<u64> = Histogram::new(3).unwrap();

    // macro insert - need to remove after each batch to make room
    bench_slab::init().bounded(NUM_SLOTS).build();
    let mut temp_entries: Vec<bench_slab::Slot> = Vec::with_capacity(BATCH_SIZE as usize);

    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let entry = bench_slab::try_insert(black_box(42u64)).unwrap();
            temp_entries.push(entry);
        });
        let end = rdtsc_end();
        let _ = entry_hist.record((end - start) / BATCH_SIZE);

        // Cleanup batch
        for entry in temp_entries.drain(..) {
            entry.into_inner();
        }
    }
    let _ = bench_slab::shutdown();

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

    print_hist("macro insert", &entry_hist);
    print_hist("slab crate insert", &slab_hist);
    println!();
}

// =============================================================================
// REMOVE Benchmarks
// =============================================================================

#[allow(unused_assignments)]
fn bench_remove() {
    println!("REMOVE (cycles per operation)");
    println!("-----------------------------");

    let mut entry_hist: Histogram<u64> = Histogram::new(3).unwrap();
    let mut slab_hist: Histogram<u64> = Histogram::new(3).unwrap();

    // slot.into_inner() - insert batch, then remove batch
    bench_slab::init().bounded(NUM_SLOTS).build();

    for _ in 0..OPS / BATCH_SIZE as usize {
        // Insert batch
        let mut temp_entries: Vec<bench_slab::Slot> = Vec::with_capacity(BATCH_SIZE as usize);
        for _ in 0..BATCH_SIZE {
            temp_entries.push(bench_slab::insert(42u64));
        }

        // Time the removes
        let start = rdtsc_start();
        unroll!(100, {
            let entry = temp_entries.pop().unwrap();
            black_box(entry.into_inner());
        });
        let end = rdtsc_end();
        let _ = entry_hist.record((end - start) / BATCH_SIZE);
    }
    let _ = bench_slab::shutdown();

    // slab.remove()
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

    print_hist("slot.into_inner()", &entry_hist);
    print_hist("slab.remove()", &slab_hist);
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

    // Setup macro-based slab
    bench_slab::init().bounded(NUM_SLOTS).build();
    let mut entries: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| bench_slab::insert(i))
        .collect();

    // Setup slab crate
    let mut slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.insert(i)).collect();

    // slot.replace() - random access
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

    print_hist("slot.replace()", &entry_hist);
    print_hist("slab get_mut+replace", &slab_hist);
    println!();

    // Cleanup
    drop(entries);
    let _ = bench_slab::shutdown();
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    println!("MACRO API vs SLAB CRATE - CYCLE COUNTS");
    println!("======================================");
    println!("Compare these results against BENCHMARKS.md (handle-based API)");
    println!(
        "Unrolled {} ops per sample, {} total ops per benchmark",
        BATCH_SIZE, OPS
    );
    println!("All times in CPU cycles (lfence+rdtsc, loop overhead eliminated)");
    println!();
    println!("Slot size: {} bytes (vs 16 bytes for handle API)",
             std::mem::size_of::<bench_slab::Slot>());
    println!();

    bench_get();
    bench_get_mut();
    bench_contains();
    bench_insert();
    bench_remove();
    bench_replace();

    println!("======================================");
    println!("Legend:");
    println!("  slot.*()              Macro Slot API (8-byte slot, TLS lookup)");
    println!("  *_unchecked() [unsafe] Key-based API via TLS");
    println!("  slab.*()              slab crate (baseline comparison)");
}
