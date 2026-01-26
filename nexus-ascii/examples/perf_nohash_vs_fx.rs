//! Focused nohash vs fxhash insert comparison.

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench_raw_wide, print_header_wide, print_intro, rdtsc};
use nexus_ascii::AsciiString;
use nohash_hasher::BuildNoHashHasher;
use rustc_hash::FxBuildHasher;
use std::collections::HashMap;
use std::hint::black_box;

type NoHashMap<const CAP: usize, V> = HashMap<AsciiString<CAP>, V, BuildNoHashHasher<u64>>;
type FxHashMap<const CAP: usize, V> = HashMap<AsciiString<CAP>, V, FxBuildHasher>;

fn make_key<const CAP: usize>(i: usize) -> AsciiString<CAP> {
    AsciiString::try_from(format!("key-{:06}", i).as_str()).unwrap()
}

fn main() {
    print_intro("NOHASH vs FXHASH — INSERT FOCUS");

    // Overwrite existing key
    for &size in &[1_000, 10_000] {
        print_header_wide(&format!("OVERWRITE (n={})", size));

        let keys: Vec<AsciiString<32>> = (0..size).map(make_key).collect();
        let overwrite_key = keys[size / 2];

        let mut nohash_map: NoHashMap<32, u64> =
            HashMap::with_capacity_and_hasher(size, BuildNoHashHasher::default());
        for (i, k) in keys.iter().enumerate() {
            nohash_map.insert(*k, i as u64);
        }
        bench_raw_wide(&format!("nohash: overwrite (n={})", size), || {
            let start = rdtsc();
            black_box(nohash_map.insert(overwrite_key, black_box(42u64)));
            rdtsc().wrapping_sub(start)
        });

        let mut fx_map: FxHashMap<32, u64> = HashMap::with_capacity_and_hasher(size, FxBuildHasher);
        for (i, k) in keys.iter().enumerate() {
            fx_map.insert(*k, i as u64);
        }
        bench_raw_wide(&format!("fxhash: overwrite (n={})", size), || {
            let start = rdtsc();
            black_box(fx_map.insert(overwrite_key, black_box(42u64)));
            rdtsc().wrapping_sub(start)
        });

        println!();
    }

    // Insert new key at varying fill levels
    for &capacity in &[1_000, 10_000] {
        let new_key: AsciiString<32> = make_key(capacity + 1);

        for &fill_pct in &[25, 50, 75] {
            print_header_wide(&format!("NEW KEY (cap={}, fill={}%)", capacity, fill_pct));

            let fill = capacity * fill_pct / 100;
            let keys: Vec<AsciiString<32>> = (0..fill).map(make_key).collect();

            let mut nohash_map: NoHashMap<32, u64> =
                HashMap::with_capacity_and_hasher(capacity, BuildNoHashHasher::default());
            for (i, k) in keys.iter().enumerate() {
                nohash_map.insert(*k, i as u64);
            }
            bench_raw_wide(
                &format!("nohash: new key (cap={}, fill={}%)", capacity, fill_pct),
                || {
                    let start = rdtsc();
                    black_box(nohash_map.insert(new_key, black_box(42u64)));
                    let elapsed = rdtsc().wrapping_sub(start);
                    nohash_map.remove(&new_key);
                    elapsed
                },
            );

            let mut fx_map: FxHashMap<32, u64> =
                HashMap::with_capacity_and_hasher(capacity, FxBuildHasher);
            for (i, k) in keys.iter().enumerate() {
                fx_map.insert(*k, i as u64);
            }
            bench_raw_wide(
                &format!("fxhash: new key (cap={}, fill={}%)", capacity, fill_pct),
                || {
                    let start = rdtsc();
                    black_box(fx_map.insert(new_key, black_box(42u64)));
                    let elapsed = rdtsc().wrapping_sub(start);
                    fx_map.remove(&new_key);
                    elapsed
                },
            );

            println!();
        }
    }
}
