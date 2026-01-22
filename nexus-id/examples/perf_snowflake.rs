//! Snowflake benchmark: ID generation latency measurement
//!
//! Measures cycle-accurate latency for snowflake operations using rdtscp.
//!
//! Run with:
//! ```bash
//! # Disable turbo boost for consistent results
//! echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
//!
//! # Pin to a single core
//! cargo build --release --example perf_snowflake
//! sudo taskset -c 2 ./target/release/examples/perf_snowflake
//! ```

use hdrhistogram::Histogram;
use nexus_id::{Snowflake32, Snowflake64};
use std::hint::black_box;
use std::time::{Duration, Instant};

const OPERATIONS: usize = 1_000_000;
const WARMUP: usize = 10_000;

#[inline(always)]
fn rdtscp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let mut aux: u32 = 0;
        std::arch::x86_64::__rdtscp(&raw mut aux)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // Fallback for non-x86
        std::time::Instant::now().elapsed().as_nanos() as u64
    }
}

struct Stats {
    next_same_ts: Histogram<u64>,
    next_new_ts: Histogram<u64>,
    unpack: Histogram<u64>,
}

impl Stats {
    fn new() -> Self {
        Self {
            next_same_ts: Histogram::new(3).unwrap(),
            next_new_ts: Histogram::new(3).unwrap(),
            unpack: Histogram::new(3).unwrap(),
        }
    }

    fn print(&self, name: &str) {
        println!("{}:", name);
        if !self.next_same_ts.is_empty() {
            println!(
                "  next (same ts): p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
                self.next_same_ts.value_at_quantile(0.50),
                self.next_same_ts.value_at_quantile(0.99),
                self.next_same_ts.value_at_quantile(0.999),
                self.next_same_ts.max(),
                self.next_same_ts.len()
            );
        }
        if !self.next_new_ts.is_empty() {
            println!(
                "  next (new ts):  p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
                self.next_new_ts.value_at_quantile(0.50),
                self.next_new_ts.value_at_quantile(0.99),
                self.next_new_ts.value_at_quantile(0.999),
                self.next_new_ts.max(),
                self.next_new_ts.len()
            );
        }
        if !self.unpack.is_empty() {
            println!(
                "  unpack:         p50={:>4}  p99={:>4}  p999={:>5}  max={:>8}  (n={})",
                self.unpack.value_at_quantile(0.50),
                self.unpack.value_at_quantile(0.99),
                self.unpack.value_at_quantile(0.999),
                self.unpack.max(),
                self.unpack.len()
            );
        }
    }
}

/// Benchmark 64-bit snowflake: 42/6/16 layout (65k/ms)
fn bench_snowflake64_trading() -> Stats {
    type TradingId = Snowflake64<42, 6, 16>;
    const SEQ_MAX: usize = TradingId::SEQUENCE_MAX as usize;

    let epoch = Instant::now();
    let mut id_gen = TradingId::new(5, epoch);
    let mut stats = Stats::new();

    // Warmup - advance time to avoid sequence exhaustion
    for i in 0..WARMUP {
        let ms = (i / SEQ_MAX) as u64;
        let now = epoch + Duration::from_millis(ms);
        let _ = black_box(id_gen.next(now));
    }

    // Reset generator
    id_gen = TradingId::new(5, epoch);

    // Benchmark same-timestamp path (sequence increment)
    // Stay within sequence limit per ms
    let ops_per_ms = SEQ_MAX.min(OPERATIONS);
    let base_ms = 1000u64; // Start at 1 second to avoid overlap with warmup

    for i in 0..ops_per_ms {
        let ms = base_ms + (i / SEQ_MAX) as u64;
        let now = epoch + Duration::from_millis(ms);

        let start = rdtscp();
        let id = id_gen.next(now).unwrap();
        let end = rdtscp();

        black_box(id);
        let _ = stats.next_same_ts.record(end.wrapping_sub(start));
    }

    // Reset generator
    id_gen = TradingId::new(5, epoch);

    // Benchmark new-timestamp path (sequence reset)
    // Each iteration uses a different timestamp
    for i in 0..OPERATIONS {
        let now = epoch + Duration::from_millis(2000 + i as u64);

        let start = rdtscp();
        let id = id_gen.next(now).unwrap();
        let end = rdtscp();

        black_box(id);
        let _ = stats.next_new_ts.record(end.wrapping_sub(start));
    }

    // Benchmark unpack
    let sample_id = id_gen
        .next(epoch + Duration::from_millis(OPERATIONS as u64 + 3000))
        .unwrap();
    for _ in 0..OPERATIONS {
        let start = rdtscp();
        let parts = TradingId::unpack(sample_id);
        let end = rdtscp();

        black_box(parts);
        let _ = stats.unpack.record(end.wrapping_sub(start));
    }

    stats
}

