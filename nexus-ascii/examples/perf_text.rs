//! AsciiText performance benchmark (batched).
//!
//! Uses 100-op batched measurements with serializing fences for sub-rdtsc-floor
//! resolution. Measures construction, conversion, and equality in CPU cycles.
//!
//! Run with:
//! ```bash
//! taskset -c 0 cargo run --release --example perf_text
//! ```

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench_batched, print_header, print_intro};
use nexus_ascii::{AsciiString, AsciiText};
use std::hint::black_box;

fn main() {
    print_intro("ASCIITEXT BENCHMARK (batched, 100 ops/sample)");

    // =========================================================================
    // Construction
    // =========================================================================
    print_header("CONSTRUCTION");

    bench_batched("empty()", || {
        let t: AsciiText<32> = black_box(AsciiText::empty());
        t.as_raw()[0] as u64
    });

    bench_batched("try_from_str (7B)", || {
        let t: AsciiText<32> = AsciiText::try_from(black_box("BTC-USD")).unwrap();
        t.as_raw()[0] as u64
    });

    bench_batched("try_from_str (20B)", || {
        let t: AsciiText<32> = AsciiText::try_from(black_box("ORDER-ID-1234567890")).unwrap();
        t.as_raw()[0] as u64
    });

    bench_batched("try_from_bytes (7B)", || {
        let t: AsciiText<32> = AsciiText::try_from_bytes(black_box(b"BTC-USD")).unwrap();
        t.as_raw()[0] as u64
    });

    bench_batched("try_from_bytes (20B)", || {
        let t: AsciiText<32> =
            AsciiText::try_from_bytes(black_box(b"ORDER-ID-1234567890")).unwrap();
        t.as_raw()[0] as u64
    });

    // Wire format
    let buffer_7: [u8; 16] = *b"BTC-USD\0\0\0\0\0\0\0\0\0";
    bench_batched("try_from_raw (7B in 16B)", || {
        let t: AsciiText<16> = AsciiText::try_from_raw(black_box(buffer_7)).unwrap();
        t.as_raw()[0] as u64
    });

    // =========================================================================
    // Conversion
    // =========================================================================
    println!();
    print_header("CONVERSION");

    let text: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();
    bench_batched("into_ascii_string()", || {
        black_box(text).into_ascii_string().as_raw()[0] as u64
    });

    let s: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    bench_batched("try_from_ascii_string() (valid)", || {
        AsciiText::try_from_ascii_string(black_box(s))
            .unwrap()
            .as_raw()[0] as u64
    });

    // With control character - validation fails
    let s_ctrl: AsciiString<32> = AsciiString::try_from_bytes(b"Hello\x00World").unwrap();
    bench_batched("try_from_ascii_string() (invalid)", || {
        let result = AsciiText::try_from_ascii_string(black_box(s_ctrl));
        result.is_err() as u64
    });

    // =========================================================================
    // Equality
    // =========================================================================
    println!();
    print_header("EQUALITY");

    let t1: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();
    let t2: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();
    let s1: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();

    bench_batched("AsciiText == AsciiText (same)", || {
        u64::from(black_box(&t1) == black_box(&t2))
    });

    bench_batched("AsciiText == AsciiString", || {
        u64::from(black_box(t1) == black_box(s1))
    });

    bench_batched("AsciiText == &str", || {
        u64::from(black_box(t1) == black_box("BTC-USD"))
    });

    println!();
}
