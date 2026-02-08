//! Skip list benchmark: cycle-accurate latency measurement.
//!
//! Measures insert, remove, get, entry, pop_first, pop_last at various
//! population sizes. Skip list operations are O(log n), so we measure at
//! both small (100) and steady-state (10k) sizes.
//!
//! Run with:
//!   cargo build --release --example perf_skiplist
//!   taskset -c 0 ./target/release/examples/perf_skiplist

use seq_macro::seq;
use std::hint::black_box;

mod sl {
    nexus_collections::skip_allocator!(u64, u64, bounded);
}

const CAPACITY: usize = 200_000;
const SAMPLES: usize = 50_000;
const WARMUP: usize = 5_000;
const BATCH_READ: usize = 100;
const STEADY_SIZE: usize = 10_000;
const SMALL_SIZE: usize = 100;

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
        let tsc = std::arch::x86_64::__rdtscp(&mut 0u32 as *mut _);
        std::arch::x86_64::_mm_lfence();
        tsc
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn print_row(label: &str, samples: &mut [u64]) {
    samples.sort_unstable();
    println!(
        "  {:<32} p50={:>5}  p90={:>5}  p99={:>6}  p999={:>7}  max={:>8}",
        label,
        percentile(samples, 50.0),
        percentile(samples, 90.0),
        percentile(samples, 99.0),
        percentile(samples, 99.9),
        samples[samples.len() - 1],
    );
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

fn main() {
    sl::Allocator::builder().capacity(CAPACITY).build().unwrap();

    let mut rng = Xorshift::new(0xDEAD_BEEF_CAFE_BABEu64);

    println!("SKIP LIST OPERATION LATENCY (cycles/op) — steady state populations");
    println!("Samples: {SAMPLES}, Warmup: {WARMUP}");
    println!("====================================================================\n");

    // ── GET (batched, read-only — seq_macro unrolled) ───────────────
    println!("GET (read-only, {BATCH_READ} unrolled ops/sample)");
    println!("---");

    // get (small @100)
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let keys: Vec<u64> = (0..SMALL_SIZE).map(|_| rng.next()).collect();
        for &k in &keys {
            map.try_insert(k, k).unwrap();
        }
        // Pick 100 keys to look up repeatedly (all hits)
        let lookup: [u64; BATCH_READ] = std::array::from_fn(|i| keys[i % keys.len()]);
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..WARMUP {
            seq!(I in 0..100 { black_box(map.get(&lookup[I])); });
        }
        for _ in 0..SAMPLES {
            let s = rdtsc_start();
            seq!(I in 0..100 { black_box(map.get(&lookup[I])); });
            let e = rdtsc_end();
            samples.push((e - s) / BATCH_READ as u64);
        }
        print_row(&format!("get (hit, @{SMALL_SIZE})"), &mut samples);
        map.clear();
    }

    // get (steady @10k)
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let keys: Vec<u64> = (0..STEADY_SIZE).map(|_| rng.next()).collect();
        for &k in &keys {
            map.try_insert(k, k).unwrap();
        }
        let lookup: [u64; BATCH_READ] = std::array::from_fn(|i| keys[i % keys.len()]);
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..WARMUP {
            seq!(I in 0..100 { black_box(map.get(&lookup[I])); });
        }
        for _ in 0..SAMPLES {
            let s = rdtsc_start();
            seq!(I in 0..100 { black_box(map.get(&lookup[I])); });
            let e = rdtsc_end();
            samples.push((e - s) / BATCH_READ as u64);
        }
        print_row(&format!("get (hit, @{STEADY_SIZE})"), &mut samples);

        // get miss
        let miss_key = u64::MAX;
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..WARMUP {
            seq!(_ in 0..100 { black_box(map.get(&miss_key)); });
        }
        for _ in 0..SAMPLES {
            let s = rdtsc_start();
            seq!(_ in 0..100 { black_box(map.get(&miss_key)); });
            let e = rdtsc_end();
            samples.push((e - s) / BATCH_READ as u64);
        }
        print_row(&format!("get (miss, @{STEADY_SIZE})"), &mut samples);
        map.clear();
    }

    // contains_key (steady @10k)
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let keys: Vec<u64> = (0..STEADY_SIZE).map(|_| rng.next()).collect();
        for &k in &keys {
            map.try_insert(k, k).unwrap();
        }
        let lookup: [u64; BATCH_READ] = std::array::from_fn(|i| keys[i % keys.len()]);
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..WARMUP {
            seq!(I in 0..100 { black_box(map.contains_key(&lookup[I])); });
        }
        for _ in 0..SAMPLES {
            let s = rdtsc_start();
            seq!(I in 0..100 { black_box(map.contains_key(&lookup[I])); });
            let e = rdtsc_end();
            samples.push((e - s) / BATCH_READ as u64);
        }
        print_row(&format!("contains_key (hit, @{STEADY_SIZE})"), &mut samples);
        map.clear();
    }

    println!();

    // ── INSERT / REMOVE (per-op timing) ─────────────────────────────
    println!("INSERT / REMOVE (per-op, cycles)");
    println!("---");

    // insert (into empty, growing)
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..WARMUP {
            map.try_insert(rng.next(), 0).unwrap();
        }
        map.clear();
        for _ in 0..SAMPLES {
            let k = rng.next();
            let s = rdtsc_start();
            black_box(map.try_insert(k, 0));
            let e = rdtsc_end();
            samples.push(e - s);
        }
        print_row("insert (growing)", &mut samples);
        map.clear();
    }

    // insert (steady @10k) — insert then remove to maintain size
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let steady_keys: Vec<u64> = (0..STEADY_SIZE).map(|_| rng.next()).collect();
        for &k in &steady_keys {
            map.try_insert(k, k).unwrap();
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..WARMUP {
            let k = rng.next();
            map.try_insert(k, 0).unwrap();
            map.remove(&k);
        }
        for _ in 0..SAMPLES {
            let k = rng.next();
            let s = rdtsc_start();
            black_box(map.try_insert(k, 0));
            let e = rdtsc_end();
            samples.push(e - s);
            map.remove(&k);
        }
        print_row(&format!("insert (steady @{STEADY_SIZE})"), &mut samples);
        map.clear();
    }

    // remove (steady @10k) — remove then reinsert to maintain size
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let keys: Vec<u64> = (0..STEADY_SIZE).map(|_| rng.next()).collect();
        for &k in &keys {
            map.try_insert(k, k).unwrap();
        }
        let num_keys = keys.len();
        let mut samples = Vec::with_capacity(SAMPLES);
        let mut idx = 0;
        for _ in 0..WARMUP {
            let k = keys[idx % num_keys];
            let v = map.remove(&k).unwrap();
            map.try_insert(k, v).unwrap();
            idx += 1;
        }
        idx = 0;
        for _ in 0..SAMPLES {
            let k = keys[idx % num_keys];
            let s = rdtsc_start();
            let v = black_box(map.remove(&k));
            let e = rdtsc_end();
            samples.push(e - s);
            map.try_insert(k, v.unwrap()).unwrap();
            idx += 1;
        }
        print_row(&format!("remove (steady @{STEADY_SIZE})"), &mut samples);
        map.clear();
    }

    // insert duplicate key (update in place)
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let keys: Vec<u64> = (0..STEADY_SIZE).map(|_| rng.next()).collect();
        for &k in &keys {
            map.try_insert(k, k).unwrap();
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        let mut idx = 0;
        for _ in 0..WARMUP {
            let k = keys[idx % keys.len()];
            map.try_insert(k, 999).unwrap();
            idx += 1;
        }
        idx = 0;
        for _ in 0..SAMPLES {
            let k = keys[idx % keys.len()];
            let s = rdtsc_start();
            black_box(map.try_insert(k, 999));
            let e = rdtsc_end();
            samples.push(e - s);
            idx += 1;
        }
        print_row(&format!("insert dup (steady @{STEADY_SIZE})"), &mut samples);
        map.clear();
    }

    println!();

    // ── ENTRY API ────────────────────────────────────────────────────
    println!("ENTRY API (per-op, cycles)");
    println!("---");

    // entry (occupied)
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let keys: Vec<u64> = (0..STEADY_SIZE).map(|_| rng.next()).collect();
        for &k in &keys {
            map.try_insert(k, k).unwrap();
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        let mut idx = 0;
        for _ in 0..WARMUP {
            let k = keys[idx % keys.len()];
            match map.entry(k) {
                nexus_collections::skiplist::Entry::Occupied(mut o) => {
                    *o.get_mut() += 1;
                }
                _ => unreachable!(),
            }
            idx += 1;
        }
        idx = 0;
        for _ in 0..SAMPLES {
            let k = keys[idx % keys.len()];
            let s = rdtsc_start();
            match map.entry(k) {
                nexus_collections::skiplist::Entry::Occupied(mut o) => {
                    *o.get_mut() += 1;
                }
                _ => unreachable!(),
            }
            let e = rdtsc_end();
            samples.push(e - s);
            idx += 1;
        }
        print_row(&format!("entry occupied (@{STEADY_SIZE})"), &mut samples);
        map.clear();
    }

    // entry (vacant — insert)
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let keys: Vec<u64> = (0..STEADY_SIZE).map(|_| rng.next()).collect();
        for &k in &keys {
            map.try_insert(k, k).unwrap();
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..WARMUP {
            let k = rng.next();
            map.entry(k).or_try_insert(0).ok();
            map.remove(&k);
        }
        for _ in 0..SAMPLES {
            let k = rng.next();
            let s = rdtsc_start();
            black_box(map.entry(k).or_try_insert(0));
            let e = rdtsc_end();
            samples.push(e - s);
            map.remove(&k);
        }
        print_row(
            &format!("entry vacant+insert (@{STEADY_SIZE})"),
            &mut samples,
        );
        map.clear();
    }

    println!();

    // ── POP ──────────────────────────────────────────────────────────
    println!("POP (per-op, cycles)");
    println!("---");

    // pop_first (steady @10k)
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let keys: Vec<u64> = (0..STEADY_SIZE).map(|_| rng.next()).collect();
        for &k in &keys {
            map.try_insert(k, k).unwrap();
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..WARMUP {
            let (k, v) = map.pop_first().unwrap();
            map.try_insert(k, v).unwrap();
        }
        for _ in 0..SAMPLES {
            let s = rdtsc_start();
            let h = black_box(map.pop_first());
            let e = rdtsc_end();
            samples.push(e - s);
            let (k, v) = h.unwrap();
            map.try_insert(k, v).unwrap();
        }
        print_row(&format!("pop_first (@{STEADY_SIZE})"), &mut samples);
        map.clear();
    }

    // pop_last (steady @10k)
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let keys: Vec<u64> = (0..STEADY_SIZE).map(|_| rng.next()).collect();
        for &k in &keys {
            map.try_insert(k, k).unwrap();
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..WARMUP {
            let (k, v) = map.pop_last().unwrap();
            map.try_insert(k, v).unwrap();
        }
        for _ in 0..SAMPLES {
            let s = rdtsc_start();
            let h = black_box(map.pop_last());
            let e = rdtsc_end();
            samples.push(e - s);
            let (k, v) = h.unwrap();
            map.try_insert(k, v).unwrap();
        }
        print_row(&format!("pop_last (@{STEADY_SIZE})"), &mut samples);
        map.clear();
    }

    // first_key_value (batched, read-only)
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        for i in 0..STEADY_SIZE {
            map.try_insert(i as u64, 0).unwrap();
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..WARMUP {
            seq!(_ in 0..100 { black_box(map.first_key_value()); });
        }
        for _ in 0..SAMPLES {
            let s = rdtsc_start();
            seq!(_ in 0..100 { black_box(map.first_key_value()); });
            let e = rdtsc_end();
            samples.push((e - s) / BATCH_READ as u64);
        }
        print_row(&format!("first_key_value (@{STEADY_SIZE})"), &mut samples);
        map.clear();
    }

    println!();

    // ── COLD CHURN ───────────────────────────────────────────────────
    // Simulates order book: insert a new price level, remove an old one.
    println!("CHURN (insert+remove pair, per-op, cycles)");
    println!("---");
    {
        let mut map = sl::SkipList::with_seed(42, sl::Allocator);
        let mut keys: Vec<u64> = (0..STEADY_SIZE).map(|_| rng.next()).collect();
        for &k in &keys {
            map.try_insert(k, k).unwrap();
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        let mut idx = 0;
        for _ in 0..WARMUP {
            let old_k = keys[idx % keys.len()];
            let new_k = rng.next();
            map.remove(&old_k);
            map.try_insert(new_k, new_k).unwrap();
            let n = keys.len();
            keys[idx % n] = new_k;
            idx += 1;
        }
        idx = 0;
        for _ in 0..SAMPLES {
            let old_k = keys[idx % keys.len()];
            let new_k = rng.next();
            let s = rdtsc_start();
            map.remove(&old_k);
            map.try_insert(new_k, new_k).unwrap();
            let e = rdtsc_end();
            samples.push(e - s);
            let n = keys.len();
            keys[idx % n] = new_k;
            idx += 1;
        }
        print_row(&format!("churn (@{STEADY_SIZE})"), &mut samples);
        map.clear();
    }
}
