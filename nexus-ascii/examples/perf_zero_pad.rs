//! Zero-padding processing strategy benchmark.
//!
//! Compares three approaches for SIMD operations on zero-padded buffers:
//! - Length-aware: process only `len` bytes (current, has scalar remainder)
//! - Round-up: process `len` rounded up to next chunk boundary (no remainder)
//! - Full-buffer: process all `CAP` bytes (no branches, processes extra zeros)
//!
//! The goal is to find the crossover point where processing extra zeros becomes
//! more expensive than the branch + scalar remainder it eliminates.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_zero_pad
//! ```

#[path = "_bench_utils.rs"]
mod bench_utils;

use bench_utils::{bench_wide, print_header_wide, print_intro};
use nexus_ascii::AsciiString;
use nexus_ascii::simd;
use std::hint::black_box;

/// Round `n` up to the next multiple of `align`.
#[inline(always)]
#[allow(dead_code)]
const fn round_up(n: usize, align: usize) -> usize {
    (n + align - 1) & !(align - 1)
}

/// Create a zero-padded buffer with `len` bytes of ASCII content.
fn make_buffer<const CAP: usize>(len: usize) -> [u8; CAP] {
    let mut buf = [0u8; CAP];
    for i in 0..len.min(CAP) {
        // Use printable ASCII that varies (avoids overly optimistic branch prediction)
        buf[i] = b'A' + (i % 26) as u8;
    }
    buf
}

