//! Tail latency baseline benchmark for targeted optimizations.
//!
//! Measures eq_ignore_ascii_case, contains_control_chars, and is_all_printable
//! with focus on p99/p999 variance.
//!
//! Run with:
//! ```bash
//! taskset -c 0 cargo run --release --example perf_tail_baseline
//! ```

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench_wide, print_header_wide, print_intro};
use nexus_ascii::AsciiString;
use std::hint::black_box;

fn main() {
    print_intro("TAIL LATENCY BASELINE");

    // =========================================================================
    // eq_ignore_ascii_case - various lengths and patterns
    // =========================================================================
    print_header_wide("EQ_IGNORE_ASCII_CASE");

    // 7 byte strings (typical ticker)
    let upper7: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    let lower7: AsciiString<32> = AsciiString::try_from("btc-usd").unwrap();
    let mixed7: AsciiString<32> = AsciiString::try_from("BtC-uSd").unwrap();

    bench_wide("7B: same (fast path)", || {
        black_box(&upper7).eq_ignore_ascii_case(black_box(&upper7)) as u64
    });

    bench_wide("7B: all diff case", || {
        black_box(&upper7).eq_ignore_ascii_case(black_box(&lower7)) as u64
    });

    bench_wide("7B: mixed case", || {
        black_box(&upper7).eq_ignore_ascii_case(black_box(&mixed7)) as u64
    });

    // 15 byte strings (SWAR boundary - 1 full word + 7 remainder)
    let upper15: AsciiString<32> = AsciiString::try_from("BTC-USD-PERPETL").unwrap();
    let lower15: AsciiString<32> = AsciiString::try_from("btc-usd-perpetl").unwrap();

    bench_wide("15B: all diff case (1 word + 7 remainder)", || {
        black_box(&upper15).eq_ignore_ascii_case(black_box(&lower15)) as u64
    });

    // 16 byte strings (exactly 2 SWAR words)
    let upper16: AsciiString<32> = AsciiString::try_from("BTC-USD-PERPETUA").unwrap();
    let lower16: AsciiString<32> = AsciiString::try_from("btc-usd-perpetua").unwrap();

    bench_wide("16B: all diff case (2 full words)", || {
        black_box(&upper16).eq_ignore_ascii_case(black_box(&lower16)) as u64
    });

    // 38 byte strings (4 words + 6 remainder)
    let upper38: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-ABCDEFGHIJKLMNOPQRSTUVWXYZ12").unwrap();
    let lower38: AsciiString<64> =
        AsciiString::try_from("order-id-abcdefghijklmnopqrstuvwxyz12").unwrap();

    bench_wide("38B: all diff case (4 words + 6 remainder)", || {
        black_box(&upper38).eq_ignore_ascii_case(black_box(&lower38)) as u64
    });

    // Worst case: differ only in last byte remainder
    let upper_last: AsciiString<32> = AsciiString::try_from("btc-usD").unwrap();
    let lower_last: AsciiString<32> = AsciiString::try_from("btc-usd").unwrap();

    bench_wide("7B: differ only in last byte", || {
        black_box(&upper_last).eq_ignore_ascii_case(black_box(&lower_last)) as u64
    });

    // =========================================================================
    // contains_control_chars
    // =========================================================================
    println!();
    print_header_wide("CONTAINS_CONTROL_CHARS");

    let clean7: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    let clean38: AsciiString<64> =
        AsciiString::try_from("ORDER-ID-ABCDEFGHIJKLMNOPQRSTUVWXYZ12").unwrap();

    bench_wide("7B: no control chars", || {
        black_box(&clean7).contains_control_chars() as u64
    });

    bench_wide("38B: no control chars", || {
        black_box(&clean38).contains_control_chars() as u64
    });

    // With control char at various positions
    let ctrl_start: AsciiString<32> = AsciiString::try_from_bytes(b"\x01TC-USD").unwrap();
    let ctrl_end: AsciiString<32> = AsciiString::try_from_bytes(b"BTC-US\x01").unwrap();

    bench_wide("7B: control at start", || {
        black_box(&ctrl_start).contains_control_chars() as u64
    });

    bench_wide("7B: control at end", || {
        black_box(&ctrl_end).contains_control_chars() as u64
    });

    // =========================================================================
    // is_all_printable
    // =========================================================================
    println!();
    print_header_wide("IS_ALL_PRINTABLE");

    bench_wide("7B: all printable", || {
        black_box(&clean7).is_all_printable() as u64
    });

    bench_wide("38B: all printable", || {
        black_box(&clean38).is_all_printable() as u64
    });

    // With non-printable at various positions
    bench_wide("7B: non-printable at start", || {
        black_box(&ctrl_start).is_all_printable() as u64
    });

    bench_wide("7B: non-printable at end", || {
        black_box(&ctrl_end).is_all_printable() as u64
    });

    // =========================================================================
    // Stdlib baseline for comparison
    // =========================================================================
    println!();
    print_header_wide("STDLIB BASELINE");

    let str_upper: &str = "BTC-USD";
    let str_lower: &str = "btc-usd";
    let bytes_clean: &[u8] = b"BTC-USD";
    let bytes_ctrl: &[u8] = b"\x01TC-USD";

    bench_wide("&str eq_ignore_ascii_case 7B", || {
        black_box(str_upper).eq_ignore_ascii_case(black_box(str_lower)) as u64
    });

    let str_upper38 = "ORDER-ID-ABCDEFGHIJKLMNOPQRSTUVWXYZ12";
    let str_lower38 = "order-id-abcdefghijklmnopqrstuvwxyz12";

    bench_wide("&str eq_ignore_ascii_case 38B", || {
        black_box(str_upper38).eq_ignore_ascii_case(black_box(str_lower38)) as u64
    });

    bench_wide("[u8].iter().any() control check 7B", || {
        black_box(bytes_clean).iter().any(|&b| b < 0x20 || b == 0x7F) as u64
    });

    bench_wide("[u8].iter().any() control check (has ctrl)", || {
        black_box(bytes_ctrl).iter().any(|&b| b < 0x20 || b == 0x7F) as u64
    });

    println!();
}
