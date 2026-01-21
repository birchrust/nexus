//! AsciiText performance benchmark.
//!
//! Measures construction and comparison with AsciiString.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_text
//!
//! # With perf stat for IPC/branch analysis:
//! perf stat -r 10 ./target/release/examples/perf_text
//! ```

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench_wide, print_header_wide, print_intro};
use nexus_ascii::{AsciiString, AsciiText};
use std::hint::black_box;

fn main() {
    print_intro("ASCIITEXT BENCHMARK");

    // =========================================================================
    // Construction
    // =========================================================================
    print_header_wide("CONSTRUCTION");

    bench_wide("AsciiText::empty()", || {
        let t: AsciiText<32> = black_box(AsciiText::empty());
        t.len() as u64
    });

    bench_wide("AsciiText::try_from_str (7B)", || {
        let t: AsciiText<32> = AsciiText::try_from(black_box("BTC-USD")).unwrap();
        t.len() as u64
    });

    bench_wide("AsciiText::try_from_str (20B)", || {
        let t: AsciiText<32> = AsciiText::try_from(black_box("ORDER-ID-1234567890")).unwrap();
        t.len() as u64
    });

    bench_wide("AsciiText::try_from_bytes (7B)", || {
        let t: AsciiText<32> = AsciiText::try_from_bytes(black_box(b"BTC-USD")).unwrap();
        t.len() as u64
    });

    bench_wide("AsciiText::try_from_bytes (20B)", || {
        let t: AsciiText<32> = AsciiText::try_from_bytes(black_box(b"ORDER-ID-1234567890")).unwrap();
        t.len() as u64
    });

    // =========================================================================
    // Comparison with AsciiString
    // =========================================================================
    println!();
    print_header_wide("COMPARISON WITH ASCIISTRING");

    bench_wide("AsciiString::try_from (7B)", || {
        let s: AsciiString<32> = AsciiString::try_from(black_box("BTC-USD")).unwrap();
        s.len() as u64
    });

    bench_wide("AsciiText::try_from (7B)", || {
        let t: AsciiText<32> = AsciiText::try_from(black_box("BTC-USD")).unwrap();
        t.len() as u64
    });

    bench_wide("AsciiString::try_from (20B)", || {
        let s: AsciiString<32> = AsciiString::try_from(black_box("ORDER-ID-1234567890")).unwrap();
        s.len() as u64
    });

    bench_wide("AsciiText::try_from (20B)", || {
        let t: AsciiText<32> = AsciiText::try_from(black_box("ORDER-ID-1234567890")).unwrap();
        t.len() as u64
    });

    // =========================================================================
    // Conversion
    // =========================================================================
    println!();
    print_header_wide("CONVERSION");

    let text: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();
    bench_wide("into_ascii_string()", || {
        let s = black_box(text).into_ascii_string();
        s.len() as u64
    });

    let s: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    bench_wide("try_from_ascii_string() (valid)", || {
        let t = AsciiText::try_from_ascii_string(black_box(s)).unwrap();
        t.len() as u64
    });

    // With control character - validation fails
    let s_ctrl: AsciiString<32> = AsciiString::try_from_bytes(b"Hello\x00World").unwrap();
    bench_wide("try_from_ascii_string() (invalid)", || {
        let result = AsciiText::try_from_ascii_string(black_box(s_ctrl));
        result.is_err() as u64
    });

    // =========================================================================
    // Deref access
    // =========================================================================
    println!();
    print_header_wide("DEREF ACCESS");

    let text: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();

    bench_wide("len() via Deref", || black_box(&text).len() as u64);

    bench_wide("as_str() via Deref", || {
        black_box(&text).as_str().len() as u64
    });

    bench_wide("as_bytes() via Deref", || {
        black_box(&text).as_bytes().len() as u64
    });

    // =========================================================================
    // Equality
    // =========================================================================
    println!();
    print_header_wide("EQUALITY");

    let t1: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();
    let t2: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();
    let s1: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();

    bench_wide("AsciiText == AsciiText (same)", || {
        if black_box(&t1) == black_box(&t2) {
            1
        } else {
            0
        }
    });

    bench_wide("AsciiText == AsciiString", || {
        if black_box(t1) == black_box(s1) {
            1
        } else {
            0
        }
    });

    bench_wide("AsciiText == &str", || {
        if black_box(t1) == black_box("BTC-USD") {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // Baseline
    // =========================================================================
    println!();
    print_header_wide("BASELINE");

    // const construction has zero runtime cost
    const STATIC_TEXT: AsciiText<16> = AsciiText::from_static("BTC-USD");
    bench_wide("const from_static (access only)", || {
        black_box(STATIC_TEXT).len() as u64
    });

    println!();
}
