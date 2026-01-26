//! String operations performance benchmark.
//!
//! Measures performance of split_once, strip_prefix/suffix, is_numeric,
//! is_alphanumeric, integer parsing, and integer formatting.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_string_ops
//! ```

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench, print_header, print_intro};
use nexus_ascii::{AsciiChar, AsciiString, AsciiText};
use std::hint::black_box;

fn main() {
    print_intro("STRING OPERATIONS PERFORMANCE BENCHMARK");

    // =========================================================================
    // split_once
    // =========================================================================
    print_header("SPLIT_ONCE");

    let composite: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    bench("split_once (found)", || {
        let result = black_box(&composite).split_once(AsciiChar::MINUS);
        if result.is_some() { 1 } else { 0 }
    });

    let no_sep: AsciiString<32> = AsciiString::try_from("BTCUSD").unwrap();
    bench("split_once (not found)", || {
        let result = black_box(&no_sep).split_once(AsciiChar::MINUS);
        if result.is_some() { 1 } else { 0 }
    });

    let multi_sep: AsciiString<32> = AsciiString::try_from("A-B-C-D-E").unwrap();
    bench("split_once (multi sep)", || {
        let result = black_box(&multi_sep).split_once(AsciiChar::MINUS);
        if result.is_some() { 1 } else { 0 }
    });

    // =========================================================================
    // strip_prefix / strip_suffix
    // =========================================================================
    println!();
    print_header("STRIP PREFIX/SUFFIX");

    let prefixed: AsciiString<32> = AsciiString::try_from("ORDER-12345").unwrap();
    bench("strip_prefix (found)", || {
        let result = black_box(&prefixed).strip_prefix("ORDER-");
        if result.is_some() { 1 } else { 0 }
    });

    bench("strip_prefix (not found)", || {
        let result = black_box(&prefixed).strip_prefix("TRADE-");
        if result.is_some() { 1 } else { 0 }
    });

    let suffixed: AsciiString<32> = AsciiString::try_from("BTC-PERP").unwrap();
    bench("strip_suffix (found)", || {
        let result = black_box(&suffixed).strip_suffix("-PERP");
        if result.is_some() { 1 } else { 0 }
    });

    bench("strip_suffix (not found)", || {
        let result = black_box(&suffixed).strip_suffix("-SPOT");
        if result.is_some() { 1 } else { 0 }
    });

    // =========================================================================
    // is_numeric / is_alphanumeric
    // =========================================================================
    println!();
    print_header("CHARACTER CLASS PREDICATES");

    let numeric: AsciiString<16> = AsciiString::try_from("12345678").unwrap();
    bench("is_numeric (true, 8B)", || {
        if black_box(&numeric).is_numeric() {
            1
        } else {
            0
        }
    });

    let not_numeric: AsciiString<16> = AsciiString::try_from("1234a678").unwrap();
    bench("is_numeric (false, 8B)", || {
        if black_box(&not_numeric).is_numeric() {
            1
        } else {
            0
        }
    });

    let alphanum: AsciiString<16> = AsciiString::try_from("ABC12345").unwrap();
    bench("is_alphanumeric (true, 8B)", || {
        if black_box(&alphanum).is_alphanumeric() {
            1
        } else {
            0
        }
    });

    let not_alphanum: AsciiString<16> = AsciiString::try_from("ABC-1234").unwrap();
    bench("is_alphanumeric (false, 8B)", || {
        if black_box(&not_alphanum).is_alphanumeric() {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // Integer Parsing
    // =========================================================================
    println!();
    print_header("INTEGER PARSING");

    let u8_str: AsciiString<8> = AsciiString::try_from("255").unwrap();
    bench("parse_u8 (3 digits)", || {
        black_box(&u8_str).parse_u8().unwrap() as u64
    });

    let u32_str: AsciiString<16> = AsciiString::try_from("4294967295").unwrap();
    bench("parse_u32 (10 digits)", || {
        black_box(&u32_str).parse_u32().unwrap() as u64
    });

    let u64_str: AsciiString<32> = AsciiString::try_from("18446744073709551615").unwrap();
    bench("parse_u64 (20 digits)", || {
        black_box(&u64_str).parse_u64().unwrap()
    });

    let i64_str: AsciiString<32> = AsciiString::try_from("-9223372036854775808").unwrap();
    bench("parse_i64 (negative, 20 chars)", || {
        black_box(&i64_str).parse_i64().unwrap() as u64
    });

    // Compare with std::str::parse
    let std_str = "18446744073709551615";
    bench("baseline: str.parse::<u64>()", || {
        black_box(std_str).parse::<u64>().unwrap()
    });

    // =========================================================================
    // Integer Formatting
    // =========================================================================
    println!();
    print_header("INTEGER FORMATTING");

    bench("from_u8 (255)", || {
        let s: AsciiString<8> = AsciiString::from_u8(black_box(255)).unwrap();
        s.len() as u64
    });

    bench("from_u32 (max)", || {
        let s: AsciiString<16> = AsciiString::from_u32(black_box(u32::MAX)).unwrap();
        s.len() as u64
    });

    bench("from_u64 (max)", || {
        let s: AsciiString<32> = AsciiString::from_u64(black_box(u64::MAX)).unwrap();
        s.len() as u64
    });

    bench("from_i64 (min)", || {
        let s: AsciiString<32> = AsciiString::from_i64(black_box(i64::MIN)).unwrap();
        s.len() as u64
    });

    // Compare with itoa crate (if we had it)
    // For now, compare with format!
    bench("baseline: format!(\"{}\", u64)", || {
        let s = format!("{}", black_box(u64::MAX));
        s.len() as u64
    });

    // =========================================================================
    // AsciiText variants
    // =========================================================================
    println!();
    print_header("ASCIITEXT VARIANTS");

    let text: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();
    bench("AsciiText::split_once", || {
        let result = black_box(&text).split_once(AsciiChar::MINUS);
        if result.is_some() { 1 } else { 0 }
    });

    let text_num: AsciiText<16> = AsciiText::try_from("12345678").unwrap();
    bench("AsciiText::parse_u64", || {
        black_box(&text_num).parse_u64().unwrap()
    });

    bench("AsciiText::from_u64", || {
        let s: AsciiText<32> = AsciiText::from_u64(black_box(12345678901234)).unwrap();
        s.len() as u64
    });

    println!("\nDone.");
}
