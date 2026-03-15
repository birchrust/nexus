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
        "  {:<28} {:>6} {:>6} {:>6} {:>7} {:>7}",
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
        "  {:<28} {:>6} {:>6} {:>6} {:>7} {:>7}",
        "(cycles/op)", "p50", "p90", "p99", "p99.9", "max"
    );
}

fn section(name: &str) {
    println!("\n  --- {name} ---");
}

const SAMPLES: usize = 100_000;
const WARMUP: usize = 10_000;
const BATCH: u64 = 64;

#[inline(always)]
fn next_val(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// ============================================================================
// Phase 1: CUSUM, EMA, Welford
// ============================================================================

fn bench_cusum_f64(samples: &mut [u64]) {
    let mut cusum = CusumF64::builder(100.0).slack(5.0).threshold(1e18).build();
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
        *s = (end - start) / BATCH;
    }
}

fn bench_cusum_i64(samples: &mut [u64]) {
    let mut cusum = CusumI64::builder(1000).slack(50).threshold(i64::MAX).build();
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
            w.update(black_box(90.0 + (next_val(&mut rng) % 20) as f64));
        }
        let end = rdtsc_end();
        black_box(w.mean());
        *s = (end - start) / BATCH;
    }
}

// ============================================================================
// Phase 2: Drawdown, Windowed Min/Max, EWMA Variance
// ============================================================================

fn bench_drawdown_f64(samples: &mut [u64]) {
    let mut dd = DrawdownF64::new();
    let mut rng = 12345u64;
    for _ in 0..WARMUP {
        let _ = dd.update(90.0 + (next_val(&mut rng) % 20) as f64);
    }
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = dd.update(90.0 + (next_val(&mut rng) % 20) as f64);
        }
        let end = rdtsc_end();
        black_box(dd.max_drawdown());
        *s = (end - start) / BATCH;
    }
}

fn bench_windowed_max_f64(samples: &mut [u64]) {
    let mut wm = WindowedMaxF64::new(1000);
    let mut rng = 12345u64;
    for t in 0..WARMUP as u64 {
        let _ = wm.update(t, 90.0 + (next_val(&mut rng) % 20) as f64);
    }
    let mut t = WARMUP as u64;
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = wm.update(t, 90.0 + (next_val(&mut rng) % 20) as f64);
            t += 1;
        }
        let end = rdtsc_end();
        black_box(wm.max());
        *s = (end - start) / BATCH;
    }
}

fn bench_windowed_min_f64(samples: &mut [u64]) {
    let mut wm = WindowedMinF64::new(1000);
    let mut rng = 12345u64;
    for t in 0..WARMUP as u64 {
        let _ = wm.update(t, 90.0 + (next_val(&mut rng) % 20) as f64);
    }
    let mut t = WARMUP as u64;
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = wm.update(t, 90.0 + (next_val(&mut rng) % 20) as f64);
            t += 1;
        }
        let end = rdtsc_end();
        black_box(wm.min());
        *s = (end - start) / BATCH;
    }
}

fn bench_ewma_var_f64(samples: &mut [u64]) {
    let mut ev = EwmaVarF64::builder().alpha(0.1).build();
    let mut rng = 12345u64;
    for _ in 0..WARMUP {
        let _ = ev.update(90.0 + (next_val(&mut rng) % 20) as f64);
    }
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = ev.update(90.0 + (next_val(&mut rng) % 20) as f64);
        }
        let end = rdtsc_end();
        black_box(ev.variance());
        *s = (end - start) / BATCH;
    }
}

// ============================================================================
// Phase 3: Liveness, MOSUM
// ============================================================================

fn bench_liveness_f64(samples: &mut [u64]) {
    let mut lv = LivenessF64::builder()
        .alpha(0.3)
        .deadline_multiple(3.0)
        .build();
    let mut rng = 12345u64;
    for i in 0..WARMUP {
        let _ = lv.record((i as f64).mul_add(10.0, (next_val(&mut rng) % 5) as f64));
    }
    let mut t = WARMUP as f64 * 10.0;
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            t += 10.0 + (next_val(&mut rng) % 5) as f64;
            black_box(lv.record(t));
        }
        let end = rdtsc_end();
        *s = (end - start) / BATCH;
    }
}

