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
    .build().unwrap();

for latency in samples {
    match cusum.update(latency) {
        Some(Direction::Rising) => println!("latency degradation detected"),
        Some(Direction::Falling) => println!("latency recovered"),
        _ => {}
    }
}

// Smooth noisy measurements with EMA
let mut ema = EmaF64::builder()
    .span(20)          // ~20-sample smoothing window
    .min_samples(10)
    .build().unwrap();

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

45 algorithms across 7 categories. See [full documentation](docs/INDEX.md)
for deep-dives on each algorithm.

### Change Detection

| Type | What It Detects | p50 |
|------|----------------|-----|
| `CusumF64` | Persistent mean shifts (up or down) | 5 |
| `MosumF64` | Transient spikes within a window | 6 |
| `ShiryaevRobertsF64` | Mean shifts with optimal detection delay | 17 |
| `MultiGateF64` | Graded anomalies: Accept/Unusual/Suspect/Reject | 12 |
| `RobustZScoreF64` | MAD-based outlier scoring with estimator freeze | 12 |
| `AdaptiveThresholdF64` | Z-score anomalies with self-learning baseline | 15 |

### Smoothing & Filtering

| Type | What It Computes | p50 |
|------|-----------------|-----|
| `EmaF64` / `EmaI64` | Exponential moving average (float / integer) | 5 |
| `AsymEmaF64` | Different alpha for rising vs falling | 11 |
| `KamaF64` | Kaufman adaptive MA (adapts to trend/noise) | 16 |
| `Kalman1dF64` | 1D Kalman filter with velocity tracking | 25 |
| `HoltF64` | Double exponential (level + trend) | 11 |
| `SpringF64` | Critically damped spring (smooth target chasing) | 12 |
| `SlewF64` | Hard rate-of-change clamp | 3 |
| `WindowedMedianF64` | Robust median filter (outlier-immune) | 132 |

### Statistics

| Type | What It Computes | p50 |
|------|-----------------|-----|
| `WelfordF64` | Online mean, variance, std dev (Chan's merge) | 10 |
| `EwmaVarF64` | Exponentially weighted variance | 12 |
| `CovarianceF64` | Online covariance + Pearson correlation | 12 |
| `HarmonicMeanF64` | Correct average for rates/throughputs | 5 |

### Monitoring

| Type | What It Tracks | p50 |
|------|---------------|-----|
| `DrawdownF64` | Peak-to-trough decline, max drawdown | 5 |
| `RunningMinF64` / `RunningMaxF64` | All-time extrema | 5 |
| `WindowedMaxF64` / `WindowedMinF64` | Sliding window extrema (Nichols'/BBR) | 9 |
| `PeakHoldF64` | Peak envelope with hold + decay | 7 |
| `MaxGaugeF64` | Reset-on-read maximum (Netflix pattern) | 5 |
| `LivenessF64` | Source alive/dead detection | 6 |
| `EventRateF64` | Smoothed events per unit time | 6 |
| `QueueDelayI64` | Queue backpressure detection (CoDel-inspired) | 7 |
| `SaturationF64` | Resource utilization threshold (USE method) | 6 |
| `ErrorRateF64` | Failure rate with weighted severity | 6 |
| `TrendAlertF64` | Trend direction (Stable/Rising/Falling) | 12 |
| `JitterF64` | Signal variability measurement | 6 |

### Frequency & Scoring

| Type | What It Tracks | p50 |
|------|---------------|-----|
| `TopK<K, CAP>` | Space-Saving top-K frequent items | 42 |
| `FlexProportionGlobal/Entity` | Per-entity fraction with lazy decay | O(1) |
| `DecayAccumF64` | Event-driven score with time decay | O(1) |

### Utilities

| Type | What It Does | p50 |
|------|-------------|-----|
| `DebounceU32` | N consecutive events before triggering | 2 |
| `DeadBandF64` | Suppress changes below threshold | 2 |
| `HysteresisF64` | Binary decision with different rising/falling thresholds | 3 |
| `BoolWindow` | Sliding pass/fail rate over last N events | 6 |
| `PeakDetectorF64` | Local maxima/minima with prominence | 3 |
| `LevelCrossingF64` | Threshold crossing counter | 2 |
| `FirstDiffF64` | Discrete derivative (rate of change) | 2 |
| `SecondDiffF64` | Discrete acceleration | 2 |

## Type Variants

Explicit concrete types — no generics to fight with. Float types use FMA
intrinsics; integer types use bit-shift arithmetic.

| Algorithm | f32 | f64 | i32 | i64 |
|-----------|:---:|:---:|:---:|:---:|
| CUSUM, EMA, Drawdown, Jitter | ✓ | ✓ | ✓ | ✓ |
| RunningMin/Max, WindowedMin/Max | ✓ | ✓ | ✓ | ✓ |
| Liveness, EventRate, AsymEMA | ✓ | ✓ | ✓ | ✓ |
| SlewLimiter, DeadBand, Hysteresis | ✓ | ✓ | ✓ | ✓ |
| PeakHold, PeakDetector, LevelCrossing | ✓ | ✓ | ✓ | ✓ |
| FirstDiff, SecondDiff, MOSUM | ✓ | ✓ | ✓ | ✓ |
| Welford, EwmaVar, Covariance, HarmonicMean | ✓ | ✓ | | |
| Holt, KAMA, Kalman1D, Spring | ✓ | ✓ | | |
| MultiGate, RobustZScore, AdaptiveThreshold | ✓ | ✓ | | |
| Saturation, ErrorRate, TrendAlert | ✓ | ✓ | | |
| QueueDelay | | | ✓ | ✓ |
| ShiryaevRoberts | | ✓ | | |

## Common API Patterns

All types follow consistent conventions:

- **Builder pattern** for config-driven types (`CusumF64::builder(target)`)
- **`const fn new()`** for zero-config types (`WelfordF64::new()`)
- **Priming** — returns `None` until `min_samples` reached
- **`is_primed()`** — check if enough data has been seen
- **`count()`** — total samples processed
- **`reset()`** — clear state for operational/admin reset
- **`seed()`** — skip warmup with pre-loaded baseline (CUSUM, EMA, AdaptiveThreshold)
- **`#[must_use]`** — compiler warns if you ignore return values

## Documentation

Comprehensive [documentation](docs/INDEX.md) including:

- [Which algorithm do I need?](docs/guides/choosing.md) — decision tree
- [Quick start recipes](docs/guides/quickstart.md) — copy-paste examples
- [Parameter tuning guide](docs/guides/parameter-tuning.md) — how to set alpha, slack, etc.
- [Composing primitives](docs/guides/composition.md) — building monitors from parts
- 40 algorithm deep-dives with ASCII diagrams, domain examples, and performance data
- 10 use-case guides (latency, backpressure, anomaly detection, feed health, networking, gaming, SRE, capacity planning, industrial, rate management)

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
| `alloc` | no | Runtime-sized windows (MOSUM, WindowedMedian, KAMA, BoolWindow) |

One of `std` or `libm` must be enabled. Update hot paths never use
transcendentals — `sqrt` and `exp` are only used in queries (`std_dev()`)
and construction (`halflife()`).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
