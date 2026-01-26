//! Performance benchmark for transformation methods.
//!
//! Measures to_ascii_uppercase, to_ascii_lowercase, and truncated.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_transform
//! ```

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench, print_header, print_intro};
use nexus_ascii::AsciiString;
use std::hint::black_box;

fn main() {
    print_intro("TRANSFORMATION BENCHMARK");

    // =========================================================================
    // Case conversion
    // =========================================================================
    print_header("CASE CONVERSION");

    // Short string (7 bytes)
    let s7: AsciiString<32> = AsciiString::try_from("BtC-uSd").unwrap();

    bench("to_ascii_uppercase (7B)", || {
        let upper = black_box(s7).to_ascii_uppercase();
        upper.len() as u64
    });

    bench("to_ascii_lowercase (7B)", || {
        let lower = black_box(s7).to_ascii_lowercase();
        lower.len() as u64
    });

    // Medium string (20 bytes)
    let s20: AsciiString<32> = AsciiString::try_from("HeLLo WoRLd AbC 1234").unwrap();

    bench("to_ascii_uppercase (20B)", || {
        let upper = black_box(s20).to_ascii_uppercase();
        upper.len() as u64
    });

    bench("to_ascii_lowercase (20B)", || {
        let lower = black_box(s20).to_ascii_lowercase();
        lower.len() as u64
    });

    // Longer string (32 bytes)
    let s32: AsciiString<32> = AsciiString::try_from("AbCdEfGhIjKlMnOpQrStUvWxYz012345").unwrap();

    bench("to_ascii_uppercase (32B)", || {
        let upper = black_box(s32).to_ascii_uppercase();
        upper.len() as u64
    });

    bench("to_ascii_lowercase (32B)", || {
        let lower = black_box(s32).to_ascii_lowercase();
        lower.len() as u64
    });

    // Already correct case (no changes needed)
    let all_upper: AsciiString<32> = AsciiString::try_from("ALREADY UPPERCASE!!").unwrap();
    let all_lower: AsciiString<32> = AsciiString::try_from("already lowercase!!").unwrap();

    bench("to_ascii_uppercase (already upper)", || {
        let upper = black_box(all_upper).to_ascii_uppercase();
        upper.len() as u64
    });

    bench("to_ascii_lowercase (already lower)", || {
        let lower = black_box(all_lower).to_ascii_lowercase();
        lower.len() as u64
    });

    // =========================================================================
    // Truncation
    // =========================================================================
    println!();
    print_header("TRUNCATION");

    let long: AsciiString<64> =
        AsciiString::try_from("Hello, World! This is a longer string for truncation.").unwrap();
    let long_len = long.len(); // 54

    bench("truncated (54B -> 5B)", || {
        let t = black_box(long).truncated(5);
        t.len() as u64
    });

    bench("truncated (54B -> 30B)", || {
        let t = black_box(long).truncated(30);
        t.len() as u64
    });

    bench(
        &format!("truncated ({}B -> {}B, no change)", long_len, long_len),
        || {
            let t = black_box(long).truncated(long_len);
            t.len() as u64
        },
    );

    bench("try_truncated (54B -> 5B)", || {
        let t = black_box(long).try_truncated(5);
        t.map_or(0, |s| s.len() as u64)
    });

    bench("try_truncated (54B -> 100B, fails)", || {
        let t = black_box(long).try_truncated(100);
        t.map_or(0, |s| s.len() as u64)
    });

    // =========================================================================
    // Baselines
    // =========================================================================
    println!();
    print_header("BASELINES");

    // Compare to std's make_ascii_uppercase on a mutable buffer
    let buf = *b"HeLLo WoRLd AbC 1234";
    bench("std make_ascii_uppercase (20B)", || {
        let mut b = black_box(buf);
        b.make_ascii_uppercase();
        b.len() as u64
    });

    bench("std make_ascii_lowercase (20B)", || {
        let mut b = black_box(buf);
        b.make_ascii_lowercase();
        b.len() as u64
    });

    // Construction baseline (what truncation avoids)
    let hello_bytes = b"Hello";
    bench("try_from_bytes (5B, baseline)", || {
        let s: AsciiString<64> = AsciiString::try_from_bytes(black_box(hello_bytes)).unwrap();
        s.len() as u64
    });

    println!();
}