fn bench_mosum_f64(samples: &mut [u64]) {
    let mut mosum = MosumF64::<64>::builder(100.0).threshold(1e18).build();
    let mut rng = 12345u64;
    for _ in 0..WARMUP {
        let _ = mosum.update(90.0 + (next_val(&mut rng) % 20) as f64);
    }
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = mosum.update(90.0 + (next_val(&mut rng) % 20) as f64);
        }
        let end = rdtsc_end();
        black_box(mosum.sum());
        *s = (end - start) / BATCH;
    }
}

// ============================================================================
// Phase 4: Covariance, Holt's, Shiryaev-Roberts, TopK
// ============================================================================

fn bench_covariance_f64(samples: &mut [u64]) {
    let mut cov = CovarianceF64::new();
    let mut rng = 12345u64;
    for _ in 0..WARMUP {
        let x = 90.0 + (next_val(&mut rng) % 20) as f64;
        let y = x * 2.0 + (next_val(&mut rng) % 5) as f64;
        cov.update(x, y);
    }
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let x = 90.0 + (next_val(&mut rng) % 20) as f64;
            let y = x * 2.0 + (next_val(&mut rng) % 5) as f64;
            cov.update(x, y);
        }
        let end = rdtsc_end();
        black_box(cov.correlation());
        *s = (end - start) / BATCH;
    }
}

fn bench_holt_f64(samples: &mut [u64]) {
    let mut h = HoltF64::builder().alpha(0.3).beta(0.1).build();
    let mut rng = 12345u64;
    for _ in 0..WARMUP {
        let _ = h.update(90.0 + (next_val(&mut rng) % 20) as f64);
    }
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = h.update(90.0 + (next_val(&mut rng) % 20) as f64);
        }
        let end = rdtsc_end();
        black_box(h.level());
        *s = (end - start) / BATCH;
    }
}

fn bench_shiryaev_roberts(samples: &mut [u64]) {
    let mut sr = ShiryaevRobertsF64::builder()
        .pre_change_mean(100.0)
        .post_change_mean(110.0)
        .variance(25.0)
        .threshold(1e18)
        .build();
    let mut rng = 12345u64;
    for _ in 0..WARMUP {
        let _ = sr.update(90.0 + (next_val(&mut rng) % 20) as f64);
        if sr.statistic() > 1e15 {
            sr.reset();
        }
    }
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = sr.update(90.0 + (next_val(&mut rng) % 20) as f64);
            if sr.statistic() > 1e15 {
                sr.reset();
            }
        }
        let end = rdtsc_end();
        black_box(sr.statistic());
        *s = (end - start) / BATCH;
    }
}

fn bench_topk(samples: &mut [u64]) {
    let mut tk: TopK<u64, 16> = TopK::new();
    let mut rng = 12345u64;
    for _ in 0..WARMUP {
        tk.observe(next_val(&mut rng) % 100);
    }
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            tk.observe(next_val(&mut rng) % 100);
        }
        let end = rdtsc_end();
        black_box(tk.total());
        *s = (end - start) / BATCH;
    }
}

// ============================================================================
// New types: RunningMin/Max, EventRate, QueueDelay
// ============================================================================

fn bench_running_min_f64(samples: &mut [u64]) {
    let mut rm = RunningMinF64::new();
    let mut rng = 12345u64;
    for _ in 0..WARMUP {
        let _ = rm.update(90.0 + (next_val(&mut rng) % 20) as f64);
    }
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = rm.update(90.0 + (next_val(&mut rng) % 20) as f64);
        }
        let end = rdtsc_end();
        black_box(rm.min());
        *s = (end - start) / BATCH;
    }
}

