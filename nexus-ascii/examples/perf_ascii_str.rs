//! AsciiStr performance benchmark.
//!
//! Measures Deref coercion, construction, and cross-type equality performance.
//!
//! Run with:
//! ```bash
//! cargo run --release --example perf_ascii_str
//!
//! # With perf stat for IPC/branch analysis:
//! perf stat -r 10 ./target/release/examples/perf_ascii_str
//! ```

use nexus_ascii::{AsciiStr, AsciiString};
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
    println!("ASCIISTR BENCHMARK");
    println!("==================\n");
    println!("Iterations: {}, Warmup: {}", ITERATIONS, WARMUP);
    println!("All times in CPU cycles\n");

    // =========================================================================
    // Construction
    // =========================================================================
    println!("=== CONSTRUCTION ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    let bytes_7 = b"BTC-USD";
    let bytes_32 = b"ORDER-ID-12345678901234567890123";

    bench("try_from_bytes (7B)", || {
        let s = AsciiStr::try_from_bytes(black_box(bytes_7)).unwrap();
        s.len() as u64
    });

    bench("try_from_bytes (32B)", || {
        let s = AsciiStr::try_from_bytes(black_box(bytes_32)).unwrap();
        s.len() as u64
    });

    bench("try_from_str (7B)", || {
        let s = AsciiStr::try_from_str(black_box("BTC-USD")).unwrap();
        s.len() as u64
    });

    bench("from_bytes_unchecked (7B)", || {
        let s = unsafe { AsciiStr::from_bytes_unchecked(black_box(bytes_7)) };
        s.len() as u64
    });

    bench("from_bytes_unchecked (32B)", || {
        let s = unsafe { AsciiStr::from_bytes_unchecked(black_box(bytes_32)) };
        s.len() as u64
    });

    // =========================================================================
    // Deref from AsciiString
    // =========================================================================
    println!("\n=== DEREF FROM ASCIISTRING ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    let ascii_string: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();

    bench("as_ascii_str()", || {
        let s = black_box(&ascii_string).as_ascii_str();
        s.len() as u64
    });

    bench("deref coercion", || {
        let s: &AsciiStr = black_box(&ascii_string);
        s.len() as u64
    });

    // Compare direct method vs deref method
    bench("AsciiString.len() direct", || {
        black_box(&ascii_string).len() as u64
    });

    bench("(&AsciiStr).len() via deref", || {
        let s: &AsciiStr = black_box(&ascii_string);
        s.len() as u64
    });

    // =========================================================================
    // Accessors
    // =========================================================================
    println!("\n=== ACCESSORS ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    let ascii_str = AsciiStr::try_from_bytes(b"BTC-USD").unwrap();

    bench("len()", || black_box(ascii_str).len() as u64);

    bench("as_bytes()", || black_box(ascii_str).as_bytes().len() as u64);

    bench("as_str()", || black_box(ascii_str).as_str().len() as u64);

    bench("get(0)", || {
        black_box(ascii_str).get(0).map_or(0, |c| c.as_u8() as u64)
    });

    bench("first()", || {
        black_box(ascii_str)
            .first()
            .map_or(0, |c| c.as_u8() as u64)
    });

    bench("last()", || {
        black_box(ascii_str)
            .last()
            .map_or(0, |c| c.as_u8() as u64)
    });

    // =========================================================================
    // Cross-type equality
    // =========================================================================
    println!("\n=== CROSS-TYPE EQUALITY ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    let ascii_str = AsciiStr::try_from_bytes(b"BTC-USD").unwrap();
    let ascii_string: AsciiString<32> = AsciiString::try_from("BTC-USD").unwrap();

    bench("AsciiStr == AsciiStr", || {
        if black_box(ascii_str) == black_box(ascii_str) {
            1
        } else {
            0
        }
    });

    bench("AsciiString == AsciiStr", || {
        if black_box(&ascii_string) == black_box(ascii_str) {
            1
        } else {
            0
        }
    });

    bench("AsciiStr == str", || {
        if black_box(ascii_str) == black_box("BTC-USD") {
            1
        } else {
            0
        }
    });

    bench("AsciiStr == &[u8]", || {
        if *black_box(ascii_str) == black_box(b"BTC-USD")[..] {
            1
        } else {
            0
        }
    });

    // =========================================================================
    // Function accepting &AsciiStr
    // =========================================================================
    println!("\n=== FUNCTION ACCEPTING &AsciiStr ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    #[inline(never)]
    fn process_ascii_str(s: &AsciiStr) -> u64 {
        s.len() as u64
    }

    bench("call with &AsciiStr directly", || {
        process_ascii_str(black_box(ascii_str))
    });

    bench("call with &AsciiString (deref)", || {
        process_ascii_str(black_box(&ascii_string))
    });

    // =========================================================================
    // Baseline comparisons
    // =========================================================================
    println!("\n=== BASELINE COMPARISONS ===\n");
    println!(
        "{:<40} {:>8} {:>8} {:>8}",
        "Operation", "p50", "p99", "p999"
    );
    println!("{}", "-".repeat(68));

    let str_ref: &str = "BTC-USD";
    let byte_slice: &[u8] = b"BTC-USD";

    bench("&str.len() (baseline)", || {
        black_box(str_ref).len() as u64
    });

    bench("&[u8].len() (baseline)", || {
        black_box(byte_slice).len() as u64
    });

    bench("&str == &str (baseline)", || {
        if black_box(str_ref) == black_box("BTC-USD") {
            1
        } else {
            0
        }
    });

    bench("&[u8] == &[u8] (baseline)", || {
        if black_box(byte_slice) == black_box(b"BTC-USD") {
            1
        } else {
            0
        }
    });

    println!();
}
