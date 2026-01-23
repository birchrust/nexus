//! Serde deserialization performance benchmark.
//!
//! Compares AsciiString vs String deserialization from JSON.
//! Both go through the same JSON parsing, but differ in type construction:
//! - String: heap allocate + memcpy
//! - AsciiString: validate ASCII (SIMD) + compute hash (XXH3) + inline copy
//!
//! Run with:
//! ```bash
//! cargo run --release --features serde --example perf_serde
//! ```

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench_wide, print_header_wide, print_intro};
use nexus_ascii::AsciiString;
use std::hint::black_box;

fn main() {
    print_intro("SERDE DESERIALIZATION BENCHMARK");

    // =========================================================================
    // 7B strings (trading symbols: "BTC-USD")
    // =========================================================================
    print_header_wide("7B STRINGS (trading symbols)");

    let json_7b = "\"BTC-USD\"";

    bench_wide("AsciiString<16> from JSON (7B)", || {
        let s: AsciiString<16> = serde_json::from_str(black_box(json_7b)).unwrap();
        s.len() as u64
    });

    bench_wide("AsciiString<32> from JSON (7B)", || {
        let s: AsciiString<32> = serde_json::from_str(black_box(json_7b)).unwrap();
        s.len() as u64
    });

    bench_wide("String from JSON (7B)", || {
        let s: String = serde_json::from_str(black_box(json_7b)).unwrap();
        s.len() as u64
    });

    bench_wide("&str from JSON (7B)", || {
        let s: &str = serde_json::from_str(black_box(json_7b)).unwrap();
        s.len() as u64
    });

    // Baseline: just the AsciiString construction (no JSON parsing)
    let raw_7b = "BTC-USD";
    bench_wide("AsciiString<32>::try_from_str (7B)", || {
        let s: AsciiString<32> = AsciiString::try_from_str(black_box(raw_7b)).unwrap();
        s.len() as u64
    });

    // =========================================================================
    // 20B strings (order IDs)
    // =========================================================================
    println!();
    print_header_wide("20B STRINGS (order IDs)");

    let json_20b = "\"ORDER-ID-1234567890\"";

    bench_wide("AsciiString<32> from JSON (20B)", || {
        let s: AsciiString<32> = serde_json::from_str(black_box(json_20b)).unwrap();
        s.len() as u64
    });

    bench_wide("AsciiString<64> from JSON (20B)", || {
        let s: AsciiString<64> = serde_json::from_str(black_box(json_20b)).unwrap();
        s.len() as u64
    });

    bench_wide("String from JSON (20B)", || {
        let s: String = serde_json::from_str(black_box(json_20b)).unwrap();
        s.len() as u64
    });

    bench_wide("&str from JSON (20B)", || {
        let s: &str = serde_json::from_str(black_box(json_20b)).unwrap();
        s.len() as u64
    });

    let raw_20b = "ORDER-ID-1234567890";
    bench_wide("AsciiString<32>::try_from_str (20B)", || {
        let s: AsciiString<32> = AsciiString::try_from_str(black_box(raw_20b)).unwrap();
        s.len() as u64
    });

    // =========================================================================
    // 38B strings (long identifiers)
    // =========================================================================
    println!();
    print_header_wide("38B STRINGS (long identifiers)");

    let json_38b = "\"ORDER-ID-ABCDEFGHIJKLMNOPQRSTUVWXYZ12\"";

    bench_wide("AsciiString<64> from JSON (38B)", || {
        let s: AsciiString<64> = serde_json::from_str(black_box(json_38b)).unwrap();
        s.len() as u64
    });

    bench_wide("String from JSON (38B)", || {
        let s: String = serde_json::from_str(black_box(json_38b)).unwrap();
        s.len() as u64
    });

    bench_wide("&str from JSON (38B)", || {
        let s: &str = serde_json::from_str(black_box(json_38b)).unwrap();
        s.len() as u64
    });

    let raw_38b = "ORDER-ID-ABCDEFGHIJKLMNOPQRSTUVWXYZ12";
    bench_wide("AsciiString<64>::try_from_str (38B)", || {
        let s: AsciiString<64> = AsciiString::try_from_str(black_box(raw_38b)).unwrap();
        s.len() as u64
    });

    // =========================================================================
    // 64B strings (large protocol fields)
    // =========================================================================
    println!();
    print_header_wide("64B STRINGS (large protocol fields)");

    let json_64b = "\"ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789-abcdefghijklmnopqrstuvwxyz\"";

    bench_wide("AsciiString<128> from JSON (64B)", || {
        let s: AsciiString<128> = serde_json::from_str(black_box(json_64b)).unwrap();
        s.len() as u64
    });

    bench_wide("String from JSON (64B)", || {
        let s: String = serde_json::from_str(black_box(json_64b)).unwrap();
        s.len() as u64
    });

    bench_wide("&str from JSON (64B)", || {
        let s: &str = serde_json::from_str(black_box(json_64b)).unwrap();
        s.len() as u64
    });

    let raw_64b = "ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789-abcdefghijklmnopqrstuvwxyz";
    bench_wide("AsciiString<128>::try_from_str (64B)", || {
        let s: AsciiString<128> = AsciiString::try_from_str(black_box(raw_64b)).unwrap();
        s.len() as u64
    });

    // =========================================================================
    // Serialization comparison
    // =========================================================================
    println!();
    print_header_wide("SERIALIZATION (to JSON string)");

    let ascii_7b: AsciiString<32> = AsciiString::try_from_str("BTC-USD").unwrap();
    let string_7b = String::from("BTC-USD");

    bench_wide("AsciiString<32> to JSON (7B)", || {
        let s = serde_json::to_string(black_box(&ascii_7b)).unwrap();
        s.len() as u64
    });

    bench_wide("String to JSON (7B)", || {
        let s = serde_json::to_string(black_box(&string_7b)).unwrap();
        s.len() as u64
    });

    let ascii_38b: AsciiString<64> =
        AsciiString::try_from_str("ORDER-ID-ABCDEFGHIJKLMNOPQRSTUVWXYZ12").unwrap();
    let string_38b = String::from("ORDER-ID-ABCDEFGHIJKLMNOPQRSTUVWXYZ12");

    bench_wide("AsciiString<64> to JSON (38B)", || {
        let s = serde_json::to_string(black_box(&ascii_38b)).unwrap();
        s.len() as u64
    });

    bench_wide("String to JSON (38B)", || {
        let s = serde_json::to_string(black_box(&string_38b)).unwrap();
        s.len() as u64
    });

    println!();
}