fn bench_running_max_f64(samples: &mut [u64]) {
    let mut rm = RunningMaxF64::new();
    let mut rng = 12345u64;
    for _ in 0..WARMUP {
        let _ = rm.update(90.0 + (next_val(&mut rng) % 20) as f64);
    }
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = rm.update(90.0 + (next_val(&mut rng) % 20) as f64);
        }
        let end = rdtsc_end();
        black_box(rm.max());
        *s = (end - start) / BATCH;
    }
}

fn bench_event_rate_f64(samples: &mut [u64]) {
    let mut er = EventRateF64::builder().alpha(0.3).build();
    let mut rng = 12345u64;
    let mut t = 0.0f64;
    for _ in 0..WARMUP {
        t += 10.0 + (next_val(&mut rng) % 5) as f64;
        er.tick(t);
    }
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            t += 10.0 + (next_val(&mut rng) % 5) as f64;
            er.tick(t);
        }
        let end = rdtsc_end();
        black_box(er.rate());
        *s = (end - start) / BATCH;
    }
}

fn bench_queue_delay_i64(samples: &mut [u64]) {
    let mut qd = QueueDelayI64::builder().target(100).window(1000).build();
    let mut rng = 12345u64;
    for t in 0..WARMUP as u64 {
        let _ = qd.update(t, 50 + (next_val(&mut rng) % 100) as i64);
    }
    let mut t = WARMUP as u64;
    for s in samples.iter_mut() {
        let start = rdtsc_start();
        for _ in 0..BATCH {
            let _ = qd.update(t, 50 + (next_val(&mut rng) % 100) as i64);
            t += 1;
        }
        let end = rdtsc_end();
        black_box(qd.is_elevated());
        *s = (end - start) / BATCH;
    }
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    println!("\nnexus-stats benchmark — cycles per operation (batch={BATCH})");
    println!("=========================================================");

    let mut buf = vec![0u64; SAMPLES];

    section("Change Detection");
    print_header();
    bench_cusum_f64(&mut buf);
    print_row("CusumF64::update", &mut buf);
    bench_cusum_i64(&mut buf);
    print_row("CusumI64::update", &mut buf);
    bench_mosum_f64(&mut buf);
    print_row("MosumF64<64>::update", &mut buf);
    bench_shiryaev_roberts(&mut buf);
    print_row("ShiryaevRoberts::update", &mut buf);

    section("Smoothing");
    print_header();
    bench_ema_f64(&mut buf);
    print_row("EmaF64::update", &mut buf);
    bench_ema_i64(&mut buf);
    print_row("EmaI64::update", &mut buf);
    bench_holt_f64(&mut buf);
    print_row("HoltF64::update", &mut buf);

    section("Variance & Correlation");
    print_header();
    bench_welford_f64(&mut buf);
    print_row("WelfordF64::update", &mut buf);
    bench_ewma_var_f64(&mut buf);
    print_row("EwmaVarF64::update", &mut buf);
    bench_covariance_f64(&mut buf);
    print_row("CovarianceF64::update", &mut buf);

    section("Monitoring");
    print_header();
    bench_drawdown_f64(&mut buf);
    print_row("DrawdownF64::update", &mut buf);
    bench_windowed_max_f64(&mut buf);
    print_row("WindowedMaxF64::update", &mut buf);
    bench_windowed_min_f64(&mut buf);
    print_row("WindowedMinF64::update", &mut buf);
    bench_liveness_f64(&mut buf);
    print_row("LivenessF64::record", &mut buf);
    bench_running_min_f64(&mut buf);
    print_row("RunningMinF64::update", &mut buf);
    bench_running_max_f64(&mut buf);
    print_row("RunningMaxF64::update", &mut buf);
    bench_event_rate_f64(&mut buf);
    print_row("EventRateF64::tick", &mut buf);
    bench_queue_delay_i64(&mut buf);
    print_row("QueueDelayI64::update", &mut buf);

    section("Frequency");
    print_header();
    bench_topk(&mut buf);
    print_row("TopK<u64,16>::observe", &mut buf);

    println!();
}
