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

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench_wide, print_header_wide, print_intro};
use nexus_ascii::AsciiString;
use std::hint::black_box;

fn main() {
    print_intro("COMPARISON & ORDERING BENCHMARK");

    // =========================================================================
    // Ordering (Ord)
    // =========================================================================
    print_header_wide("ORDERING (Ord)");

    let s1: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    let s2: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    let s3: AsciiString<32> = AsciiString::try_from("ETH-USD").unwrap();
    let s4: AsciiString<32> = AsciiString::try_from("BTC").unwrap();

    bench_wide("cmp() equal strings (7B)", || black_box(s1.cmp(&s2)) as u64);

    bench_wide("cmp() different strings (7B)", || {
        black_box(s1.cmp(&s3)) as u64
    });

    bench_wide("cmp() different lengths (7B vs 3B)", || {
        black_box(s1.cmp(&s4)) as u64
    });

    let long1: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-1234567890123456789012345678").unwrap();
    let long2: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-1234567890123456789012345678").unwrap();
    let long3: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-1234567890123456789012345679").unwrap();

    bench_wide("cmp() equal strings (37B)", || {
        black_box(long1.cmp(&long2)) as u64
    });

    bench_wide("cmp() differ at end (37B)", || {
        black_box(long1.cmp(&long3)) as u64
    });

    // =========================================================================
    // Case-insensitive equality
    // =========================================================================
    println!();
    print_header_wide("CASE-INSENSITIVE EQUALITY");

    let upper: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    let lower: AsciiString<32> = AsciiString::try_from("btc-usd").unwrap();
    let mixed: AsciiString<32> = AsciiString::try_from("Btc-Usd").unwrap();
    let diff: AsciiString<32> = AsciiString::try_from("ETH-USD").unwrap();
    let shorter: AsciiString<32> = AsciiString::try_from("BTC").unwrap();

    bench_wide("eq_ignore_ascii_case() same case (7B)", || {
        u64::from(black_box(&upper).eq_ignore_ascii_case(black_box(&upper)))
    });

    bench_wide("eq_ignore_ascii_case() diff case (7B)", || {
        u64::from(black_box(&upper).eq_ignore_ascii_case(black_box(&lower)))
    });

    bench_wide("eq_ignore_ascii_case() mixed case (7B)", || {
        u64::from(black_box(&upper).eq_ignore_ascii_case(black_box(&mixed)))
    });

    bench_wide("eq_ignore_ascii_case() different (7B)", || {
        u64::from(black_box(&upper).eq_ignore_ascii_case(black_box(&diff)))
    });

    bench_wide("eq_ignore_ascii_case() diff len (fast)", || {
        u64::from(black_box(&upper).eq_ignore_ascii_case(black_box(&shorter)))
    });

    let long_upper: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-ABCDEFGHIJKLMNOPQRSTUVWXYZ12").unwrap();
    let long_lower: AsciiString<64> =
        AsciiString::try_from("order-id-abcdefghijklmnopqrstuvwxyz12").unwrap();

    bench_wide("eq_ignore_ascii_case() same case (38B)", || {
        u64::from(black_box(&long_upper).eq_ignore_ascii_case(black_box(&long_upper)))
    });

    bench_wide("eq_ignore_ascii_case() diff case (38B)", || {
        u64::from(black_box(&long_upper).eq_ignore_ascii_case(black_box(&long_lower)))
    });

    // 72B strings: above the 64B SIMD crossover threshold
    let xl_upper: AsciiString<128> = AsciiString::try_from(
        "ORDER-ID-ABCDEFGHIJKLMNOPQRSTUVWXYZ-ABCDEFGHIJKLMNOPQRSTUVWXYZ-12345",
    )
    .unwrap();
    let xl_lower: AsciiString<128> = AsciiString::try_from(
        "order-id-abcdefghijklmnopqrstuvwxyz-abcdefghijklmnopqrstuvwxyz-12345",
    )
    .unwrap();

    bench_wide("eq_ignore_ascii_case() same case (69B)", || {
        u64::from(black_box(&xl_upper).eq_ignore_ascii_case(black_box(&xl_upper)))
    });

    bench_wide("eq_ignore_ascii_case() diff case (69B)", || {
        u64::from(black_box(&xl_upper).eq_ignore_ascii_case(black_box(&xl_lower)))
    });

    // =========================================================================
    // starts_with
    // =========================================================================
    println!();
    print_header_wide("STARTS_WITH");

    let symbol: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();

    bench_wide("starts_with() 3B prefix (match)", || {
        u64::from(black_box(&symbol).starts_with(black_box("BTC")))
    });

    bench_wide("starts_with() 3B prefix (no match)", || {
        u64::from(black_box(&symbol).starts_with(black_box("ETH")))
    });

    bench_wide("starts_with() full string", || {
        u64::from(black_box(&symbol).starts_with(black_box("BTC-USD")))
    });

    bench_wide("starts_with() empty prefix", || {
        u64::from(black_box(&symbol).starts_with(black_box("")))
    });

    bench_wide("starts_with() longer prefix (no match)", || {
        u64::from(black_box(&symbol).starts_with(black_box("BTC-USD-PERP")))
    });

    let long_str: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-1234567890123456789012345678").unwrap();

    bench_wide("starts_with() 8B prefix (37B string)", || {
        u64::from(black_box(&long_str).starts_with(black_box("ORDER-ID")))
    });

    // =========================================================================
    // ends_with
    // =========================================================================
    println!();
    print_header_wide("ENDS_WITH");

    bench_wide("ends_with() 3B suffix (match)", || {
        u64::from(black_box(&symbol).ends_with(black_box("USD")))
    });

    bench_wide("ends_with() 3B suffix (no match)", || {
        u64::from(black_box(&symbol).ends_with(black_box("EUR")))
    });

    bench_wide("ends_with() full string", || {
        u64::from(black_box(&symbol).ends_with(black_box("BTC-USD")))
    });

    bench_wide("ends_with() empty suffix", || {
        u64::from(black_box(&symbol).ends_with(black_box("")))
    });

    bench_wide("ends_with() 8B suffix (37B string)", || {
        u64::from(black_box(&long_str).ends_with(black_box("45678")))
    });

    // =========================================================================
    // contains
    // =========================================================================
    println!();
    print_header_wide("CONTAINS");

    bench_wide("contains() 1B needle (match)", || {
        u64::from(black_box(&symbol).contains(black_box("-")))
    });

    bench_wide("contains() 1B needle (no match)", || {
        u64::from(black_box(&symbol).contains(black_box("@")))
    });

    bench_wide("contains() 3B needle at start", || {
        u64::from(black_box(&symbol).contains(black_box("BTC")))
    });

    bench_wide("contains() 3B needle at end", || {
        u64::from(black_box(&symbol).contains(black_box("USD")))
    });

    bench_wide("contains() 3B needle in middle", || {
        u64::from(black_box(&symbol).contains(black_box("C-U")))
    });

    bench_wide("contains() full string", || {
        u64::from(black_box(&symbol).contains(black_box("BTC-USD")))
    });

    bench_wide("contains() empty needle", || {
        u64::from(black_box(&symbol).contains(black_box("")))
    });

    bench_wide("contains() 5B needle (37B string, match)", || {
        u64::from(black_box(&long_str).contains(black_box("12345")))
    });

    bench_wide("contains() 5B needle (37B string, no match)", || {
        u64::from(black_box(&long_str).contains(black_box("ZZZZZ")))
    });

    // =========================================================================
    // Baseline comparisons
    // =========================================================================
    println!();
    print_header_wide("BASELINE COMPARISONS");

    let bytes1: &[u8] = b"BTC-USD";
    let bytes2: &[u8] = b"BTC-USD";
    let bytes3: &[u8] = b"ETH-USD";

    bench_wide("[u8] cmp() equal (baseline)", || {
        black_box(bytes1.cmp(bytes2)) as u64
    });

    bench_wide("[u8] cmp() different (baseline)", || {
        black_box(bytes1.cmp(bytes3)) as u64
    });

    let str1: &str = "BTC-USD";
    let str2: &str = "btc-usd";

    bench_wide("&str eq_ignore_ascii_case (baseline)", || {
        u64::from(black_box(str1).eq_ignore_ascii_case(black_box(str2)))
    });

    bench_wide("&str starts_with (baseline)", || {
        u64::from(black_box(str1).starts_with(black_box("BTC")))
    });

    bench_wide("&str ends_with (baseline)", || {
        u64::from(black_box(str1).ends_with(black_box("USD")))
    });

    bench_wide("&str contains (baseline)", || {
        u64::from(black_box(str1).contains(black_box("-")))
    });

    println!();
}
