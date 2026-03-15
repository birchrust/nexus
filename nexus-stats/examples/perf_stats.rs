//! Cycles-per-update benchmark for all nexus-stats primitives.
//!
//! Batches 64 updates per measurement to amortize rdtsc overhead (~20 cycles).
//!
//! Usage:
//!   cargo build --release --example perf_stats
//!   taskset -c 0 ./target/release/examples/perf_stats

use std::hint::black_box;

use nexus_stats::*;

// ============================================================================
// Timing
// ============================================================================

#[inline(always)]
fn rdtsc_start() -> u64 {
    unsafe {
        std::arch::x86_64::_mm_lfence();
        std::arch::x86_64::_rdtsc()
    }
}

#[inline(always)]
fn rdtsc_end() -> u64 {
    unsafe {
        let mut aux = 0u32;
        let tsc = std::arch::x86_64::__rdtscp(&raw mut aux);
        std::arch::x86_64::_mm_lfence();
        tsc
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((sorted.len() as f64) * p / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn print_row(label: &str, samples: &mut [u64]) {
    samples.sort_unstable();
    println!(
        "  {:<24} {:>6} {:>6} {:>6} {:>7} {:>7}",
        label,
        percentile(samples, 50.0),
        percentile(samples, 90.0),
        percentile(samples, 99.0),
        percentile(samples, 99.9),
        samples[samples.len() - 1],
    );
}

fn print_header() {
    println!(
        "  {:<24} {:>6} {:>6} {:>6} {:>7} {:>7}",
        "(cycles/op)", "p50", "p90", "p99", "p99.9", "max"
    );
}

const SAMPLES: usize = 100_000;
const WARMUP: usize = 10_000;
const BATCH: u64 = 64;

// ============================================================================
// Input — varying values to prevent constant-folding
// ============================================================================

#[inline(always)]
fn next_val(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// ============================================================================
// Benchmarks
// ============================================================================

fn bench_cusum_f64(samples: &mut [u64]) {
    let mut cusum = CusumF64::builder(100.0)
        .slack(5.0)
        .threshold(1e18)
        .build();
    let mut rng = 12345u64;

    for _ in 0..WARMUP {
        let _ = cusum.update(90.0 + (next_val(&mut rng) % 20) as f64);
    }

    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = cusum.update(90.0 + (next_val(&mut rng) % 20) as f64);
        }
        let end = rdtsc_end();
        black_box(cusum.upper());
        black_box(cusum.lower());
        *s = (end - start) / BATCH;
    }
}

fn bench_cusum_i64(samples: &mut [u64]) {
    let mut cusum = CusumI64::builder(1000)
        .slack(50)
        .threshold(i64::MAX)
        .build();
    let mut rng = 12345u64;

    for _ in 0..WARMUP {
        let _ = cusum.update(990 + (next_val(&mut rng) % 20) as i64);
    }

    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let v = 990 + (next_val(&mut rng) % 20) as i64;
            black_box(cusum.update(black_box(v)));
        }
        let end = rdtsc_end();
        *s = (end - start) / BATCH;
    }
}

fn bench_ema_f64(samples: &mut [u64]) {
    let mut ema = EmaF64::builder().alpha(0.1).build();
    let mut rng = 12345u64;
    let _ = ema.update(100.0);

    for _ in 0..WARMUP {
        let _ = ema.update(90.0 + (next_val(&mut rng) % 20) as f64);
    }

    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = ema.update(90.0 + (next_val(&mut rng) % 20) as f64);
        }
        let end = rdtsc_end();
        black_box(ema.value());
        *s = (end - start) / BATCH;
    }
}

fn bench_ema_i64(samples: &mut [u64]) {
    let mut ema = EmaI64::builder().span(15).build();
    let mut rng = 12345u64;
    let _ = ema.update(1000);

    for _ in 0..WARMUP {
        let _ = ema.update(990 + (next_val(&mut rng) % 20) as i64);
    }

    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = ema.update(990 + (next_val(&mut rng) % 20) as i64);
        }
        let end = rdtsc_end();
        black_box(ema.value());
        *s = (end - start) / BATCH;
    }
}

fn bench_welford_f64(samples: &mut [u64]) {
    let mut w = WelfordF64::new();
    let mut rng = 12345u64;

    for _ in 0..WARMUP {
        w.update(90.0 + (next_val(&mut rng) % 20) as f64);
    }

    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            w.update(90.0 + (next_val(&mut rng) % 20) as f64);
        }
        let end = rdtsc_end();
        black_box(w.mean());
        *s = (end - start) / BATCH;
    }
}

fn bench_welford_f64_query(samples: &mut [u64]) {
    let mut w = WelfordF64::new();
    let mut rng = 12345u64;
    for _ in 0..10_000 {
        w.update(90.0 + (next_val(&mut rng) % 20) as f64);
    }
    let w = black_box(w); // prevent hoisting

    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            black_box(w.mean());
            black_box(w.variance());
            black_box(w.std_dev());
        }
        let end = rdtsc_end();
        *s = (end - start) / BATCH;
    }
}

fn main() {
    println!("\nnexus-stats benchmark — cycles per operation (batch={BATCH})");
    println!("=========================================================\n");
    print_header();

    let mut buf = vec![0u64; SAMPLES];

    bench_cusum_f64(&mut buf);
    print_row("CusumF64::update", &mut buf);

    bench_cusum_i64(&mut buf);
    print_row("CusumI64::update", &mut buf);

    bench_ema_f64(&mut buf);
    print_row("EmaF64::update", &mut buf);

    bench_ema_i64(&mut buf);
    print_row("EmaI64::update", &mut buf);

    bench_welford_f64(&mut buf);
    print_row("WelfordF64::update", &mut buf);

    bench_welford_f64_query(&mut buf);
    print_row("WelfordF64::query(3x)", &mut buf);

    println!();
}