fn main() {
    print_intro("ZERO-PAD PROCESSING STRATEGY BENCHMARK");

    // =========================================================================
    // CAP=8: Only SWAR available (no SSE2 for CAP < 16)
    // =========================================================================
    println!();
    print_header_wide("CAP=8 (SWAR only)");

    let buf8: [u8; 8] = make_buffer(7);
    bench_wide("validate len=7  (len-aware)", || {
        black_box(simd::validate_ascii_bounded::<8>(black_box(&buf8[..7]))).is_ok() as u64
    });
    bench_wide("validate len=8  (round-8 / full)", || {
        black_box(simd::validate_ascii_bounded::<8>(black_box(&buf8[..8]))).is_ok() as u64
    });

    // =========================================================================
    // CAP=16: 1 SSE2 chunk covers entire buffer
    // =========================================================================
    println!();
    print_header_wide("CAP=16 (1 SSE2 chunk)");

    let buf16: [u8; 16] = make_buffer(7);
    bench_wide("validate len=7  (len-aware)", || {
        black_box(simd::validate_ascii_bounded::<16>(black_box(&buf16[..7]))).is_ok() as u64
    });
    bench_wide("validate len=7  (round-8)", || {
        black_box(simd::validate_ascii_bounded::<16>(black_box(&buf16[..8]))).is_ok() as u64
    });
    bench_wide("validate len=7  (round-16 / full)", || {
        black_box(simd::validate_ascii_bounded::<16>(black_box(&buf16[..16]))).is_ok() as u64
    });

    let buf16b: [u8; 16] = make_buffer(13);
    bench_wide("validate len=13 (len-aware)", || {
        black_box(simd::validate_ascii_bounded::<16>(black_box(&buf16b[..13]))).is_ok() as u64
    });
    bench_wide("validate len=13 (round-16 / full)", || {
        black_box(simd::validate_ascii_bounded::<16>(black_box(&buf16b[..16]))).is_ok() as u64
    });

    // =========================================================================
    // CAP=32: Key crossover region (1-2 SSE2 chunks, or 1 AVX2)
    // =========================================================================
    println!();
    print_header_wide("CAP=32 (2 SSE2 or 1 AVX2)");

    let buf32: [u8; 32] = make_buffer(7);
    bench_wide("validate len=7  (len-aware)", || {
        black_box(simd::validate_ascii_bounded::<32>(black_box(&buf32[..7]))).is_ok() as u64
    });
    bench_wide("validate len=7  (round-8)", || {
        black_box(simd::validate_ascii_bounded::<32>(black_box(&buf32[..8]))).is_ok() as u64
    });
    bench_wide("validate len=7  (round-16)", || {
        black_box(simd::validate_ascii_bounded::<32>(black_box(&buf32[..16]))).is_ok() as u64
    });
    bench_wide("validate len=7  (full-32)", || {
        black_box(simd::validate_ascii_bounded::<32>(black_box(&buf32[..32]))).is_ok() as u64
    });

    let buf32b: [u8; 32] = make_buffer(20);
    bench_wide("validate len=20 (len-aware)", || {
        black_box(simd::validate_ascii_bounded::<32>(black_box(&buf32b[..20]))).is_ok() as u64
    });
    bench_wide("validate len=20 (round-24)", || {
        black_box(simd::validate_ascii_bounded::<32>(black_box(&buf32b[..24]))).is_ok() as u64
    });
    bench_wide("validate len=20 (round-32 / full)", || {
        black_box(simd::validate_ascii_bounded::<32>(black_box(&buf32b[..32]))).is_ok() as u64
    });

    // =========================================================================
    // CAP=64: Large buffer, short content (worst case for full-buffer)
    // =========================================================================
    println!();
    print_header_wide("CAP=64 (4 SSE2 or 2 AVX2)");

    let buf64: [u8; 64] = make_buffer(7);
    bench_wide("validate len=7  (len-aware)", || {
        black_box(simd::validate_ascii_bounded::<64>(black_box(&buf64[..7]))).is_ok() as u64
    });
    bench_wide("validate len=7  (round-8)", || {
        black_box(simd::validate_ascii_bounded::<64>(black_box(&buf64[..8]))).is_ok() as u64
    });
    bench_wide("validate len=7  (round-16)", || {
        black_box(simd::validate_ascii_bounded::<64>(black_box(&buf64[..16]))).is_ok() as u64
    });
    bench_wide("validate len=7  (round-32)", || {
        black_box(simd::validate_ascii_bounded::<64>(black_box(&buf64[..32]))).is_ok() as u64
    });
    bench_wide("validate len=7  (full-64)", || {
        black_box(simd::validate_ascii_bounded::<64>(black_box(&buf64[..64]))).is_ok() as u64
    });

    let buf64b: [u8; 64] = make_buffer(20);
    bench_wide("validate len=20 (len-aware)", || {
        black_box(simd::validate_ascii_bounded::<64>(black_box(&buf64b[..20]))).is_ok() as u64
    });
    bench_wide("validate len=20 (round-32)", || {
        black_box(simd::validate_ascii_bounded::<64>(black_box(&buf64b[..32]))).is_ok() as u64
    });
    bench_wide("validate len=20 (full-64)", || {
        black_box(simd::validate_ascii_bounded::<64>(black_box(&buf64b[..64]))).is_ok() as u64
    });

    let buf64c: [u8; 64] = make_buffer(48);
    bench_wide("validate len=48 (len-aware)", || {
        black_box(simd::validate_ascii_bounded::<64>(black_box(&buf64c[..48]))).is_ok() as u64
    });
    bench_wide("validate len=48 (round-64 / full)", || {
        black_box(simd::validate_ascii_bounded::<64>(black_box(&buf64c[..64]))).is_ok() as u64
    });

    // =========================================================================
    // Case conversion: same CAP/len matrix
    // =========================================================================
    println!();
    print_header_wide("CASE CONVERSION (to_uppercase)");

    let s32_7: AsciiString<32> = AsciiString::try_from("btc-usd").unwrap();
    bench_wide("uppercase CAP=32 len=7  (current)", || {
        black_box(black_box(&s32_7).to_ascii_uppercase()).len() as u64
    });

    let s32_20: AsciiString<32> = AsciiString::try_from("order-id-1234567890").unwrap();
    bench_wide("uppercase CAP=32 len=19 (current)", || {
        black_box(black_box(&s32_20).to_ascii_uppercase()).len() as u64
    });

    let s64_7: AsciiString<64> = AsciiString::try_from("btc-usd").unwrap();
    bench_wide("uppercase CAP=64 len=7  (current)", || {
        black_box(black_box(&s64_7).to_ascii_uppercase()).len() as u64
    });

    let s64_20: AsciiString<64> = AsciiString::try_from("order-id-1234567890").unwrap();
    bench_wide("uppercase CAP=64 len=19 (current)", || {
        black_box(black_box(&s64_20).to_ascii_uppercase()).len() as u64
    });

    let s64_48: AsciiString<64> =
        AsciiString::try_from("abcdefghijklmnopqrstuvwxyz-0123456789-abcdefghijk").unwrap();
    bench_wide("uppercase CAP=64 len=49 (current)", || {
        black_box(black_box(&s64_48).to_ascii_uppercase()).len() as u64
    });

    // =========================================================================
    // eq_ignore_ascii_case: same CAP/len matrix
    // =========================================================================
    println!();
    print_header_wide("EQ_IGNORE_ASCII_CASE");

    let a32: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    let b32: AsciiString<32> = AsciiString::try_from("btc-usd").unwrap();
    bench_wide("eq_icase CAP=32 len=7  (same)", || {
        black_box(black_box(&a32).eq_ignore_ascii_case(black_box(&b32))) as u64
    });

    let a32b: AsciiString<32> = AsciiString::try_from("ORDER-ID-1234567890").unwrap();
    let b32b: AsciiString<32> = AsciiString::try_from("order-id-1234567890").unwrap();
    bench_wide("eq_icase CAP=32 len=19 (same)", || {
        black_box(black_box(&a32b).eq_ignore_ascii_case(black_box(&b32b))) as u64
    });

    let a64: AsciiString<64> = AsciiString::try_from("BTC-USD").unwrap();
    let b64: AsciiString<64> = AsciiString::try_from("btc-usd").unwrap();
    bench_wide("eq_icase CAP=64 len=7  (same)", || {
        black_box(black_box(&a64).eq_ignore_ascii_case(black_box(&b64))) as u64
    });

    let a64b: AsciiString<64> = AsciiString::try_from("ORDER-ID-1234567890").unwrap();
    let b64b: AsciiString<64> = AsciiString::try_from("order-id-1234567890").unwrap();
    bench_wide("eq_icase CAP=64 len=19 (same)", || {
        black_box(black_box(&a64b).eq_ignore_ascii_case(black_box(&b64b))) as u64
    });

    println!();
}
