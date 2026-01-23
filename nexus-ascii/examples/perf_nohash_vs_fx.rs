//! Focused nohash vs fxhash insert comparison.

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench_wide, print_header_wide, print_intro};
use nexus_ascii::AsciiString;
use nohash_hasher::BuildNoHashHasher;
use rustc_hash::FxBuildHasher;
use std::collections::HashMap;
use std::hint::black_box;

type NoHashMap<const CAP: usize, V> =
    HashMap<AsciiString<CAP>, V, BuildNoHashHasher<u64>>;
type FxHashMap<const CAP: usize, V> =
    HashMap<AsciiString<CAP>, V, FxBuildHasher>;

fn make_key<const CAP: usize>(i: usize) -> AsciiString<CAP> {
    AsciiString::try_from(format!("key-{:06}", i).as_str()).unwrap()
}

fn main() {
    print_intro("NOHASH vs FXHASH — INSERT FOCUS");

    for &size in &[1_000, 10_000] {
        print_header_wide(&format!("INSERT n={}", size));

        let keys: Vec<AsciiString<32>> = (0..size).map(make_key).collect();

        bench_wide(
            &format!("nohash: insert (n={})", size),
            || {
                let mut map: NoHashMap<32, u64> =
                    HashMap::with_capacity_and_hasher(size, BuildNoHashHasher::default());
                for (i, k) in keys.iter().enumerate() {
                    map.insert(*k, i as u64);
                }
                black_box(map.len() as u64)
            },
        );

        bench_wide(
            &format!("fxhash: insert (n={})", size),
            || {
                let mut map: FxHashMap<32, u64> =
                    HashMap::with_capacity_and_hasher(size, FxBuildHasher);
                for (i, k) in keys.iter().enumerate() {
                    map.insert(*k, i as u64);
                }
                black_box(map.len() as u64)
            },
        );

        println!();
    }
}
