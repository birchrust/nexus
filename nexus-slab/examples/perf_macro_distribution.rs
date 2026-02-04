//! Full latency distribution: Direct Slot vs Macro/TLS Slot vs slab crate.
//!
//! Same structure as perf_full_distribution but adds the macro-generated
//! allocator path to show actual TLS cost per operation.
//!
//! TLS is hit on: alloc, drop, into_inner, contains_key, from_key, from_key_mut
//! TLS is NOT hit on: deref, deref_mut, replace, key()
//!
//! Run with:
//!   cargo build --release --example perf_macro_distribution
//!   taskset -c 0 ./target/release/examples/perf_macro_distribution

use hdrhistogram::Histogram;
use nexus_slab::bounded::{Slab as BoundedSlab, Slot as BoundedSlot};
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
    let mut macro_key_hist = Histogram::<u64>::new(3).unwrap();
    let mut direct_key_hist = Histogram::<u64>::new(3).unwrap();
    let mut slab_hist = Histogram::<u64>::new(3).unwrap();

    // Direct slab
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let entries: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.new_slot(i)).collect();

    // Macro slab
    let macro_slots: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| macro_alloc::Slot::new(Val(i)))
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

    // Direct get_by_key - random (leaked keys)
    let key_slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let leaked_keys: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| key_slab.new_slot(i).leak())
        .collect();
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let key = black_box(leaked_keys[idx % NUM_SLOTS]);
            black_box(unsafe { key_slab.get_by_key(key) }.unwrap());
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = direct_key_hist.record((end - start) / BATCH_SIZE);
    }

    // Macro from_key - random (leaked keys — hits TLS via A::slot_cell)
    let macro_leaked: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| macro_alloc::Slot::new(Val(i)).leak())
        .collect();
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let key = black_box(macro_leaked[idx % NUM_SLOTS]);
            black_box(unsafe { macro_alloc::Slot::from_key(key) });
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = macro_key_hist.record((end - start) / BATCH_SIZE);
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

    print_hist("Direct deref random", &direct_hist);
    print_hist("Macro  deref random", &macro_hist);
    print_hist("Direct deref hot", &direct_hot_hist);
    print_hist("Macro  deref hot", &macro_hot_hist);
    print_hist("Direct get_by_key [unsafe]", &direct_key_hist);
    print_hist("Macro  from_key [unsafe,TLS]", &macro_key_hist);
    print_hist("slab crate get", &slab_hist);
    println!();

    // Cleanup leaked macro keys
    for key in macro_leaked {
        unsafe { macro_alloc::Slot::remove_by_key(key) };
    }
    drop(entries);
    drop(macro_slots);
}

// =============================================================================
// GET_MUT
// =============================================================================

fn bench_get_mut() {
    println!("GET_MUT (cycles per operation)");
    println!("------------------------------");

    let mut direct_hist = Histogram::<u64>::new(3).unwrap();
    let mut macro_hist = Histogram::<u64>::new(3).unwrap();
    let mut direct_key_hist = Histogram::<u64>::new(3).unwrap();
    let mut macro_key_hist = Histogram::<u64>::new(3).unwrap();
    let mut slab_hist = Histogram::<u64>::new(3).unwrap();

    // Direct slab
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let mut entries: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.new_slot(i)).collect();

    // Macro slab
    let mut macro_slots: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| macro_alloc::Slot::new(Val(i)))
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

    // Direct get_by_key_mut (leaked keys)
    let key_slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let leaked_keys: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| key_slab.new_slot(i).leak())
        .collect();
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let key = black_box(leaked_keys[idx % NUM_SLOTS]);
            black_box(unsafe { key_slab.get_by_key_mut(key) }.unwrap());
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = direct_key_hist.record((end - start) / BATCH_SIZE);
    }

    // Macro from_key_mut (TLS via A::slot_cell)
    let macro_leaked: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| macro_alloc::Slot::new(Val(i)).leak())
        .collect();
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let key = black_box(macro_leaked[idx % NUM_SLOTS]);
            black_box(unsafe { macro_alloc::Slot::from_key_mut(key) });
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = macro_key_hist.record((end - start) / BATCH_SIZE);
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

    print_hist("Direct deref_mut", &direct_hist);
    print_hist("Macro  deref_mut", &macro_hist);
    print_hist("Direct get_by_key_mut [unsafe]", &direct_key_hist);
    print_hist("Macro  from_key_mut [unsafe,TLS]", &macro_key_hist);
    print_hist("slab crate get_mut", &slab_hist);
    println!();

    for key in macro_leaked {
        unsafe { macro_alloc::Slot::remove_by_key(key) };
    }
    drop(entries);
    drop(macro_slots);
}

// =============================================================================
// CONTAINS
// =============================================================================

