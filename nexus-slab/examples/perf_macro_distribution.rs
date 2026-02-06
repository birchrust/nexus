//! Full latency distribution: Raw Slot vs Macro/TLS BoxSlot vs slab crate.
//!
//! Same structure as perf_full_distribution but adds the macro-generated
//! allocator path to show actual TLS cost per operation.
//!
//! TLS is hit on: alloc, drop, into_inner, from_slot
//! TLS is NOT hit on: deref, deref_mut, replace, key()
//!
//! Run with:
//!   cargo build --release --example perf_macro_distribution
//!   taskset -c 0 ./target/release/examples/perf_macro_distribution

use hdrhistogram::Histogram;
use nexus_slab::bounded::Slab as BoundedSlab;
use nexus_slab::Slot;
use seq_macro::seq;
use std::hint::black_box;

// Wrapper type for macro allocator (orphan rules prevent using bare u64)
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Val(pub u64);

// Macro-generated bounded allocator
mod macro_alloc {
    nexus_slab::bounded_allocator!(super::Val);
}

const NUM_SLOTS: usize = 10_000;
const OPS: usize = 500_000;
const BATCH_SIZE: u64 = 100;

macro_rules! unroll {
    ($n:literal, $op:expr) => {
        seq!(_ in 0..$n { $op; })
    };
}

#[inline(always)]
fn rdtsc_start() -> u64 {
    unsafe {
        std::arch::x86_64::_mm_lfence();
        std::arch::x86_64::_rdtsc()
    }
}

#[inline(always)]
fn rdtsc_end() -> u64 {
    unsafe {
        std::arch::x86_64::_mm_lfence();
        std::arch::x86_64::_rdtsc()
    }
}

fn print_hist(name: &str, hist: &Histogram<u64>) {
    println!(
        "  {:34} p50={:>3}  p90={:>3}  p99={:>3}  p99.9={:>4}  max={:>6}",
        name,
        hist.value_at_quantile(0.50),
        hist.value_at_quantile(0.90),
        hist.value_at_quantile(0.99),
        hist.value_at_quantile(0.999),
        hist.max()
    );
}

// =============================================================================
// GET
// =============================================================================

fn bench_get() {
    println!("GET (cycles per operation)");
    println!("--------------------------");

    let mut direct_hist = Histogram::<u64>::new(3).unwrap();
    let mut direct_hot_hist = Histogram::<u64>::new(3).unwrap();
    let mut macro_hist = Histogram::<u64>::new(3).unwrap();
    let mut macro_hot_hist = Histogram::<u64>::new(3).unwrap();
    let mut slab_hist = Histogram::<u64>::new(3).unwrap();

    // Direct slab (raw API)
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let entries: Vec<Slot<u64>> = (0..NUM_SLOTS as u64).map(|i| slab.alloc(i)).collect();

    // Macro slab (RAII BoxSlot)
    let macro_slots: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| macro_alloc::BoxSlot::new(Val(i)))
        .collect();

    // slab crate
    let mut ext_slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let ext_keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| ext_slab.insert(i)).collect();

    // Direct deref - random
    let mut idx = 0usize;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let entry = black_box(&entries[idx % NUM_SLOTS]);
            black_box(&**entry);
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = direct_hist.record((end - start) / BATCH_SIZE);
    }

    // Macro deref - random (should be identical — no TLS)
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slot = black_box(&macro_slots[idx % NUM_SLOTS]);
            black_box(&**slot);
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = macro_hist.record((end - start) / BATCH_SIZE);
    }

    // Direct deref - hot
    let hot_entry = &entries[0];
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            black_box(&**black_box(hot_entry));
        });
        let end = rdtsc_end();
        let _ = direct_hot_hist.record((end - start) / BATCH_SIZE);
    }

    // Macro deref - hot
    let hot_macro = &macro_slots[0];
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            black_box(&**black_box(hot_macro));
        });
        let end = rdtsc_end();
        let _ = macro_hot_hist.record((end - start) / BATCH_SIZE);
    }

    // slab crate get - random
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let s = black_box(&ext_slab);
            let key = black_box(ext_keys[idx % NUM_SLOTS]);
            black_box(s.get(key));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
    }

    print_hist("Raw    deref random", &direct_hist);
    print_hist("BoxSlot deref random", &macro_hist);
    print_hist("Raw    deref hot", &direct_hot_hist);
    print_hist("BoxSlot deref hot", &macro_hot_hist);
    print_hist("slab crate get", &slab_hist);
    println!();

    // Cleanup raw slots
    for slot in entries {
        // SAFETY: slot was allocated from this slab
        unsafe { slab.free(slot) };
    }
    // macro_slots drop automatically via RAII
}