/// Benchmark 64-bit snowflake: Twitter layout 41/10/12 (4k/ms)
fn bench_snowflake64_twitter() -> Stats {
    type TwitterId = Snowflake64<41, 10, 12>;
    const SEQ_MAX: usize = TwitterId::SEQUENCE_MAX as usize;

    let epoch = Instant::now();
    let mut id_gen = TwitterId::new(5, epoch);
    let mut stats = Stats::new();

    // Warmup
    for i in 0..WARMUP {
        let ms = (i / SEQ_MAX) as u64;
        let now = epoch + Duration::from_millis(ms);
        let _ = black_box(id_gen.next(now));
    }

    // Reset generator
    id_gen = TwitterId::new(5, epoch);

    // Same-timestamp path - stay within sequence limit
    let ops_per_ms = SEQ_MAX.min(OPERATIONS);
    let base_ms = 1000u64;

    for i in 0..ops_per_ms {
        let ms = base_ms + (i / SEQ_MAX) as u64;
        let now = epoch + Duration::from_millis(ms);

        let start = rdtscp();
        let id = id_gen.next(now).unwrap();
        let end = rdtscp();

        black_box(id);
        let _ = stats.next_same_ts.record(end.wrapping_sub(start));
    }

    // Reset
    id_gen = TwitterId::new(5, epoch);

    // New-timestamp path
    for i in 0..OPERATIONS {
        let now = epoch + Duration::from_millis(2000 + i as u64);

        let start = rdtscp();
        let id = id_gen.next(now).unwrap();
        let end = rdtscp();

        black_box(id);
        let _ = stats.next_new_ts.record(end.wrapping_sub(start));
    }

    // Unpack
    let sample_id = id_gen
        .next(epoch + Duration::from_millis(OPERATIONS as u64 + 3000))
        .unwrap();
    for _ in 0..OPERATIONS {
        let start = rdtscp();
        let parts = TwitterId::unpack(sample_id);
        let end = rdtscp();

        black_box(parts);
        let _ = stats.unpack.record(end.wrapping_sub(start));
    }

    stats
}

/// Benchmark 32-bit snowflake: compact layout 20/4/8 (256/ms)
fn bench_snowflake32() -> Stats {
    type CompactId = Snowflake32<20, 4, 8>;
    const SEQ_MAX: usize = CompactId::SEQUENCE_MAX as usize;

    let epoch = Instant::now();
    let mut id_gen = CompactId::new(5, epoch);
    let mut stats = Stats::new();

    // Warmup
    for i in 0..WARMUP {
        let ms = (i / SEQ_MAX) as u64;
        let now = epoch + Duration::from_millis(ms);
        let _ = black_box(id_gen.next(now));
    }

    // Reset
    id_gen = CompactId::new(5, epoch);

    // Same-timestamp path - stay within sequence limit
    let ops_per_ms = SEQ_MAX.min(OPERATIONS);
    let base_ms = 1000u64;

    for i in 0..ops_per_ms {
        let ms = base_ms + (i / SEQ_MAX) as u64;
        let now = epoch + Duration::from_millis(ms);

        let start = rdtscp();
        let id = id_gen.next(now).unwrap();
        let end = rdtscp();

        black_box(id);
        let _ = stats.next_same_ts.record(end.wrapping_sub(start));
    }

    // Reset
    id_gen = CompactId::new(5, epoch);

    // New-timestamp path
    for i in 0..OPERATIONS {
        let now = epoch + Duration::from_millis(2000 + i as u64);

        let start = rdtscp();
        let id = id_gen.next(now).unwrap();
        let end = rdtscp();

        black_box(id);
        let _ = stats.next_new_ts.record(end.wrapping_sub(start));
    }

    // Unpack
    let sample_id = id_gen
        .next(epoch + Duration::from_millis(OPERATIONS as u64 + 3000))
        .unwrap();
    for _ in 0..OPERATIONS {
        let start = rdtscp();
        let parts = CompactId::unpack(sample_id);
        let end = rdtscp();

        black_box(parts);
        let _ = stats.unpack.record(end.wrapping_sub(start));
    }

    stats
}

