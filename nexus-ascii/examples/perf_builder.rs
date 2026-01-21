//! AsciiStringBuilder performance benchmark.
//!
//! Measures construction, push operations, and build finalization.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_builder
//!
//! # With perf stat for IPC/branch analysis:
//! perf stat -r 10 ./target/release/examples/perf_builder
//! ```

use nexus_ascii::{AsciiChar, AsciiStr, AsciiString, AsciiStringBuilder};
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
    println!("ASCIISTRING BUILDER BENCHMARK");
    println!("==============================\n");
    println!("Iterations: {}, Warmup: {}", ITERATIONS, WARMUP);
    println!("All times in CPU cycles\n");

    // =========================================================================
    // Construction
    // =========================================================================
    println!("=== CONSTRUCTION ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    bench("AsciiStringBuilder::new()", || {
        let builder: AsciiStringBuilder<32> = black_box(AsciiStringBuilder::new());
        builder.len() as u64
    });

    let source: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    bench("AsciiStringBuilder::from_ascii_string()", || {
        let builder = AsciiStringBuilder::from_ascii_string(black_box(source));
        builder.len() as u64
    });

    // =========================================================================
    // Push operations
    // =========================================================================
    println!("\n=== PUSH OPERATIONS ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    bench("push(AsciiChar)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push(black_box(AsciiChar::A)).unwrap();
        builder.len() as u64
    });

    bench("push_byte(b'A')", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_byte(black_box(b'A')).unwrap();
        builder.len() as u64
    });

    bench("push_str (7B \"BTC-USD\")", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_str(black_box("BTC-USD")).unwrap();
        builder.len() as u64
    });

    bench("push_str (20B)", || {
        let mut builder: AsciiStringBuilder<64> = AsciiStringBuilder::new();
        builder.push_str(black_box("ORDER-ID-1234567890")).unwrap();
        builder.len() as u64
    });

    bench("push_bytes (7B)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_bytes(black_box(b"BTC-USD")).unwrap();
        builder.len() as u64
    });

    bench("push_bytes_unchecked (7B)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        unsafe { builder.push_bytes_unchecked(black_box(b"BTC-USD")) };
        builder.len() as u64
    });

    let ascii_str = AsciiStr::try_from_bytes(b"BTC-USD").unwrap();
    bench("push_ascii_str (7B)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_ascii_str(black_box(ascii_str)).unwrap();
        builder.len() as u64
    });

    let ascii_string: AsciiString<16> = AsciiString::try_from("BTC-USD").unwrap();
    bench("push_ascii_string (7B)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_ascii_string(black_box(&ascii_string)).unwrap();
        builder.len() as u64
    });

    let raw_buffer: [u8; 16] = *b"BTC-USD\0\0\0\0\0\0\0\0\0";
    bench("push_raw (7B in 16B buffer)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_raw(black_box(raw_buffer)).unwrap();
        builder.len() as u64
    });

    bench("push_raw_unchecked (7B in 16B buffer)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        unsafe { builder.push_raw_unchecked(black_box(raw_buffer)) };
        builder.len() as u64
    });

    // =========================================================================
    // Multiple pushes
    // =========================================================================
    println!("\n=== MULTIPLE PUSHES ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    bench("push_str x3 (BTC + - + USD)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_str(black_box("BTC")).unwrap();
        builder.push_str(black_box("-")).unwrap();
        builder.push_str(black_box("USD")).unwrap();
        builder.len() as u64
    });

    bench("push_byte x7 (B-T-C--U-S-D)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        for b in b"BTC-USD" {
            builder.push_byte(black_box(*b)).unwrap();
        }
        builder.len() as u64
    });

    // =========================================================================
    // Build finalization
    // =========================================================================
    println!("\n=== BUILD FINALIZATION ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    bench("build() (7B content)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_str("BTC-USD").unwrap();
        let s = black_box(builder).build();
        s.len() as u64
    });

    bench("build() (20B content)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_str("ORDER-ID-1234567890").unwrap();
        let s = black_box(builder).build();
        s.len() as u64
    });

    bench("build() (empty)", || {
        let builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        let s = black_box(builder).build();
        s.len() as u64
    });

    // =========================================================================
    // Full pipeline comparison
    // =========================================================================
    println!("\n=== FULL PIPELINE ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    bench("builder: push_str + build (7B)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_str(black_box("BTC-USD")).unwrap();
        let s = builder.build();
        s.len() as u64
    });

    bench("direct: AsciiString::try_from (7B)", || {
        let s: AsciiString<32> = AsciiString::try_from(black_box("BTC-USD")).unwrap();
        s.len() as u64
    });

    bench("builder: 3x push_str + build", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_str(black_box("BTC")).unwrap();
        builder.push_str(black_box("-")).unwrap();
        builder.push_str(black_box("USD")).unwrap();
        let s = builder.build();
        s.len() as u64
    });

    // =========================================================================
    // Mutation operations
    // =========================================================================
    println!("\n=== MUTATION ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    bench("clear()", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_str("Hello, World!").unwrap();
        black_box(&mut builder).clear();
        builder.len() as u64
    });

    bench("truncate(5)", || {
        let mut builder: AsciiStringBuilder<32> = AsciiStringBuilder::new();
        builder.push_str("Hello, World!").unwrap();
        black_box(&mut builder).truncate(5);
        builder.len() as u64
    });

    println!();
}
