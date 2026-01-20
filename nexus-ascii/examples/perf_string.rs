//! AsciiString performance benchmark.
//!
//! Measures construction, equality, and hashing performance in CPU cycles.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_string
//!
//! # With perf stat for IPC/branch analysis:
//! perf stat -r 10 ./target/release/examples/perf_string
//! ```

use nexus_ascii::AsciiString;
use std::collections::HashMap;
use std::hint::black_box;

const ITERATIONS: usize = 100_000;
const WARMUP: usize = 10_000;

#[cfg(target_arch = "x86_64")]
fn rdtsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[cfg(not(target_arch = "x86_64"))]
fn rdtsc() -> u64 {
    // Fallback for non-x86
    std::time::Instant::now().elapsed().as_nanos() as u64
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn bench<F: FnMut() -> u64>(name: &str, mut f: F) -> (u64, u64, u64) {
    // Warmup
    for _ in 0..WARMUP {
        black_box(f());
    }

    // Collect samples
    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = rdtsc();
        black_box(f());
        let end = rdtsc();
        samples.push(end.wrapping_sub(start));
    }

    samples.sort_unstable();
    let p50 = percentile(&samples, 50.0);
    let p99 = percentile(&samples, 99.0);
    let p999 = percentile(&samples, 99.9);

    println!("{:<30} {:>8} {:>8} {:>8}", name, p50, p99, p999);
    (p50, p99, p999)
}

fn main() {
    println!("ASCIISTRING PERFORMANCE BENCHMARK");
    println!("==================================\n");
    println!("Iterations: {}, Warmup: {}", ITERATIONS, WARMUP);
    println!("All times in CPU cycles\n");

    // =========================================================================
    // Construction
    // =========================================================================
    println!("=== CONSTRUCTION ===\n");
    println!(
        "{:<30} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(58));

    // Empty
    bench("empty()", || {
        let s: AsciiString<32> = AsciiString::empty();
        black_box(s).header()
    });

    // From str - various sizes
    let input_8 = "BTC-USD!";
    bench("try_from_str (8B)", || {
        let s: AsciiString<32> = AsciiString::try_from(black_box(input_8)).unwrap();
        s.header()
    });

    let input_16 = "ETHUSDT-PERP1234";
    bench("try_from_str (16B)", || {
        let s: AsciiString<32> = AsciiString::try_from(black_box(input_16)).unwrap();
        s.header()
    });

    let input_32 = "ORDER-ID-1234567890123456789012";
    bench("try_from_str (32B)", || {
        let s: AsciiString<64> = AsciiString::try_from(black_box(input_32)).unwrap();
        s.header()
    });

    // Unchecked construction
    let bytes_16 = b"ETHUSDT-PERP1234";
    bench("from_bytes_unchecked (16B)", || {
        let s: AsciiString<32> = unsafe { AsciiString::from_bytes_unchecked(black_box(bytes_16)) };
        s.header()
    });

    // =========================================================================
    // Equality
    // =========================================================================
    println!("\n=== EQUALITY ===\n");
    println!(
        "{:<30} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(58));

    // Equal strings (fast path match, then byte compare)
    let s1: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    let s2: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    bench("eq (equal, 7B)", || {
        if black_box(s1) == black_box(s2) { 1 } else { 0 }
    });

    // Different strings - different hash (fast path reject)
    let s3: AsciiString<32> = AsciiString::try_from("ETH-USD").unwrap();
    bench("eq (different hash)", || {
        if black_box(s1) == black_box(s3) { 1 } else { 0 }
    });

    // Different strings - different length (fast path reject via header)
    let s4: AsciiString<32> = AsciiString::try_from("BTC").unwrap();
    bench("eq (different length)", || {
        if black_box(s1) == black_box(s4) { 1 } else { 0 }
    });

    // Longer equal strings
    let long1: AsciiString<64> = AsciiString::try_from("ORDER-1234567890-ABCDEF").unwrap();
    let long2: AsciiString<64> = AsciiString::try_from("ORDER-1234567890-ABCDEF").unwrap();
    bench("eq (equal, 23B)", || {
        if black_box(long1) == black_box(long2) {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // Hashing
    // =========================================================================
    println!("\n=== HASHING ===\n");
    println!(
        "{:<30} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(58));

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let s: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    bench("Hash::hash (header extract)", || {
        let mut hasher = DefaultHasher::new();
        black_box(&s).hash(&mut hasher);
        hasher.finish()
    });

    // =========================================================================
    // HashMap operations
    // =========================================================================
    println!("\n=== HASHMAP OPERATIONS ===\n");
    println!(
        "{:<30} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(58));

    // Prepare a HashMap with some entries
    let mut map: HashMap<AsciiString<16>, u64> = HashMap::new();
    let symbols = [
        "BTC-USD",
        "ETH-USD",
        "SOL-USD",
        "AVAX-USD",
        "MATIC-USD",
        "LINK-USD",
        "UNI-USD",
        "AAVE-USD",
        "DOGE-USD",
        "SHIB-USD",
    ];
    for (i, sym) in symbols.iter().enumerate() {
        let key: AsciiString<16> = AsciiString::try_from(*sym).unwrap();
        map.insert(key, i as u64);
    }

    let lookup_key: AsciiString<16> = AsciiString::try_from("ETH-USD").unwrap();
    bench("HashMap::get (hit)", || {
        black_box(map.get(black_box(&lookup_key))).map_or(0, |v| *v)
    });

    let miss_key: AsciiString<16> = AsciiString::try_from("XRP-USD").unwrap();
    bench("HashMap::get (miss)", || {
        black_box(map.get(black_box(&miss_key))).map_or(0, |v| *v)
    });

    // =========================================================================
    // Accessors
    // =========================================================================
    println!("\n=== ACCESSORS ===\n");
    println!(
        "{:<30} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(58));

    let s: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();

    bench("len()", || black_box(&s).len() as u64);

    bench(
        "is_empty()",
        || {
            if black_box(&s).is_empty() { 1 } else { 0 }
        },
    );

    bench("as_str()", || black_box(&s).as_str().len() as u64);

    bench("as_bytes()", || black_box(&s).as_bytes().len() as u64);

    // =========================================================================
    // Comparison with raw operations
    // =========================================================================
    println!("\n=== BASELINE COMPARISONS ===\n");
    println!(
        "{:<30} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(58));

    // Raw u64 comparison (what our header compare compiles to)
    let a: u64 = 0x123456789ABCDEF0;
    let b: u64 = 0x123456789ABCDEF0;
    bench("u64 == u64 (baseline)", || {
        if black_box(a) == black_box(b) { 1 } else { 0 }
    });

    // Raw byte slice comparison
    let bytes_a = b"BTC-USD";
    let bytes_b = b"BTC-USD";
    bench("[u8; 7] == [u8; 7] (baseline)", || {
        if black_box(bytes_a) == black_box(bytes_b) {
            1
        } else {
            0
        }
    });

    // std String comparison for reference
    let std_s1 = String::from("BTC-USD");
    let std_s2 = String::from("BTC-USD");
    bench("String == String (std)", || {
        if black_box(&std_s1) == black_box(&std_s2) {
            1
        } else {
            0
        }
    });

    println!();
}
