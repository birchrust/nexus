//! HashMap performance benchmark with identity hashing.
//!
//! Compares AsciiHashMap (nohash/identity hashing with precomputed XXH3)
//! vs standard HashMap (SipHash) at various map sizes.
//!
//! Run with:
//! ```bash
//! taskset -c 0 cargo run --release --features nohash --example perf_hashmap
//! ```

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench_wide, print_header_wide, print_intro};
use nexus_ascii::AsciiString;
use nohash_hasher::BuildNoHashHasher;
use std::collections::HashMap;
use std::hint::black_box;

type NoHashMap<const CAP: usize, V> =
    HashMap<AsciiString<CAP>, V, BuildNoHashHasher<u64>>;

fn main() {
    print_intro("HASHMAP IDENTITY HASHING BENCHMARK");

    // =========================================================================
    // Varying map sizes — GET (hit)
    // =========================================================================
    println!();
    print_header_wide("GET (hit) — varying map size");

    for &size in &[10, 100, 1_000, 10_000] {
        run_get_hit(size);
        println!();
    }

    // =========================================================================
    // Varying map sizes — GET (miss)
    // =========================================================================
    print_header_wide("GET (miss) — varying map size");

    for &size in &[10, 100, 1_000, 10_000] {
        run_get_miss(size);
        println!();
    }

    // =========================================================================
    // INSERT
    // =========================================================================
    print_header_wide("INSERT — varying map size");

    for &size in &[10, 100, 1_000, 10_000] {
        run_insert(size);
        println!();
    }

    // =========================================================================
    // Varying string sizes (CAP) at fixed map size
    // =========================================================================
    print_header_wide("GET (hit) — varying string size (map=1000)");

    run_get_hit_by_cap::<16>(1000, 8);
    run_get_hit_by_cap::<32>(1000, 8);
    run_get_hit_by_cap::<64>(1000, 8);
    run_get_hit_by_cap::<128>(1000, 8);

    println!();
}

fn make_key<const CAP: usize>(i: usize) -> AsciiString<CAP> {
    AsciiString::try_from(format!("key-{:06}", i).as_str()).unwrap()
}

fn run_get_hit(size: usize) {
    // Identity hash (nohash)
    let mut nohash_map: NoHashMap<32, u64> =
        HashMap::with_capacity_and_hasher(size, BuildNoHashHasher::default());
    for i in 0..size {
        nohash_map.insert(make_key(i), i as u64);
    }
    let lookup_key: AsciiString<32> = make_key(size / 2);

    bench_wide(
        &format!("nohash: get (n={})", size),
        || black_box(*black_box(&nohash_map).get(black_box(&lookup_key)).unwrap()),
    );

    // Default hasher (SipHash)
    let mut default_map: HashMap<AsciiString<32>, u64> = HashMap::with_capacity(size);
    for i in 0..size {
        default_map.insert(make_key(i), i as u64);
    }

    bench_wide(
        &format!("default: get (n={})", size),
        || black_box(*black_box(&default_map).get(black_box(&lookup_key)).unwrap()),
    );

    // std String with default hasher
    let mut string_map: HashMap<String, u64> = HashMap::with_capacity(size);
    for i in 0..size {
        string_map.insert(format!("key-{:06}", i), i as u64);
    }
    let string_key = format!("key-{:06}", size / 2);

    bench_wide(
        &format!("String: get (n={})", size),
        || black_box(*black_box(&string_map).get(black_box(&string_key)).unwrap()),
    );
}

fn run_get_miss(size: usize) {
    let mut nohash_map: NoHashMap<32, u64> =
        HashMap::with_capacity_and_hasher(size, BuildNoHashHasher::default());
    for i in 0..size {
        nohash_map.insert(make_key(i), i as u64);
    }
    let miss_key: AsciiString<32> = AsciiString::try_from("nonexistent-key!").unwrap();

    bench_wide(
        &format!("nohash: get miss (n={})", size),
        || black_box(black_box(&nohash_map).get(black_box(&miss_key))).is_some() as u64,
    );

    let mut default_map: HashMap<AsciiString<32>, u64> = HashMap::with_capacity(size);
    for i in 0..size {
        default_map.insert(make_key(i), i as u64);
    }

    bench_wide(
        &format!("default: get miss (n={})", size),
        || black_box(black_box(&default_map).get(black_box(&miss_key))).is_some() as u64,
    );

    let mut string_map: HashMap<String, u64> = HashMap::with_capacity(size);
    for i in 0..size {
        string_map.insert(format!("key-{:06}", i), i as u64);
    }
    let string_miss = String::from("nonexistent-key!");

    bench_wide(
        &format!("String: get miss (n={})", size),
        || black_box(black_box(&string_map).get(black_box(&string_miss))).is_some() as u64,
    );
}

fn run_insert(size: usize) {
    // Pre-generate keys
    let keys: Vec<AsciiString<32>> = (0..size).map(make_key).collect();
    let string_keys: Vec<String> = (0..size).map(|i| format!("key-{:06}", i)).collect();

    // nohash insert
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

    // default hasher insert
    bench_wide(
        &format!("default: insert (n={})", size),
        || {
            let mut map: HashMap<AsciiString<32>, u64> = HashMap::with_capacity(size);
            for (i, k) in keys.iter().enumerate() {
                map.insert(*k, i as u64);
            }
            black_box(map.len() as u64)
        },
    );

    // String insert
    bench_wide(
        &format!("String: insert (n={})", size),
        || {
            let mut map: HashMap<String, u64> = HashMap::with_capacity(size);
            for (i, k) in string_keys.iter().enumerate() {
                map.insert(k.clone(), i as u64);
            }
            black_box(map.len() as u64)
        },
    );
}

fn run_get_hit_by_cap<const CAP: usize>(size: usize, key_len: usize) {
    let mut nohash_map: NoHashMap<CAP, u64> =
        HashMap::with_capacity_and_hasher(size, BuildNoHashHasher::default());
    for i in 0..size {
        let key: AsciiString<CAP> =
            AsciiString::try_from(format!("key-{:06}", i).as_str()).unwrap();
        nohash_map.insert(key, i as u64);
    }
    let lookup: AsciiString<CAP> =
        AsciiString::try_from(format!("key-{:06}", size / 2).as_str()).unwrap();

    bench_wide(
        &format!("nohash CAP={:<3} (key={}B, n={})", CAP, key_len, size),
        || black_box(*black_box(&nohash_map).get(black_box(&lookup)).unwrap()),
    );

    let mut default_map: HashMap<AsciiString<CAP>, u64> = HashMap::with_capacity(size);
    for i in 0..size {
        let key: AsciiString<CAP> =
            AsciiString::try_from(format!("key-{:06}", i).as_str()).unwrap();
        default_map.insert(key, i as u64);
    }

    bench_wide(
        &format!("default CAP={:<3} (key={}B, n={})", CAP, key_len, size),
        || black_box(*black_box(&default_map).get(black_box(&lookup)).unwrap()),
    );
}