fn bench_contains() {
    println!("CONTAINS (cycles per operation)");
    println!("-------------------------------");

    let mut direct_hist = Histogram::<u64>::new(3).unwrap();
    let mut macro_hist = Histogram::<u64>::new(3).unwrap();
    let mut slab_hist = Histogram::<u64>::new(3).unwrap();

    // Direct slab
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let entries: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.new_slot(i)).collect();

    // Macro slab
    let macro_slots: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| macro_alloc::Slot::new(Val(i)))
        .collect();

    // slab crate
    let mut ext_slab = slab::Slab::<u64>::with_capacity(NUM_SLOTS);
    let ext_keys: Vec<_> = (0..NUM_SLOTS as u64).map(|i| ext_slab.insert(i)).collect();

    // Direct contains_key
    let mut idx = 0usize;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let entry = black_box(&entries[idx % NUM_SLOTS]);
            black_box(slab.contains_key(entry.key()));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = direct_hist.record((end - start) / BATCH_SIZE);
    }

    // Macro contains_key (TLS via A::contains_key)
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let slot = black_box(&macro_slots[idx % NUM_SLOTS]);
            black_box(macro_alloc::Slot::contains_key(slot.key()));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = macro_hist.record((end - start) / BATCH_SIZE);
    }

    // slab crate contains
    idx = 0;
    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            let s = black_box(&ext_slab);
            let key = black_box(ext_keys[idx % NUM_SLOTS]);
            black_box(s.contains(key));
            idx = idx.wrapping_add(1);
        });
        let end = rdtsc_end();
        let _ = slab_hist.record((end - start) / BATCH_SIZE);
    }

    print_hist("Direct contains_key", &direct_hist);
    print_hist("Macro  contains_key [TLS]", &macro_hist);
    print_hist("slab crate contains", &slab_hist);
    println!();

    drop(entries);
    drop(macro_slots);
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

    // Direct slab insert
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let mut temp: Vec<BoundedSlot<u64>> = Vec::with_capacity(BATCH_SIZE as usize);

    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            temp.push(slab.try_new_slot(black_box(42u64)).unwrap());
        });
        let end = rdtsc_end();
        let _ = direct_hist.record((end - start) / BATCH_SIZE);
        for entry in temp.drain(..) {
            entry.into_inner();
        }
    }

    // Macro insert (TLS on alloc)
    let mut macro_temp: Vec<macro_alloc::Slot> = Vec::with_capacity(BATCH_SIZE as usize);

    for _ in 0..OPS / BATCH_SIZE as usize {
        let start = rdtsc_start();
        unroll!(100, {
            macro_temp.push(macro_alloc::Slot::new(black_box(Val(42))));
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

    print_hist("Direct insert", &direct_hist);
    print_hist("Macro  insert [TLS]", &macro_hist);
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

    // Direct into_inner
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    for _ in 0..OPS / BATCH_SIZE as usize {
        let mut temp: Vec<BoundedSlot<u64>> = Vec::with_capacity(BATCH_SIZE as usize);
        for _ in 0..BATCH_SIZE {
            temp.push(slab.new_slot(42u64));
        }
        let start = rdtsc_start();
        unroll!(100, {
            black_box(temp.pop().unwrap().into_inner());
        });
        let end = rdtsc_end();
        let _ = direct_hist.record((end - start) / BATCH_SIZE);
    }

    // Macro into_inner (TLS on dealloc)
    for _ in 0..OPS / BATCH_SIZE as usize {
        let mut temp: Vec<macro_alloc::Slot> = Vec::with_capacity(BATCH_SIZE as usize);
        for _ in 0..BATCH_SIZE {
            temp.push(macro_alloc::Slot::new(Val(42)));
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

    print_hist("Direct into_inner", &direct_hist);
    print_hist("Macro  into_inner [TLS]", &macro_hist);
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

    // Direct slab
    let slab = BoundedSlab::<u64>::new(NUM_SLOTS as u32);
    let mut entries: Vec<_> = (0..NUM_SLOTS as u64).map(|i| slab.new_slot(i)).collect();

    // Macro slab
    let mut macro_slots: Vec<_> = (0..NUM_SLOTS as u64)
        .map(|i| macro_alloc::Slot::new(Val(i)))
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
            black_box(entry.replace(black_box(999u64)));
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

    print_hist("Direct replace", &direct_hist);
    print_hist("Macro  replace", &macro_hist);
    print_hist("slab crate get_mut+replace", &slab_hist);
    println!();

    drop(entries);
    drop(macro_slots);
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    macro_alloc::Allocator::builder()
        .capacity(NUM_SLOTS * 4)
        .build()
        .expect("init macro allocator");

    println!("DIRECT vs MACRO/TLS vs SLAB CRATE — FULL DISTRIBUTION");
    println!("======================================================");
    println!(
        "Unrolled {} ops per sample, {} total ops per benchmark",
        BATCH_SIZE, OPS
    );
    println!("All times in CPU cycles (lfence+rdtsc, loop overhead eliminated)");
    println!();
    println!(
        "Direct Slot size: {} bytes  (bounded::Slot — slot_ptr + slab ref)",
        std::mem::size_of::<BoundedSlot<u64>>()
    );
    println!(
        "Macro  Slot size: {} bytes  (alloc::Slot  — slot_ptr only, TLS for slab ref)",
        std::mem::size_of::<macro_alloc::Slot>()
    );
    println!();

    bench_get();
    bench_get_mut();
    bench_contains();
    bench_insert();
    bench_remove();
    bench_replace();

    println!("======================================================");
    println!("Legend:");
    println!("  Direct          bounded::Slot (16B, no TLS)");
    println!("  Macro  [TLS]    alloc::Slot via bounded_allocator! (8B, TLS on marked ops)");
    println!("  slab crate      slab 0.4 crate (baseline)");
    println!();
    println!("TLS operations: insert, drop/into_inner, contains_key, from_key, from_key_mut");
    println!("Non-TLS:        deref, deref_mut, replace, key()");
}