// =============================================================================
// GET_MUT
// =============================================================================

fn bench_get_mut() {
    println!("GET_MUT (cycles per operation)");
    println!("------------------------------");

    let mut direct_hist = Histogram::<u64>::new(3).unwrap();
    let mut macro_hist = Histogram::<u64>::new(3).unwrap();
    let mut slab_hist = Histogram::<u64>::new(3).unwrap();

    // Direct slab (raw API)
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let mut entries: Vec<Slot<u64>> = (0..NUM_SLOTS as u64).map(|i| slab.alloc(i)).collect();

    // Macro slab (RAII BoxSlot)
    let mut macro_slots: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| macro_alloc::BoxSlot::new(Val(i)))
        .collect();

    // slab crate
    let mut ext_slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let ext_keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| ext_slab.insert(i)).collect();

    // Direct deref_mut
    let mut idx = 0usize;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let entry = black_box(&mut entries[idx % NUM_SLOTS]);
            black_box(&mut **entry);
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = direct_hist.record((end - start) / BATCH_SIZE);
    }

    // Macro deref_mut (no TLS)
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slot = black_box(&mut macro_slots[idx % NUM_SLOTS]);
            black_box(&mut **slot);
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = macro_hist.record((end - start) / BATCH_SIZE);
    }

    // slab crate get_mut
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let s = black_box(&mut ext_slab);
            let key = black_box(ext_keys[idx % NUM_SLOTS]);
            black_box(s.get_mut(key));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
    }

    print_hist("Raw    deref_mut", &direct_hist);
    print_hist("BoxSlot deref_mut", &macro_hist);
    print_hist("slab crate get_mut", &slab_hist);
    println!();

    // Cleanup raw slots
    for slot in entries {
        // SAFETY: slot was allocated from this slab
        unsafe { slab.free(slot) };
    }
}

// =============================================================================
// INSERT
// =============================================================================

fn bench_insert() {
    println!("INSERT (cycles per operation)");
    println!("-----------------------------");

    let mut direct_hist = Histogram::<u64>::new(3).unwrap();
    let mut macro_hist = Histogram::<u64>::new(3).unwrap();
    let mut slab_hist = Histogram::<u64>::new(3).unwrap();

    // Direct slab insert (raw API)
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let mut temp: Vec<Slot<u64>> = Vec::with_capacity(BATCH_SIZE as usize);

    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            temp.push(slab.try_alloc(black_box(42u64)).unwrap());
        });
        let end = rdtsc_end();
        let _ = direct_hist.record((end - start) / BATCH_SIZE);
        for entry in temp.drain(..) {
            // SAFETY: slot was allocated from this slab
            unsafe { slab.free(entry) };
        }
    }

    // Macro insert (TLS on alloc)
    let mut macro_temp: Vec<macro_alloc::BoxSlot> = Vec::with_capacity(BATCH_SIZE as usize);

    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            macro_temp.push(macro_alloc::BoxSlot::new(black_box(Val(42))));
        });
        let end = rdtsc_end();
        let _ = macro_hist.record((end - start) / BATCH_SIZE);
        // Drop returns them (TLS on drop, but not timed)
        macro_temp.clear();
    }

    // slab crate insert
    let mut ext_slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let mut temp_keys: Vec<usize> = Vec::with_capacity(BATCH_SIZE as usize);

    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            temp_keys.push(black_box(&mut ext_slab).insert(black_box(42u64)));
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
        for key in temp_keys.drain(..) {
            ext_slab.remove(key);
        }
    }

    print_hist("Raw    insert", &direct_hist);
    print_hist("BoxSlot insert [TLS]", &macro_hist);
    print_hist("slab crate insert", &slab_hist);
    println!();
}

// =============================================================================
// REMOVE / DROP
// =============================================================================

