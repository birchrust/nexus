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

use bench_utils::{bench_raw_wide, bench_wide, print_header_wide, print_intro, rdtsc};
use nexus_ascii::AsciiString;
use nohash_hasher::BuildNoHashHasher;
use rustc_hash::FxBuildHasher;
use std::collections::HashMap;
use std::hint::black_box;

type NoHashMap<const CAP: usize, V> =
    HashMap<AsciiString<CAP>, V, BuildNoHashHasher<u64>>;
type AHashMap<const CAP: usize, V> =
    HashMap<AsciiString<CAP>, V, ahash::RandomState>;
type FxHashMap<const CAP: usize, V> =
    HashMap<AsciiString<CAP>, V, FxBuildHasher>;

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
    // INSERT (overwrite existing key)
    // =========================================================================
    print_header_wide("INSERT (overwrite) — varying map size");

    for &size in &[100, 1_000, 10_000] {
        run_insert_overwrite(size);
        println!();
    }

    // =========================================================================
    // INSERT (new key at varying fill levels)
    // =========================================================================
    print_header_wide("INSERT (new key) — varying fill level (capacity=1000)");
    run_insert_new(1_000);

    println!();
    print_header_wide("INSERT (new key) — varying fill level (capacity=10000)");
    run_insert_new(10_000);

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

    // ahash
    let mut ahash_map: AHashMap<32, u64> =
        HashMap::with_capacity_and_hasher(size, ahash::RandomState::new());
    for i in 0..size {
        ahash_map.insert(make_key(i), i as u64);
    }

    bench_wide(
        &format!("ahash: get (n={})", size),
        || black_box(*black_box(&ahash_map).get(black_box(&lookup_key)).unwrap()),
    );

    // fxhash
    let mut fx_map: FxHashMap<32, u64> =
        HashMap::with_capacity_and_hasher(size, FxBuildHasher);
    for i in 0..size {
        fx_map.insert(make_key(i), i as u64);
    }

    bench_wide(
        &format!("fxhash: get (n={})", size),
        || black_box(*black_box(&fx_map).get(black_box(&lookup_key)).unwrap()),
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

    // ahash
    let mut ahash_map: AHashMap<32, u64> =
        HashMap::with_capacity_and_hasher(size, ahash::RandomState::new());
    for i in 0..size {
        ahash_map.insert(make_key(i), i as u64);
    }

    bench_wide(
        &format!("ahash: get miss (n={})", size),
        || black_box(black_box(&ahash_map).get(black_box(&miss_key))).is_some() as u64,
    );

    // fxhash
    let mut fx_map: FxHashMap<32, u64> =
        HashMap::with_capacity_and_hasher(size, FxBuildHasher);
    for i in 0..size {
        fx_map.insert(make_key(i), i as u64);
    }

    bench_wide(
        &format!("fxhash: get miss (n={})", size),
        || black_box(black_box(&fx_map).get(black_box(&miss_key))).is_some() as u64,
    );

    // std String
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

fn run_insert_overwrite(size: usize) {
    let keys: Vec<AsciiString<32>> = (0..size).map(make_key).collect();
    let overwrite_key = keys[size / 2];
    let string_keys: Vec<String> = (0..size).map(|i| format!("key-{:06}", i)).collect();
    let string_overwrite = string_keys[size / 2].clone();

    // nohash
    let mut nohash_map: NoHashMap<32, u64> =
        HashMap::with_capacity_and_hasher(size, BuildNoHashHasher::default());
    for (i, k) in keys.iter().enumerate() {
        nohash_map.insert(*k, i as u64);
    }
    bench_raw_wide(
        &format!("nohash: overwrite (n={})", size),
        || {
            let start = rdtsc();
            black_box(nohash_map.insert(overwrite_key, black_box(42u64)));
            rdtsc().wrapping_sub(start)
        },
    );

    // default hasher
    let mut default_map: HashMap<AsciiString<32>, u64> = HashMap::with_capacity(size);
    for (i, k) in keys.iter().enumerate() {
        default_map.insert(*k, i as u64);
    }
    bench_raw_wide(
        &format!("default: overwrite (n={})", size),
        || {
            let start = rdtsc();
            black_box(default_map.insert(overwrite_key, black_box(42u64)));
            rdtsc().wrapping_sub(start)
        },
    );

    // ahash
    let mut ahash_map: AHashMap<32, u64> =
        HashMap::with_capacity_and_hasher(size, ahash::RandomState::new());
    for (i, k) in keys.iter().enumerate() {
        ahash_map.insert(*k, i as u64);
    }
    bench_raw_wide(
        &format!("ahash: overwrite (n={})", size),
        || {
            let start = rdtsc();
            black_box(ahash_map.insert(overwrite_key, black_box(42u64)));
            rdtsc().wrapping_sub(start)
        },
    );

    // fxhash
    let mut fx_map: FxHashMap<32, u64> =
        HashMap::with_capacity_and_hasher(size, FxBuildHasher);
    for (i, k) in keys.iter().enumerate() {
        fx_map.insert(*k, i as u64);
    }
    bench_raw_wide(
        &format!("fxhash: overwrite (n={})", size),
        || {
            let start = rdtsc();
            black_box(fx_map.insert(overwrite_key, black_box(42u64)));
            rdtsc().wrapping_sub(start)
        },
    );

    // String
    let mut string_map: HashMap<String, u64> = HashMap::with_capacity(size);
    for (i, k) in string_keys.iter().enumerate() {
        string_map.insert(k.clone(), i as u64);
    }
    bench_raw_wide(
        &format!("String: overwrite (n={})", size),
        || {
            let start = rdtsc();
            black_box(string_map.insert(string_overwrite.clone(), black_box(42u64)));
            rdtsc().wrapping_sub(start)
        },
    );
}

fn run_insert_new(capacity: usize) {
    let new_key: AsciiString<32> = make_key(capacity + 1);
    let string_new_key = format!("key-{:06}", capacity + 1);

    for &fill_pct in &[25, 50, 75] {
        let fill = capacity * fill_pct / 100;
        let keys: Vec<AsciiString<32>> = (0..fill).map(make_key).collect();

        // nohash
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

        // default hasher
        let mut default_map: HashMap<AsciiString<32>, u64> =
            HashMap::with_capacity(capacity);
        for (i, k) in keys.iter().enumerate() {
            default_map.insert(*k, i as u64);
        }
        bench_raw_wide(
            &format!("default: new key (cap={}, fill={}%)", capacity, fill_pct),
            || {
                let start = rdtsc();
                black_box(default_map.insert(new_key, black_box(42u64)));
                let elapsed = rdtsc().wrapping_sub(start);
                default_map.remove(&new_key);
                elapsed
            },
        );

        // ahash
        let mut ahash_map: AHashMap<32, u64> =
            HashMap::with_capacity_and_hasher(capacity, ahash::RandomState::new());
        for (i, k) in keys.iter().enumerate() {
            ahash_map.insert(*k, i as u64);
        }
        bench_raw_wide(
            &format!("ahash: new key (cap={}, fill={}%)", capacity, fill_pct),
            || {
                let start = rdtsc();
                black_box(ahash_map.insert(new_key, black_box(42u64)));
                let elapsed = rdtsc().wrapping_sub(start);
                ahash_map.remove(&new_key);
                elapsed
            },
        );

        // fxhash
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

        // String
        let mut string_map: HashMap<String, u64> = HashMap::with_capacity(capacity);
        for (i, k) in keys.iter().enumerate() {
            string_map.insert(format!("key-{:06}", i), k.len() as u64);
        }
        bench_raw_wide(
            &format!("String: new key (cap={}, fill={}%)", capacity, fill_pct),
            || {
                let start = rdtsc();
                black_box(string_map.insert(string_new_key.clone(), black_box(42u64)));
                let elapsed = rdtsc().wrapping_sub(start);
                string_map.remove(&string_new_key);
                elapsed
            },
        );

        println!();
    }
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

    let mut ahash_map: AHashMap<CAP, u64> =
        HashMap::with_capacity_and_hasher(size, ahash::RandomState::new());
    for i in 0..size {
        let key: AsciiString<CAP> =
            AsciiString::try_from(format!("key-{:06}", i).as_str()).unwrap();
        ahash_map.insert(key, i as u64);
    }

    bench_wide(
        &format!("ahash CAP={:<3} (key={}B, n={})", CAP, key_len, size),
        || black_box(*black_box(&ahash_map).get(black_box(&lookup)).unwrap()),
    );

    let mut fx_map: FxHashMap<CAP, u64> =
        HashMap::with_capacity_and_hasher(size, FxBuildHasher);
    for i in 0..size {
        let key: AsciiString<CAP> =
            AsciiString::try_from(format!("key-{:06}", i).as_str()).unwrap();
        fx_map.insert(key, i as u64);
    }

    bench_wide(
        &format!("fxhash CAP={:<3} (key={}B, n={})", CAP, key_len, size),
        || black_box(*black_box(&fx_map).get(black_box(&lookup)).unwrap()),
    );
}
