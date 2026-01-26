//! Benchmark to find SIMD vs scalar crossover point.
//!
//! Run with:
//! ```bash
//! # SSE2 (default)
//! cargo run --release --example perf_simd_crossover
//!
//! # AVX2
//! RUSTFLAGS="-C target-feature=+avx2" cargo run --release --example perf_simd_crossover
//! ```

#[path = "_bench_utils.rs"]
mod _bench_utils;

use _bench_utils::{ITERATIONS, WARMUP, percentile, print_intro, rdtsc};
use nexus_ascii::simd;
use std::hint::black_box;

/// Run benchmark and return stats
fn benchmark<T, F: FnMut() -> T>(iterations: usize, warmup: usize, mut f: F) -> (u64, u64, u64) {
    // Warmup
    for _ in 0..warmup {
        black_box(f());
    }

    // Collect samples
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = rdtsc();
        black_box(f());
        let end = rdtsc();
        samples.push(end.wrapping_sub(start));
    }

    samples.sort_unstable();
    let p50 = percentile(&samples, 50.0);
    let p99 = percentile(&samples, 99.0);
    let p999 = percentile(&samples, 99.9);
    (p50, p99, p999)
}

fn main() {
    print_intro("SIMD CROSSOVER BENCHMARK");

    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    println!("SIMD: AVX2 (32 bytes/iteration)\n");
    #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
    println!("SIMD: SSE2 (16 bytes/iteration)\n");
    #[cfg(not(target_arch = "x86_64"))]
    println!("SIMD: Scalar SWAR (8 bytes/iteration)\n");

    // Test various lengths to find crossover
    let lengths = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14, 15, 16, 17, 18, 20, 24, 28, 31, 32, 33, 40, 48, 56,
        63, 64, 65, 80, 96, 112, 128,
    ];

    println!("=== ASCII VALIDATION (simd::validate_ascii) ===\n");
    println!(
        "{:<30} {:>8} {:>8} {:>8} {:>10}",
        "Length", "p50", "p99", "p999", "cyc/byte"
    );
    println!("{}", "-".repeat(70));

    for &len in &lengths {
        let data: Vec<u8> = (0..len).map(|i| b'A' + (i % 26) as u8).collect();

        let (p50, p99, p999) = benchmark(ITERATIONS, WARMUP, || {
            simd::validate_ascii(black_box(&data))
        });

        let cycles_per_byte = if len > 0 {
            format!("{:.2}", p50 as f64 / len as f64)
        } else {
            "-".to_string()
        };

        println!(
            "{:<30} {:>8} {:>8} {:>8} {:>10}",
            format!("{}B", len),
            p50,
            p99,
            p999,
            cycles_per_byte
        );
    }

    println!("\n=== PRINTABLE VALIDATION (simd::validate_printable) ===\n");
    println!(
        "{:<30} {:>8} {:>8} {:>8} {:>10}",
        "Length", "p50", "p99", "p999", "cyc/byte"
    );
    println!("{}", "-".repeat(70));

    for &len in &lengths {
        let data: Vec<u8> = (0..len).map(|i| b'A' + (i % 26) as u8).collect();

        let (p50, p99, p999) = benchmark(ITERATIONS, WARMUP, || {
            simd::validate_printable(black_box(&data))
        });

        let cycles_per_byte = if len > 0 {
            format!("{:.2}", p50 as f64 / len as f64)
        } else {
            "-".to_string()
        };

        println!(
            "{:<30} {:>8} {:>8} {:>8} {:>10}",
            format!("{}B", len),
            p50,
            p99,
            p999,
            cycles_per_byte
        );
    }

    println!("\n=== FULL CONSTRUCTION (AsciiString::try_from_bytes) ===\n");
    println!(
        "{:<30} {:>8} {:>8} {:>8} {:>10}",
        "Length", "p50", "p99", "p999", "cyc/byte"
    );
    println!("{}", "-".repeat(70));

    for &len in &lengths {
        let data: Vec<u8> = (0..len).map(|i| b'A' + (i % 26) as u8).collect();

        let (p50, p99, p999) = benchmark(ITERATIONS, WARMUP, || {
            nexus_ascii::AsciiString::<128>::try_from_bytes(black_box(&data))
        });

        let cycles_per_byte = if len > 0 {
            format!("{:.2}", p50 as f64 / len as f64)
        } else {
            "-".to_string()
        };

        println!(
            "{:<30} {:>8} {:>8} {:>8} {:>10}",
            format!("{}B", len),
            p50,
            p99,
            p999,
            cycles_per_byte
        );
    }

    // Analysis
    println!("\n=== ANALYSIS ===\n");
    println!("Crossover point: where cycles/byte stabilizes (SIMD amortizes setup cost)");
    println!("  - Below crossover: scalar/SWAR may be faster due to lower setup overhead");
    println!("  - Above crossover: SIMD wins with lower cycles/byte\n");
    println!("Typical crossover points from literature:");
    println!("  - SSE2 (16B chunks):    ~16-32 bytes");
    println!("  - AVX2 (32B chunks):    ~32-64 bytes");
    println!("  - AVX-512 (64B chunks): ~64-128 bytes");
}
