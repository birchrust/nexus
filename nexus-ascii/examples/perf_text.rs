//! AsciiText performance benchmark.
//!
//! Measures construction and comparison with AsciiString.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_text
//!
//! # With perf stat for IPC/branch analysis:
//! perf stat -r 10 ./target/release/examples/perf_text
//! ```

use nexus_ascii::{AsciiString, AsciiText};
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
    println!("ASCIITEXT BENCHMARK");
    println!("===================\n");
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

    bench("AsciiText::empty()", || {
        let t: AsciiText<32> = black_box(AsciiText::empty());
        t.len() as u64
    });

    bench("AsciiText::try_from_str (7B)", || {
        let t: AsciiText<32> = AsciiText::try_from(black_box("BTC-USD")).unwrap();
        t.len() as u64
    });

    bench("AsciiText::try_from_str (20B)", || {
        let t: AsciiText<32> = AsciiText::try_from(black_box("ORDER-ID-1234567890")).unwrap();
        t.len() as u64
    });

    bench("AsciiText::try_from_bytes (7B)", || {
        let t: AsciiText<32> = AsciiText::try_from_bytes(black_box(b"BTC-USD")).unwrap();
        t.len() as u64
    });

    bench("AsciiText::try_from_bytes (20B)", || {
        let t: AsciiText<32> = AsciiText::try_from_bytes(black_box(b"ORDER-ID-1234567890")).unwrap();
        t.len() as u64
    });

    // =========================================================================
    // Comparison with AsciiString
    // =========================================================================
    println!("\n=== COMPARISON WITH ASCIISTRING ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    bench("AsciiString::try_from (7B)", || {
        let s: AsciiString<32> = AsciiString::try_from(black_box("BTC-USD")).unwrap();
        s.len() as u64
    });

    bench("AsciiText::try_from (7B)", || {
        let t: AsciiText<32> = AsciiText::try_from(black_box("BTC-USD")).unwrap();
        t.len() as u64
    });

    bench("AsciiString::try_from (20B)", || {
        let s: AsciiString<32> = AsciiString::try_from(black_box("ORDER-ID-1234567890")).unwrap();
        s.len() as u64
    });

    bench("AsciiText::try_from (20B)", || {
        let t: AsciiText<32> = AsciiText::try_from(black_box("ORDER-ID-1234567890")).unwrap();
        t.len() as u64
    });

    // =========================================================================
    // Conversion
    // =========================================================================
    println!("\n=== CONVERSION ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    let text: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();
    bench("into_ascii_string()", || {
        let s = black_box(text).into_ascii_string();
        s.len() as u64
    });

    let s: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();
    bench("try_from_ascii_string() (valid)", || {
        let t = AsciiText::try_from_ascii_string(black_box(s)).unwrap();
        t.len() as u64
    });

    // With control character - validation fails
    let s_ctrl: AsciiString<32> = AsciiString::try_from_bytes(b"Hello\x00World").unwrap();
    bench("try_from_ascii_string() (invalid)", || {
        let result = AsciiText::try_from_ascii_string(black_box(s_ctrl));
        result.is_err() as u64
    });

    // =========================================================================
    // Deref access
    // =========================================================================
    println!("\n=== DEREF ACCESS ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    let text: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();

    bench("len() via Deref", || black_box(&text).len() as u64);

    bench("as_str() via Deref", || {
        black_box(&text).as_str().len() as u64
    });

    bench("as_bytes() via Deref", || {
        black_box(&text).as_bytes().len() as u64
    });

    // =========================================================================
    // Equality
    // =========================================================================
    println!("\n=== EQUALITY ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    let t1: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();
    let t2: AsciiText<32> = AsciiText::try_from("BTC-USD").unwrap();
    let s1: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();

    bench("AsciiText == AsciiText (same)", || {
        if black_box(&t1) == black_box(&t2) {
            1
        } else {
            0
        }
    });

    bench("AsciiText == AsciiString", || {
        if black_box(t1) == black_box(s1) {
            1
        } else {
            0
        }
    });

    bench("AsciiText == &str", || {
        if black_box(t1) == black_box("BTC-USD") {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // Baseline
    // =========================================================================
    println!("\n=== BASELINE ===\n");
    println!(
        "{:<45} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(73));

    // const construction has zero runtime cost
    const STATIC_TEXT: AsciiText<16> = AsciiText::from_static("BTC-USD");
    bench("const from_static (access only)", || {
        black_box(STATIC_TEXT).len() as u64
    });

    println!();
}
