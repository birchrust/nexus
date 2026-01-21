//! Performance benchmark for try_from_raw and related methods.
//!
//! Measures the overhead of null-byte detection and buffer construction.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_from_raw
//!
//! # With perf stat for IPC/branch analysis:
//! perf stat -r 10 ./target/release/examples/perf_from_raw
//! ```

use nexus_ascii::AsciiString;
use std::hint::black_box;

const ITERATIONS: usize = 100_000;
const WARMUP: usize = 10_000;

#[cfg(target_arch = "x86_64")]
fn rdtsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

#[cfg(not(target_arch = "x86_64"))]
fn rdtsc() -> u64 {
    std::time::Instant::now().elapsed().as_nanos() as u64
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn bench<F: FnMut() -> u64>(name: &str, mut f: F) -> (u64, u64, u64) {
    // Warmup
    for _ in 0..WARMUP {
        black_box(f());
    }

    // Collect samples
    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = rdtsc();
        black_box(f());
        let end = rdtsc();
        samples.push(end.wrapping_sub(start));
    }

    samples.sort_unstable();
    let p50 = percentile(&samples, 50.0);
    let p99 = percentile(&samples, 99.0);
    let p999 = percentile(&samples, 99.9);

    println!("{:<45} {:>8} {:>8} {:>8}", name, p50, p99, p999);
    (p50, p99, p999)
}

fn main() {
    println!("FROM_RAW BENCHMARK");
    println!("==================\n");
    println!("Iterations: {}, Warmup: {}", ITERATIONS, WARMUP);
    println!("All times in CPU cycles\n");

    // =========================================================================
    // try_from_raw vs try_from_bytes
    // =========================================================================
    println!("=== try_from_raw vs try_from_bytes ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    // 7-byte string (typical symbol like "BTC-USD")
    let buffer_7: [u8; 16] = *b"BTC-USD\0\0\0\0\0\0\0\0\0";
    let bytes_7 = b"BTC-USD";

    bench("try_from_bytes (7B slice)", || {
        let s: AsciiString<16> = AsciiString::try_from_bytes(black_box(bytes_7)).unwrap();
        s.len() as u64
    });

    bench("try_from_raw (7B in 16B buffer)", || {
        let s: AsciiString<16> = AsciiString::try_from_raw(black_box(buffer_7)).unwrap();
        s.len() as u64
    });

    bench("from_raw_unchecked (7B in 16B buffer)", || {
        let s: AsciiString<16> = unsafe { AsciiString::from_raw_unchecked(black_box(buffer_7)) };
        s.len() as u64
    });

    // 8-byte string (boundary case)
    let buffer_8: [u8; 16] = *b"BTCUSDT!\0\0\0\0\0\0\0\0";
    let bytes_8 = b"BTCUSDT!";

    bench("try_from_bytes (8B slice)", || {
        let s: AsciiString<16> = AsciiString::try_from_bytes(black_box(bytes_8)).unwrap();
        s.len() as u64
    });

    bench("try_from_raw (8B in 16B buffer)", || {
        let s: AsciiString<16> = AsciiString::try_from_raw(black_box(buffer_8)).unwrap();
        s.len() as u64
    });

    // 20-byte string (spans multiple 8-byte chunks)
    let buffer_20: [u8; 32] = *b"ORDER-ID-12345678901\0\0\0\0\0\0\0\0\0\0\0\0";
    let bytes_20 = b"ORDER-ID-12345678901";

    bench("try_from_bytes (20B slice)", || {
        let s: AsciiString<32> = AsciiString::try_from_bytes(black_box(bytes_20)).unwrap();
        s.len() as u64
    });

    bench("try_from_raw (20B in 32B buffer)", || {
        let s: AsciiString<32> = AsciiString::try_from_raw(black_box(buffer_20)).unwrap();
        s.len() as u64
    });

    bench("from_raw_unchecked (20B in 32B buffer)", || {
        let s: AsciiString<32> = unsafe { AsciiString::from_raw_unchecked(black_box(buffer_20)) };
        s.len() as u64
    });

    // =========================================================================
    // try_from_right_padded
    // =========================================================================
    println!("\n=== try_from_right_padded ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    let padded_7: [u8; 16] = *b"BTC-USD         ";

    bench("try_from_right_padded (7B, space pad)", || {
        let s: AsciiString<16> = AsciiString::try_from_right_padded(black_box(padded_7), b' ').unwrap();
        s.len() as u64
    });

    let padded_20: [u8; 32] = *b"ORDER-ID-1234567890             ";

    bench("try_from_right_padded (20B, space pad)", || {
        let s: AsciiString<32> = AsciiString::try_from_right_padded(black_box(padded_20), b' ').unwrap();
        s.len() as u64
    });

    // =========================================================================
    // Null byte at various positions
    // =========================================================================
    println!("\n=== Null position impact ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    // Null at position 0 (empty)
    let buffer_null_0: [u8; 32] = [0u8; 32];
    bench("try_from_raw (null at 0)", || {
        let s: AsciiString<32> = AsciiString::try_from_raw(black_box(buffer_null_0)).unwrap();
        s.len() as u64
    });

    // Null at position 7 (within first chunk)
    let mut buffer_null_7 = [b'X'; 32];
    buffer_null_7[7] = 0;
    bench("try_from_raw (null at 7)", || {
        let s: AsciiString<32> = AsciiString::try_from_raw(black_box(buffer_null_7)).unwrap();
        s.len() as u64
    });

    // Null at position 15 (end of second chunk)
    let mut buffer_null_15 = [b'X'; 32];
    buffer_null_15[15] = 0;
    bench("try_from_raw (null at 15)", || {
        let s: AsciiString<32> = AsciiString::try_from_raw(black_box(buffer_null_15)).unwrap();
        s.len() as u64
    });

    // Null at position 24 (third chunk)
    let mut buffer_null_24 = [b'X'; 32];
    buffer_null_24[24] = 0;
    bench("try_from_raw (null at 24)", || {
        let s: AsciiString<32> = AsciiString::try_from_raw(black_box(buffer_null_24)).unwrap();
        s.len() as u64
    });

    // No null (full buffer)
    let buffer_full: [u8; 32] = [b'X'; 32];
    bench("try_from_raw (no null, full 32B)", || {
        let s: AsciiString<32> = AsciiString::try_from_raw(black_box(buffer_full)).unwrap();
        s.len() as u64
    });

    // =========================================================================
    // Baseline comparisons
    // =========================================================================
    println!("\n=== Baselines ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    // memchr-style null search baseline
    let search_buffer: [u8; 32] = *b"ABCDEFGHIJKLMNOP\0...............";
    bench("memchr find null (16B content)", || {
        let pos = black_box(&search_buffer)
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(32);
        pos as u64
    });

    // from_bytes_unchecked (no null search, no validation)
    bench("from_bytes_unchecked (7B, no null search)", || {
        let s: AsciiString<16> = unsafe { AsciiString::from_bytes_unchecked(black_box(bytes_7)) };
        s.len() as u64
    });

    println!();
}
