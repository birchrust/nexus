//! Performance benchmark for transformation methods.
//!
//! Measures to_ascii_uppercase, to_ascii_lowercase, and truncated.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_transform
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

    println!("{:<40} {:>8} {:>8} {:>8}", name, p50, p99, p999);
    (p50, p99, p999)
}

fn main() {
    println!("TRANSFORMATION BENCHMARK");
    println!("========================\n");
    println!("Iterations: {}, Warmup: {}", ITERATIONS, WARMUP);
    println!("All times in CPU cycles\n");

    // =========================================================================
    // Case conversion
    // =========================================================================
    println!("=== CASE CONVERSION ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

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
    println!("\n=== TRUNCATION ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    let long: AsciiString<64> = AsciiString::try_from("Hello, World! This is a longer string for truncation.").unwrap();
    let long_len = long.len(); // 54

    bench("truncated (54B -> 5B)", || {
        let t = black_box(long).truncated(5);
        t.len() as u64
    });

    bench("truncated (54B -> 30B)", || {
        let t = black_box(long).truncated(30);
        t.len() as u64
    });

    bench(&format!("truncated ({}B -> {}B, no change)", long_len, long_len), || {
        let t = black_box(long).truncated(long_len);
        t.len() as u64
    });

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
    println!("\n=== BASELINES ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

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
