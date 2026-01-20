//! Comparison and ordering performance benchmark.
//!
//! Measures Ord, eq_ignore_ascii_case, starts_with, ends_with, and contains
//! performance in CPU cycles.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_comparison
//!
//! # With perf stat for IPC/branch analysis:
//! perf stat -r 10 ./target/release/examples/perf_comparison
//! ```

use nexus_ascii::AsciiString;
use std::hint::black_box;

const ITERATIONS: usize = 100_000;
const WARMUP: usize = 10_000;

#[cfg(target_arch = "x86_64")]
fn rdtsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[cfg(not(target_arch = "x86_64"))]
fn rdtsc() -> u64 {
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

    println!("{:<40} {:>8} {:>8} {:>8}", name, p50, p99, p999);
    (p50, p99, p999)
}

fn main() {
    println!("COMPARISON & ORDERING BENCHMARK");
    println!("================================\n");
    println!("Iterations: {}, Warmup: {}", ITERATIONS, WARMUP);
    println!("All times in CPU cycles\n");

    // =========================================================================
    // Ordering (Ord)
    // =========================================================================
    println!("=== ORDERING (Ord) ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    let s1: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    let s2: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    let s3: AsciiString<32> = AsciiString::try_from("ETH-USD").unwrap();
    let s4: AsciiString<32> = AsciiString::try_from("BTC").unwrap();

    bench("cmp() equal strings (7B)", || {
        black_box(s1.cmp(&s2)) as u64
    });

    bench("cmp() different strings (7B)", || {
        black_box(s1.cmp(&s3)) as u64
    });

    bench("cmp() different lengths (7B vs 3B)", || {
        black_box(s1.cmp(&s4)) as u64
    });

    let long1: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-1234567890123456789012345678").unwrap();
    let long2: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-1234567890123456789012345678").unwrap();
    let long3: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-1234567890123456789012345679").unwrap();

    bench("cmp() equal strings (37B)", || {
        black_box(long1.cmp(&long2)) as u64
    });

    bench("cmp() differ at end (37B)", || {
        black_box(long1.cmp(&long3)) as u64
    });

    // =========================================================================
    // Case-insensitive equality
    // =========================================================================
    println!("\n=== CASE-INSENSITIVE EQUALITY ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    let upper: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    let lower: AsciiString<32> = AsciiString::try_from("btc-usd").unwrap();
    let mixed: AsciiString<32> = AsciiString::try_from("Btc-Usd").unwrap();
    let diff: AsciiString<32> = AsciiString::try_from("ETH-USD").unwrap();
    let shorter: AsciiString<32> = AsciiString::try_from("BTC").unwrap();

    bench("eq_ignore_ascii_case() same case (7B)", || {
        if black_box(&upper).eq_ignore_ascii_case(black_box(&upper)) {
            1
        } else {
            0
        }
    });

    bench("eq_ignore_ascii_case() diff case (7B)", || {
        if black_box(&upper).eq_ignore_ascii_case(black_box(&lower)) {
            1
        } else {
            0
        }
    });

    bench("eq_ignore_ascii_case() mixed case (7B)", || {
        if black_box(&upper).eq_ignore_ascii_case(black_box(&mixed)) {
            1
        } else {
            0
        }
    });

    bench("eq_ignore_ascii_case() different (7B)", || {
        if black_box(&upper).eq_ignore_ascii_case(black_box(&diff)) {
            1
        } else {
            0
        }
    });

    bench("eq_ignore_ascii_case() diff len (fast)", || {
        if black_box(&upper).eq_ignore_ascii_case(black_box(&shorter)) {
            1
        } else {
            0
        }
    });

    let long_upper: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-ABCDEFGHIJKLMNOPQRSTUVWXYZ12").unwrap();
    let long_lower: AsciiString<64> =
        AsciiString::try_from("order-id-abcdefghijklmnopqrstuvwxyz12").unwrap();

    bench("eq_ignore_ascii_case() same case (38B)", || {
        if black_box(&long_upper).eq_ignore_ascii_case(black_box(&long_upper)) {
            1
        } else {
            0
        }
    });

    bench("eq_ignore_ascii_case() diff case (38B)", || {
        if black_box(&long_upper).eq_ignore_ascii_case(black_box(&long_lower)) {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // starts_with
    // =========================================================================
    println!("\n=== STARTS_WITH ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    let symbol: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();

    bench("starts_with() 3B prefix (match)", || {
        if black_box(&symbol).starts_with(black_box("BTC")) {
            1
        } else {
            0
        }
    });

    bench("starts_with() 3B prefix (no match)", || {
        if black_box(&symbol).starts_with(black_box("ETH")) {
            1
        } else {
            0
        }
    });

    bench("starts_with() full string", || {
        if black_box(&symbol).starts_with(black_box("BTC-USD")) {
            1
        } else {
            0
        }
    });

    bench("starts_with() empty prefix", || {
        if black_box(&symbol).starts_with(black_box("")) {
            1
        } else {
            0
        }
    });

    bench("starts_with() longer prefix (no match)", || {
        if black_box(&symbol).starts_with(black_box("BTC-USD-PERP")) {
            1
        } else {
            0
        }
    });

    let long_str: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-1234567890123456789012345678").unwrap();

    bench("starts_with() 8B prefix (37B string)", || {
        if black_box(&long_str).starts_with(black_box("ORDER-ID")) {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // ends_with
    // =========================================================================
    println!("\n=== ENDS_WITH ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    bench("ends_with() 3B suffix (match)", || {
        if black_box(&symbol).ends_with(black_box("USD")) {
            1
        } else {
            0
        }
    });

    bench("ends_with() 3B suffix (no match)", || {
        if black_box(&symbol).ends_with(black_box("EUR")) {
            1
        } else {
            0
        }
    });

    bench("ends_with() full string", || {
        if black_box(&symbol).ends_with(black_box("BTC-USD")) {
            1
        } else {
            0
        }
    });

    bench("ends_with() empty suffix", || {
        if black_box(&symbol).ends_with(black_box("")) {
            1
        } else {
            0
        }
    });

    bench("ends_with() 8B suffix (37B string)", || {
        if black_box(&long_str).ends_with(black_box("45678")) {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // contains
    // =========================================================================
    println!("\n=== CONTAINS ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    bench("contains() 1B needle (match)", || {
        if black_box(&symbol).contains(black_box("-")) {
            1
        } else {
            0
        }
    });

    bench("contains() 1B needle (no match)", || {
        if black_box(&symbol).contains(black_box("@")) {
            1
        } else {
            0
        }
    });

    bench("contains() 3B needle at start", || {
        if black_box(&symbol).contains(black_box("BTC")) {
            1
        } else {
            0
        }
    });

    bench("contains() 3B needle at end", || {
        if black_box(&symbol).contains(black_box("USD")) {
            1
        } else {
            0
        }
    });

    bench("contains() 3B needle in middle", || {
        if black_box(&symbol).contains(black_box("C-U")) {
            1
        } else {
            0
        }
    });

    bench("contains() full string", || {
        if black_box(&symbol).contains(black_box("BTC-USD")) {
            1
        } else {
            0
        }
    });

    bench("contains() empty needle", || {
        if black_box(&symbol).contains(black_box("")) {
            1
        } else {
            0
        }
    });

    bench("contains() 5B needle (37B string, match)", || {
        if black_box(&long_str).contains(black_box("12345")) {
            1
        } else {
            0
        }
    });

    bench("contains() 5B needle (37B string, no match)", || {
        if black_box(&long_str).contains(black_box("ZZZZZ")) {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // Baseline comparisons
    // =========================================================================
    println!("\n=== BASELINE COMPARISONS ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    let bytes1: &[u8] = b"BTC-USD";
    let bytes2: &[u8] = b"BTC-USD";
    let bytes3: &[u8] = b"ETH-USD";

    bench("[u8] cmp() equal (baseline)", || {
        black_box(bytes1.cmp(bytes2)) as u64
    });

    bench("[u8] cmp() different (baseline)", || {
        black_box(bytes1.cmp(bytes3)) as u64
    });

    let str1: &str = "BTC-USD";
    let str2: &str = "btc-usd";

    bench("&str eq_ignore_ascii_case (baseline)", || {
        if black_box(str1).eq_ignore_ascii_case(black_box(str2)) {
            1
        } else {
            0
        }
    });

    bench("&str starts_with (baseline)", || {
        if black_box(str1).starts_with(black_box("BTC")) {
            1
        } else {
            0
        }
    });

    bench("&str ends_with (baseline)", || {
        if black_box(str1).ends_with(black_box("USD")) {
            1
        } else {
            0
        }
    });

    bench("&str contains (baseline)", || {
        if black_box(str1).contains(black_box("-")) {
            1
        } else {
            0
        }
    });

    println!();
}