/// Benchmark realistic trading scenario: mixed timestamp patterns
fn bench_realistic_trading() -> Stats {
    type TradingId = Snowflake64<42, 6, 16>;
    const SEQ_MAX: u64 = TradingId::SEQUENCE_MAX;
    const BURST_SIZE: u64 = 50; // Average orders per ms burst

    let epoch = Instant::now();
    let mut id_gen = TradingId::new(5, epoch);
    let mut stats = Stats::new();

    // Simulate: burst of orders, then time passes, another burst, etc.
    // Track sequence to advance time before overflow
    let mut current_ms = 0u64;
    let mut seq_in_ms = 0u64;

    for i in 0..OPERATIONS {
        // Advance time if we'd overflow OR every BURST_SIZE to simulate time passing
        if seq_in_ms >= SEQ_MAX || (i as u64 % BURST_SIZE == 0 && i > 0) {
            current_ms += 1;
            seq_in_ms = 0;
        }

        let now = epoch + Duration::from_millis(current_ms);
        let is_new_ts = id_gen.last_timestamp() != current_ms;

        let start = rdtscp();
        let id = id_gen.next(now).unwrap();
        let end = rdtscp();

        black_box(id);
        seq_in_ms += 1;

        if is_new_ts {
            let _ = stats.next_new_ts.record(end.wrapping_sub(start));
        } else {
            let _ = stats.next_same_ts.record(end.wrapping_sub(start));
        }
    }

    // Unpack samples
    let sample_id = id_gen
        .next(epoch + Duration::from_millis(current_ms + 1))
        .unwrap();
    for _ in 0..OPERATIONS {
        let start = rdtscp();
        let parts = TradingId::unpack(sample_id);
        let end = rdtscp();

        black_box(parts);
        let _ = stats.unpack.record(end.wrapping_sub(start));
    }

    stats
}

fn main() {
    println!("SNOWFLAKE ID BENCHMARK");
    println!("Operations: {}, Warmup: {}", OPERATIONS, WARMUP);
    println!("================================================================\n");

    let trading64 = bench_snowflake64_trading();
    let twitter64 = bench_snowflake64_twitter();
    let compact32 = bench_snowflake32();
    let realistic = bench_realistic_trading();

    trading64.print("Snowflake64<42,6,16> (trading: 65k/ms)");
    println!();
    twitter64.print("Snowflake64<41,10,12> (twitter: 4k/ms)");
    println!();
    compact32.print("Snowflake32<20,4,8> (compact: 256/ms)");
    println!();
    realistic.print("Realistic trading (bursts of 50, mixed)");
    println!();

    println!("================================================================");
    println!("COMPARISON (cycles) - next() same timestamp:");
    println!("----------------------------------------------------------------");
    println!("              Trading64    Twitter64    Compact32    Realistic");
    println!(
        "  p50:          {:>4}         {:>4}         {:>4}         {:>4}",
        trading64.next_same_ts.value_at_quantile(0.50),
        twitter64.next_same_ts.value_at_quantile(0.50),
        compact32.next_same_ts.value_at_quantile(0.50),
        realistic.next_same_ts.value_at_quantile(0.50),
    );
    println!(
        "  p99:          {:>4}         {:>4}         {:>4}         {:>4}",
        trading64.next_same_ts.value_at_quantile(0.99),
        twitter64.next_same_ts.value_at_quantile(0.99),
        compact32.next_same_ts.value_at_quantile(0.99),
        realistic.next_same_ts.value_at_quantile(0.99),
    );
    println!(
        "  p999:         {:>4}         {:>4}         {:>4}         {:>4}",
        trading64.next_same_ts.value_at_quantile(0.999),
        twitter64.next_same_ts.value_at_quantile(0.999),
        compact32.next_same_ts.value_at_quantile(0.999),
        realistic.next_same_ts.value_at_quantile(0.999),
    );

    println!();
    println!("COMPARISON (cycles) - next() new timestamp:");
    println!("----------------------------------------------------------------");
    println!("              Trading64    Twitter64    Compact32    Realistic");
    println!(
        "  p50:          {:>4}         {:>4}         {:>4}         {:>4}",
        trading64.next_new_ts.value_at_quantile(0.50),
        twitter64.next_new_ts.value_at_quantile(0.50),
        compact32.next_new_ts.value_at_quantile(0.50),
        realistic.next_new_ts.value_at_quantile(0.50),
    );
    println!(
        "  p99:          {:>4}         {:>4}         {:>4}         {:>4}",
        trading64.next_new_ts.value_at_quantile(0.99),
        twitter64.next_new_ts.value_at_quantile(0.99),
        compact32.next_new_ts.value_at_quantile(0.99),
        realistic.next_new_ts.value_at_quantile(0.99),
    );

    println!();
    println!("COMPARISON (cycles) - unpack():");
    println!("----------------------------------------------------------------");
    println!("              Trading64    Twitter64    Compact32    Realistic");
    println!(
        "  p50:          {:>4}         {:>4}         {:>4}         {:>4}",
        trading64.unpack.value_at_quantile(0.50),
        twitter64.unpack.value_at_quantile(0.50),
        compact32.unpack.value_at_quantile(0.50),
        realistic.unpack.value_at_quantile(0.50),
    );
    println!(
        "  p99:          {:>4}         {:>4}         {:>4}         {:>4}",
        trading64.unpack.value_at_quantile(0.99),
        twitter64.unpack.value_at_quantile(0.99),
        compact32.unpack.value_at_quantile(0.99),
        realistic.unpack.value_at_quantile(0.99),
    );

    println!();
    println!("NOTE: 'same ts' = sequence increment (common case in bursts)");
    println!("      'new ts'  = sequence reset (time advanced)");
}
