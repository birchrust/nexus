//! AsciiChar performance benchmark.
//!
//! Measures construction, classification, and transformation performance in CPU cycles.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_char
//! ```

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench, print_header, print_intro};
use nexus_ascii::AsciiChar;
use std::hint::black_box;

fn main() {
    print_intro("ASCIICHAR BENCHMARK");

    // =========================================================================
    // Construction
    // =========================================================================
    print_header("CONSTRUCTION");

    bench("try_new (valid)", || {
        let c = AsciiChar::try_new(black_box(b'A')).unwrap();
        c.as_u8() as u64
    });

    bench("try_new (invalid)", || {
        let r = AsciiChar::try_new(black_box(0x80));
        if r.is_err() { 1 } else { 0 }
    });

    bench("new_unchecked", || {
        let c = unsafe { AsciiChar::new_unchecked(black_box(b'A')) };
        c.as_u8() as u64
    });

    bench("from_char (valid)", || {
        let c = AsciiChar::from_char(black_box('A')).unwrap();
        c.as_u8() as u64
    });

    bench("from_char (invalid)", || {
        let r = AsciiChar::from_char(black_box('é'));
        if r.is_err() { 1 } else { 0 }
    });

    // =========================================================================
    // Accessors
    // =========================================================================
    println!();
    print_header("ACCESSORS");

    let c = AsciiChar::A;

    bench("as_u8", || black_box(c).as_u8() as u64);

    bench("as_char", || black_box(c).as_char() as u64);

    // =========================================================================
    // Classification
    // =========================================================================
    println!();
    print_header("CLASSIFICATION");

    let upper = AsciiChar::A;
    let lower = AsciiChar::a;
    let digit = AsciiChar::DIGIT_5;
    let space = AsciiChar::SPACE;
    let ctrl = AsciiChar::SOH;

    bench("is_uppercase (true)", || {
        if black_box(upper).is_uppercase() {
            1
        } else {
            0
        }
    });

    bench("is_uppercase (false)", || {
        if black_box(lower).is_uppercase() {
            1
        } else {
            0
        }
    });

    bench("is_lowercase", || {
        if black_box(lower).is_lowercase() {
            1
        } else {
            0
        }
    });

    bench("is_alphabetic", || {
        if black_box(upper).is_alphabetic() {
            1
        } else {
            0
        }
    });

    bench(
        "is_digit",
        || {
            if black_box(digit).is_digit() { 1 } else { 0 }
        },
    );

    bench("is_alphanumeric", || {
        if black_box(upper).is_alphanumeric() {
            1
        } else {
            0
        }
    });

    bench("is_whitespace", || {
        if black_box(space).is_whitespace() {
            1
        } else {
            0
        }
    });

    bench("is_printable", || {
        if black_box(upper).is_printable() {
            1
        } else {
            0
        }
    });

    bench("is_control", || {
        if black_box(ctrl).is_control() { 1 } else { 0 }
    });

    bench("is_hex_digit", || {
        if black_box(upper).is_hex_digit() {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // Transformations
    // =========================================================================
    println!();
    print_header("TRANSFORMATIONS");

    bench("to_uppercase (from lower)", || {
        black_box(lower).to_uppercase().as_u8() as u64
    });

    bench("to_uppercase (already upper)", || {
        black_box(upper).to_uppercase().as_u8() as u64
    });

    bench("to_lowercase (from upper)", || {
        black_box(upper).to_lowercase().as_u8() as u64
    });

    bench("to_lowercase (already lower)", || {
        black_box(lower).to_lowercase().as_u8() as u64
    });

    bench("eq_ignore_case (same case)", || {
        if black_box(upper).eq_ignore_case(black_box(upper)) {
            1
        } else {
            0
        }
    });

    bench("eq_ignore_case (diff case)", || {
        if black_box(upper).eq_ignore_case(black_box(lower)) {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // Baseline comparisons
    // =========================================================================
    println!();
    print_header("BASELINE COMPARISONS");

    // Raw u8 comparison
    let a: u8 = b'A';
    let b: u8 = b'A';
    bench("u8 == u8 (baseline)", || {
        if black_box(a) == black_box(b) { 1 } else { 0 }
    });

    // Raw u8 range check (like is_uppercase)
    bench("u8 range check (baseline)", || {
        let x = black_box(a);
        if x >= b'A' && x <= b'Z' { 1 } else { 0 }
    });

    // char.is_ascii_uppercase (std)
    let ch: char = 'A';
    bench("char.is_ascii_uppercase (std)", || {
        if black_box(ch).is_ascii_uppercase() {
            1
        } else {
            0
        }
    });

    // char.to_ascii_lowercase (std)
    bench("char.to_ascii_lowercase (std)", || {
        black_box(ch).to_ascii_lowercase() as u64
    });

    println!();
}
