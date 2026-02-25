//! RawAsciiString performance benchmark (batched).
//!
//! Uses 100-op batched measurements with serializing fences for sub-rdtsc-floor
//! resolution. Measures construction, accessors, operations, and promotion cost
//! in CPU cycles. Compares against AsciiString where relevant.
//!
//! Run with:
//! ```bash
//! taskset -c 0 cargo run --release --example perf_raw_string
//! ```

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench_batched, print_header, print_intro};
use nexus_ascii::{AsciiChar, RawAsciiString};
use std::hint::black_box;

fn main() {
    print_intro("RAWASCIISTRING PERFORMANCE BENCHMARK (batched, 100 ops/sample)");

    // =========================================================================
    // Construction
    // =========================================================================
    print_header("CONSTRUCTION");

    bench_batched("empty()", || {
        let s: RawAsciiString<32> = RawAsciiString::empty();
        black_box(s).as_raw()[0] as u64
    });

    bench_batched("try_from str (7B \"BTC-USD\")", || {
        let s: RawAsciiString<32> = RawAsciiString::try_from(black_box("BTC-USD")).unwrap();
        s.as_raw()[0] as u64
    });

    bench_batched("try_from str (20B)", || {
        let s: RawAsciiString<32> =
            RawAsciiString::try_from(black_box("ABCDEFGHIJ1234567890")).unwrap();
        s.as_raw()[0] as u64
    });

    bench_batched("try_from str (32B, full cap)", || {
        let s: RawAsciiString<32> =
            RawAsciiString::try_from(black_box("ABCDEFGHIJKLMNOPQRSTUVWXYZ123456")).unwrap();
        s.as_raw()[0] as u64
    });

    bench_batched("try_from_bytes (7B)", || {
        let s: RawAsciiString<32> = RawAsciiString::try_from_bytes(black_box(b"BTC-USD")).unwrap();
        s.as_raw()[0] as u64
    });

    bench_batched("from_bytes_unchecked (7B)", || {
        let s: RawAsciiString<32> =
            unsafe { RawAsciiString::from_bytes_unchecked(black_box(b"BTC-USD")) };
        s.as_raw()[0] as u64
    });

    // =========================================================================
    // Wire format construction (null-terminated / raw buffer)
    // =========================================================================
    println!();
    print_header("WIRE FORMAT CONSTRUCTION");

    let buffer_7: [u8; 16] = *b"BTC-USD\0\0\0\0\0\0\0\0\0";
    bench_batched("try_from_null_terminated (7B)", || {
        let s: RawAsciiString<16> =
            RawAsciiString::try_from_null_terminated(black_box(&buffer_7[..])).unwrap();
        s.as_raw()[0] as u64
    });

    let buffer_full: [u8; 16] = *b"ABCDEFGHIJKLMNOP";
    bench_batched("try_from_null_terminated (16B full)", || {
        let s: RawAsciiString<16> =
            RawAsciiString::try_from_null_terminated(black_box(&buffer_full[..])).unwrap();
        s.as_raw()[0] as u64
    });

    bench_batched("try_from_raw (7B in 16B)", || {
        let s: RawAsciiString<16> = RawAsciiString::try_from_raw(black_box(buffer_7)).unwrap();
        s.as_raw()[0] as u64
    });

    bench_batched("try_from_raw_ref (7B in &[u8;16])", || {
        let s: RawAsciiString<16> = RawAsciiString::try_from_raw_ref(black_box(&buffer_7)).unwrap();
        s.as_raw()[0] as u64
    });

    bench_batched("from_raw_unchecked (7B in 16B)", || {
        let s: RawAsciiString<16> =
            unsafe { RawAsciiString::from_raw_unchecked(black_box(buffer_7)) };
        s.as_raw()[0] as u64
    });

    let padded: [u8; 16] = *b"BTC-USD         ";
    bench_batched("try_from_right_padded (7B)", || {
        let s: RawAsciiString<16> =
            RawAsciiString::try_from_right_padded(black_box(padded), b' ').unwrap();
        s.as_raw()[0] as u64
    });

    // =========================================================================
    // Accessors — len() is the key cost (SIMD null scan)
    // =========================================================================
    println!();
    print_header("ACCESSORS");

    let s7: RawAsciiString<32> = RawAsciiString::try_from("BTC-USD").unwrap();
    let s20: RawAsciiString<32> = RawAsciiString::try_from("ABCDEFGHIJ1234567890").unwrap();
    let s_full: RawAsciiString<32> =
        RawAsciiString::try_from("ABCDEFGHIJKLMNOPQRSTUVWXYZ123456").unwrap();

    bench_batched("len() (7B content)", || black_box(s7).len() as u64);

    bench_batched("len() (20B content)", || black_box(s20).len() as u64);

    bench_batched("len() (32B full buffer)", || black_box(s_full).len() as u64);

    bench_batched("as_str() (7B)", || black_box(s7).as_str().len() as u64);

    bench_batched("as_bytes() (7B)", || black_box(s7).as_bytes().len() as u64);

    bench_batched(
        "is_empty()",
        || {
            if black_box(s7).is_empty() { 0 } else { 1 }
        },
    );

    // =========================================================================
    // String operations
    // =========================================================================
    println!();
    print_header("OPERATIONS");

    let s: RawAsciiString<32> = RawAsciiString::try_from("BTC-USD").unwrap();

    let padded_s: RawAsciiString<32> = RawAsciiString::try_from("  BTC-USD  ").unwrap();
    bench_batched("trimmed()", || black_box(padded_s).trimmed().len() as u64);

    bench_batched("trimmed_start()", || {
        black_box(padded_s).trimmed_start().len() as u64
    });

    bench_batched("trimmed_end()", || {
        black_box(padded_s).trimmed_end().len() as u64
    });

    bench_batched("contains (found, 3B)", || {
        if black_box(&s).contains(black_box(b"USD")) {
            1
        } else {
            0
        }
    });

    bench_batched("contains (not found)", || {
        if black_box(&s).contains(black_box(b"EUR")) {
            1
        } else {
            0
        }
    });

    bench_batched("split_once (found)", || {
        if black_box(&s).split_once(b'-').is_some() {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // Replacement
    // =========================================================================
    println!();
    print_header("REPLACEMENT");

    let sym: RawAsciiString<32> = RawAsciiString::try_from("BTC-USD-PERP").unwrap();
    let minus = AsciiChar::try_new(b'-').unwrap();
    let underscore = AsciiChar::try_new(b'_').unwrap();

    bench_batched("replaced_char (AsciiChar)", || {
        black_box(sym).replaced_char(minus, underscore).len() as u64
    });

    bench_batched("replace_first_char (AsciiChar)", || {
        black_box(sym).replace_first_char(minus, underscore).len() as u64
    });

    bench_batched("replaced_byte (unsafe)", || {
        unsafe { black_box(sym).replaced_byte(b'-', b'_') }.len() as u64
    });

    bench_batched("replace_first_byte (unsafe)", || {
        unsafe { black_box(sym).replace_first_byte(b'-', b'_') }.len() as u64
    });

    // Multi-byte replacement
    let long: RawAsciiString<32> = RawAsciiString::try_from("foo bar foo baz").unwrap();
    bench_batched("replaced (3B->3B, multi)", || {
        // SAFETY: b"qux" is valid ASCII
        unsafe { black_box(long).replaced(b"foo", b"qux") }.len() as u64
    });

    bench_batched("replace_first (3B->3B)", || {
        // SAFETY: b"qux" is valid ASCII
        unsafe { black_box(long).replace_first(b"foo", b"qux") }.len() as u64
    });

    // =========================================================================
    // Integer parsing / formatting
    // =========================================================================
    println!();
    print_header("INTEGER PARSING / FORMATTING");

    let num_str: RawAsciiString<32> = RawAsciiString::try_from("18446744073709551615").unwrap();
    bench_batched("parse_u64 (20 digits)", || {
        black_box(&num_str).parse_u64().unwrap()
    });

    let i64_str: RawAsciiString<32> = RawAsciiString::try_from("-9223372036854775808").unwrap();
    bench_batched("parse_i64 (negative, 20 chars)", || {
        black_box(&i64_str).parse_i64().unwrap() as u64
    });

    bench_batched("from_u64 (max)", || {
        let s: RawAsciiString<32> = RawAsciiString::from_u64(black_box(u64::MAX)).unwrap();
        s.len() as u64
    });

    // =========================================================================
    // Promotion to AsciiString (adds header/hash)
    // =========================================================================
    println!();
    print_header("PROMOTION (to_ascii_string)");

    bench_batched("to_ascii_string (7B)", || {
        black_box(s7).to_ascii_string().header()
    });

    bench_batched("to_ascii_string (20B)", || {
        black_box(s20).to_ascii_string().header()
    });

    bench_batched("to_ascii_string (32B full)", || {
        black_box(s_full).to_ascii_string().header()
    });

    // =========================================================================
    println!();
}