#[allow(unused_assignments)]
fn bench_remove() {
    println!("REMOVE (cycles per operation)");
    println!("-----------------------------");

    let mut direct_hist = Histogram::<u64>::new(3).unwrap();
    let mut macro_hist = Histogram::<u64>::new(3).unwrap();
    let mut slab_hist = Histogram::<u64>::new(3).unwrap();

    // Direct free_take (raw API)
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    for _ in 0..OPS / BATCH_SIZE as usize {
        let mut temp: Vec<Slot<u64>> = Vec::with_capacity(BATCH_SIZE as usize);
        for _ in 0..BATCH_SIZE {
            temp.push(slab.alloc(42u64));
        }
        let start = rdtsc_start();
        unroll!(100, {
            // SAFETY: slot was allocated from this slab
            black_box(unsafe { slab.free_take(temp.pop().unwrap()) });
        });
        let end = rdtsc_end();
        let _ = direct_hist.record((end - start) / BATCH_SIZE);
    }

    // Macro into_inner (TLS on dealloc)
    for _ in 0..OPS / BATCH_SIZE as usize {
        let mut temp: Vec<macro_alloc::BoxSlot> = Vec::with_capacity(BATCH_SIZE as usize);
        for _ in 0..BATCH_SIZE {
            temp.push(macro_alloc::BoxSlot::new(Val(42)));
        }
        let start = rdtsc_start();
        unroll!(100, {
            black_box(temp.pop().unwrap().into_inner());
        });
        let end = rdtsc_end();
        let _ = macro_hist.record((end - start) / BATCH_SIZE);
    }

    // slab crate remove
    let mut ext_slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    for _ in 0..OPS / BATCH_SIZE as usize {
        let temp_keys: Vec<_> = (0..BATCH_SIZE).map(|_| ext_slab.insert(42u64)).collect();
        let mut idx = 0usize;
        let start = rdtsc_start();
        unroll!(100, {
            let s = black_box(&mut ext_slab);
            black_box(s.remove(black_box(temp_keys[idx])));
            idx += 1;
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
    }

    print_hist("Raw    free_take", &direct_hist);
    print_hist("BoxSlot into_inner [TLS]", &macro_hist);
    print_hist("slab crate remove", &slab_hist);
    println!();
}

// =============================================================================
// REPLACE
// =============================================================================

fn bench_replace() {
    println!("REPLACE (cycles per operation)");
    println!("------------------------------");

    let mut direct_hist = Histogram::<u64>::new(3).unwrap();
    let mut macro_hist = Histogram::<u64>::new(3).unwrap();
    let mut slab_hist = Histogram::<u64>::new(3).unwrap();

    // Direct slab (raw API)
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let mut entries: Vec<Slot<u64>> = (0..NUM_SLOTS as u64).map(|i| slab.alloc(i)).collect();

    // Macro slab (RAII BoxSlot)
    let mut macro_slots: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| macro_alloc::BoxSlot::new(Val(i)))
        .collect();

    // slab crate
    let mut ext_slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let ext_keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| ext_slab.insert(i)).collect();

    // Direct replace (no TLS)
    let mut idx = 0usize;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let entry = black_box(&mut entries[idx % NUM_SLOTS]);
            black_box(std::mem::replace(&mut **entry, black_box(999u64)));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = direct_hist.record((end - start) / BATCH_SIZE);
    }

    // Macro replace (no TLS — in-place write)
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slot = black_box(&mut macro_slots[idx % NUM_SLOTS]);
            black_box(slot.replace(black_box(Val(999))));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = macro_hist.record((end - start) / BATCH_SIZE);
    }

    // slab get_mut + replace
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let s = black_box(&mut ext_slab);
            let key = black_box(ext_keys[idx % NUM_SLOTS]);
            if let Some(v) = s.get_mut(key) {
                black_box(std::mem::replace(v, black_box(999u64)));
            }
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
    }

    print_hist("Raw    replace", &direct_hist);
    print_hist("BoxSlot replace", &macro_hist);
    print_hist("slab crate get_mut+replace", &slab_hist);
    println!();

    // Cleanup raw slots
    for slot in entries {
        // SAFETY: slot was allocated from this slab
        unsafe { slab.free(slot) };
    }
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    macro_alloc::Allocator::builder()
        .capacity(NUM_SLOTS * 4)
        .build()
        .expect("init macro allocator");

    println!("RAW vs BOXSLOT/TLS vs SLAB CRATE — FULL DISTRIBUTION");
    println!("====================================================");
    println!(
        "Unrolled {} ops per sample, {} total ops per benchmark",
        BATCH_SIZE, OPS
    );
    println!("All times in CPU cycles (lfence+rdtsc, loop overhead eliminated)");
    println!();
    println!(
        "Raw Slot<T>   size: {} bytes  (pointer wrapper, explicit free)",
        std::mem::size_of::<Slot<u64>>()
    );
    println!(
        "BoxSlot<T,A>  size: {} bytes  (RAII handle, TLS for slab ref)",
        std::mem::size_of::<macro_alloc::BoxSlot>()
    );
    println!();

    bench_get();
    bench_get_mut();
    bench_insert();
    bench_remove();
    bench_replace();

    println!("====================================================");
    println!("Legend:");
    println!("  Raw             Slot<T> via raw slab API (8B, explicit free)");
    println!("  BoxSlot [TLS]   BoxSlot<T,A> via bounded_allocator! (8B, TLS on marked ops)");
    println!("  slab crate      slab 0.4 crate (baseline)");
    println!();
    println!("TLS operations: insert, drop/into_inner, from_slot");
    println!("Non-TLS:        deref, deref_mut, replace, key()");
}
