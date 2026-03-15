# nexus-stats

Fixed-memory, zero-allocation streaming statistics for real-time systems.

Every primitive is O(1) per update, fixed memory after construction, and
`no_std` compatible. Designed for event loops, trading systems, and
anywhere you need statistics without latency jitter.

## Quick Start

```rust
use nexus_stats::*;

// Detect latency shifts with CUSUM
let mut cusum = CusumF64::builder(100.0)  // target: 100μs baseline
    .slack(5.0)                            // sensitivity
    .threshold(50.0)                       // decision boundary
    .min_samples(20)                       // warmup
    .build();

for latency in samples {
    match cusum.update(latency) {
        Some(Shift::Upper) => println!("latency degradation detected"),
        Some(Shift::Lower) => println!("latency recovered"),
        _ => {}
    }
}

// Smooth noisy measurements with EMA
let mut ema = EmaF64::builder()
    .span(20)          // ~20-sample smoothing window
    .min_samples(10)
    .build();

if let Some(smoothed) = ema.update(sample) {
    // use smoothed value
}

// Track running statistics with Welford
let mut stats = WelfordF64::new();
stats.update(sample);
if let Some(mean) = stats.mean() {
    println!("mean={mean}, std_dev={}", stats.std_dev().unwrap());
}
```

## Algorithms

### Change Detection

| Type | Algorithm | What It Detects | Cycles (p50) |
|------|-----------|----------------|-------------|
| `CusumF64` | CUSUM (Page, 1954) | Persistent mean shifts (up or down) | 5 |
| `MosumF64<N>` | Moving Sum | Transient spikes within a window | 6 |
| `ShiryaevRobertsF64` | Shiryaev-Roberts | Mean shifts with optimal detection delay | 17 |

CUSUM is the workhorse — detects when the mean of a process has shifted
and reports the direction (`Shift::Upper` or `Shift::Lower`). MOSUM
complements it by catching transient spikes that CUSUM would ignore.
Shiryaev-Roberts offers theoretically optimal detection at the cost of
one `exp()` per update.

### Smoothing

| Type | Algorithm | What It Computes | Cycles (p50) |
|------|-----------|-----------------|-------------|
| `EmaF64` | Exponential Moving Average | Smoothed signal (float) | 5 |
| `EmaI64` | EMA (kernel fixed-point) | Smoothed signal (integer, no float) | 5 |
| `HoltF64` | Holt's Double Exponential | Level + trend | 11 |

EMA is available in float (`EmaF64`) and integer (`EmaI64`) variants. The
integer variant uses the Linux kernel's fixed-point bit-shift pattern — no
floating point at all. Holt's adds trend tracking: "latency isn't just high,
it's getting worse."

Three ways to configure EMA smoothing:
- `.alpha(a)` — direct smoothing factor, α ∈ (0, 1)
- `.halflife(h)` — samples for weight to decay by half
- `.span(n)` — pandas/finance convention, α = 2/(n+1)

### Variance & Correlation

| Type | Algorithm | What It Computes | Cycles (p50) |
|------|-----------|-----------------|-------------|
| `WelfordF64` | Welford's | Online mean, variance, std dev | 10 |
| `EwmaVarF64` | EWMA Variance | Exponentially weighted variance | 12 |
| `CovarianceF64` | Online Covariance | Covariance + Pearson correlation | 12 |

Welford's is numerically stable (no catastrophic cancellation) and supports
`merge()` via Chan's algorithm for parallel aggregation. EWMA Variance
tracks recent volatility (RiskMetrics / JP Morgan 1996 pattern).

### Monitoring

| Type | Algorithm | What It Tracks | Cycles (p50) |
|------|-----------|---------------|-------------|
| `DrawdownF64` | Peak tracker | Peak-to-trough decline | 5 |
| `RunningMinF64` | All-time min | Minimum value ever seen | 5 |
| `RunningMaxF64` | All-time max | Maximum value ever seen | 5 |
| `WindowedMaxF64` | Nichols' (kernel) | Max within a sliding time window | 9 |
| `WindowedMinF64` | Nichols' (kernel) | Min within a sliding time window | 10 |
| `LivenessF64` | EMA + deadline | Source alive/dead detection | 6 |
| `EventRateF64` | EMA of arrivals | Smoothed events per unit time | 6 |
| `QueueDelayI64` | CoDel-inspired | Queue backpressure detection | 7 |

Windowed Min/Max is ported from the Linux kernel's `win_minmax.h` (used by
TCP BBR). Three samples, 24 bytes, O(1) amortized.

QueueDelay detects standing queues before buffers fill — if the minimum
sojourn time exceeds a target for an entire observation window, the queue
has structural congestion.

### Frequency

| Type | Algorithm | What It Tracks | Cycles (p50) |
|------|-----------|---------------|-------------|
| `TopK<K, CAP>` | Space-Saving | Top-K frequent items | 42 (CAP=16) |

Fixed-size top-K tracker. No heap allocation — uses a const-generic array.

## Type Variants

Every algorithm is available as explicit concrete types — no generics to
fight with. Float types use FMA intrinsics; integer types use bit-shift
arithmetic.

| Algorithm | f32 | f64 | i32 | i64 |
|-----------|-----|-----|-----|-----|
| CUSUM | ✓ | ✓ | ✓ | ✓ |
| EMA | ✓ | ✓ | ✓ | ✓ |
| Welford | ✓ | ✓ | | |
| Drawdown | ✓ | ✓ | ✓ | ✓ |
| RunningMin/Max | ✓ | ✓ | ✓ | ✓ |
| WindowedMin/Max | ✓ | ✓ | ✓ | ✓ |
| EWMA Variance | ✓ | ✓ | | |
| Liveness | ✓ | ✓ | ✓ | ✓ |
| EventRate | ✓ | ✓ | ✓ | ✓ |
| QueueDelay | | | ✓ | ✓ |
| MOSUM | ✓ | ✓ | ✓ | ✓ |
| Holt's | ✓ | ✓ | | |
| Covariance | ✓ | ✓ | | |
| Shiryaev-Roberts | | ✓ | | |
| TopK | | | generic key, u64 count | |

## Common API Patterns

All types follow consistent conventions:

- **Builder pattern** for config-driven types (`CusumF64::builder(target)`)
- **`const fn new()`** for zero-config types (`WelfordF64::new()`)
- **Priming** — returns `None` until `min_samples` reached
- **`is_primed()`** — check if enough data has been seen
- **`count()`** — total samples processed
- **`reset()`** — clear state for operational/admin reset
- **`#[must_use]`** — compiler warns if you ignore return values

## Performance

All measurements in CPU cycles (`rdtsc`), pinned to a single core.
Batch of 64 updates per sample to amortize timing overhead.

```bash
cargo build --release --example perf_stats -p nexus-stats
taskset -c 0 ./target/release/examples/perf_stats
```

## Features

| Feature | Default | What |
|---------|---------|------|
| `std` | yes | Hardware intrinsics for `sqrt`/`exp` |
| `libm` | no | Pure Rust math fallback for `no_std` |

One of `std` or `libm` must be enabled. Update hot paths never use
transcendentals — `sqrt` and `exp` are only used in queries (`std_dev()`)
and construction (`halflife()`).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
