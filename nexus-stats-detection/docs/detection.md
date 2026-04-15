# Detection Algorithms

Module path: `nexus_stats_detection::detection`.

---

## MOSUM — Moving Sum

**Types:** `MosumF64`, `MosumF32`. Requires `alloc`.

### What it does

Maintains a sliding window sum of `(sample - target)` and fires when the moving sum exceeds a positive or negative threshold. Like CUSUM but bounded to recent history — old evidence ages out.

```
mosum_t = sum_{i = t - window + 1}^{t} (sample_i - target)
fire if mosum_t > +threshold   (Direction::Up)
fire if mosum_t < -threshold   (Direction::Down)
```

### When to use it

- **Transient spikes.** "Did something go wrong in the last 100 samples?" — a CUSUM would latch the evidence forever, but MOSUM forgets.
- **Local anomalies.** Temporary drift due to a one-off cause (deploy, GC storm, exchange hiccup).
- **Low false alarm rate over long runs.** CUSUM accumulates forever; a slow drift will eventually trigger. MOSUM stays bounded.

### API

```rust
impl MosumF64 {
    pub fn builder(target: f64) -> MosumF64Builder;
    pub fn update(&mut self, sample: f64) -> Result<Option<Direction>, DataError>;
    pub fn is_primed(&self) -> bool;
    pub fn reset(&mut self);
    // value accessors, see source
}

impl MosumF64Builder {
    pub fn window(self, n: u64) -> Self;
    pub fn threshold(self, t: f64) -> Self;
    pub fn build(self) -> Result<MosumF64, ConfigError>;
}
```

### Example — latency spike detector

```rust
use nexus_stats_detection::detection::MosumF64;
use nexus_stats_core::Direction;

let mut mosum = MosumF64::builder(120.0) // target latency 120us
    .window(50)
    .threshold(500.0)                    // 500us-us cumulative budget
    .build()
    .unwrap();

for &latency in &request_latencies {
    if let Some(dir) = mosum.update(latency).unwrap() {
        eprintln!("mosum fired: {dir:?}");
    }
}
```

### Parameter tuning

- `window`: trade responsiveness vs stability. 20-200 typical.
- `threshold`: roughly `3 * window * sigma_expected`. Start conservative and tighten.

### Caveats

- Not optimal in the minimax sense — use Shiryaev-Roberts if you need that.
- Window sum has a burn-in period; `is_primed()` is false until the window fills.

---

## ShiryaevRoberts — Optimal change detection

**Type:** `ShiryaevRobertsF64`. Requires `std` or `libm`.

### What it does

Shiryaev-Roberts is the asymptotically optimal (minimax) change-point detector when the pre-change and post-change distributions are known. It maintains a likelihood-ratio statistic `R_t` and fires when `R_t` exceeds a threshold.

Under the standard framing, you provide the pre-change mean (`mu_0`), post-change mean (`mu_1`), and noise scale (`sigma`). The detector updates the likelihood ratio on every sample.

### When to use it

- You *know* what the "bad" regime looks like (from calibration or domain knowledge).
- You want the lowest possible expected detection delay at a fixed false-alarm rate.
- You have Gaussian-ish data with known variance.

### API

```rust
impl ShiryaevRobertsF64 {
    pub fn builder() -> ShiryaevRobertsF64Builder;
    pub fn update(&mut self, sample: f64) -> Result<Option<bool>, DataError>;
    pub fn is_primed(&self) -> bool;
    // more accessors, see source
}

impl ShiryaevRobertsF64Builder {
    pub fn mu_before(self, mu: f64) -> Self;
    pub fn mu_after(self, mu: f64) -> Self;
    pub fn sigma(self, sigma: f64) -> Self;
    pub fn threshold(self, t: f64) -> Self;
    pub fn build(self) -> Result<ShiryaevRobertsF64, ConfigError>;
}
```

### Example — regime-change detector

```rust
use nexus_stats_detection::detection::ShiryaevRobertsF64;

// Known: pre-change mean 0.5, post-change mean 1.2, noise sigma 0.3.
let mut sr = ShiryaevRobertsF64::builder()
    .mu_before(0.5)
    .mu_after(1.2)
    .sigma(0.3)
    .threshold(100.0) // larger = fewer false alarms, slower detection
    .build()
    .unwrap();

for &x in &stream {
    if let Some(true) = sr.update(x).unwrap() {
        println!("regime change detected");
        break;
    }
}
```

### Caveats

- Needs the post-change distribution known. If you don't know it, use CUSUM or MOSUM.
- Log-likelihood ratio accumulates; reset after each detection.

---

## AdaptiveThreshold — EMA-based z-score

**Types:** `AdaptiveThresholdF64`, `AdaptiveThresholdF32`. Requires `std` or `libm`.

### What it does

Maintains a streaming EMA and EW variance. On each update, computes `z = (x - ema) / stddev`. Fires when `|z|` exceeds a configured threshold.

### When to use it

- You want the simplest streaming z-score outlier detector.
- Data is Gaussian-ish (no fat tails).
- You want the baseline to adapt to gradual drift.

### API

```rust
impl AdaptiveThresholdF64 {
    pub fn builder() -> AdaptiveThresholdF64Builder;
    pub fn update(&mut self, sample: f64) -> Result<Option<Condition>, DataError>;
    pub fn z_score(&self) -> Option<f64>;
    // more
}
```

### Example — CPU load outlier

```rust
use nexus_stats_detection::detection::AdaptiveThresholdF64;
use nexus_stats_core::Condition;

let mut at = AdaptiveThresholdF64::builder()
    .halflife(500.0)
    .warning_sigma(2.0)
    .critical_sigma(4.0)
    .build()
    .unwrap();

for &load in &cpu_samples {
    match at.update(load).unwrap() {
        Some(Condition::Warning) => log_warn("cpu anomaly"),
        Some(Condition::Critical) => page_oncall("cpu spike"),
        _ => {}
    }
}
```

### Caveats

- Gaussian assumption. Fat tails → frequent false alarms. Use `RobustZScoreF64` instead.
- The baseline adapts — if the anomaly is *sustained*, the baseline eventually catches up and the alarm clears.

---

## RobustZScore — MAD-based anomaly score

**Types:** `RobustZScoreF64`, `RobustZScoreF32`.

### What it does

Maintains streaming estimates of the median and MAD (median absolute deviation). Computes `z = 0.6745 * (x - median) / MAD`. The factor `0.6745` makes MAD ≈ stddev under Gaussian data, so the score reads in standard-deviation units regardless of the data's actual distribution.

Robust to up to ~50% corruption — unlike AdaptiveThreshold, whose variance estimate gets polluted by the very outliers you're trying to detect.

### When to use it

- **Heavy-tailed data.** Financial returns, sensor flakes, anything with real outliers.
- **Unknown noise model.** MAD is nonparametric.
- **Long-running detectors** where accumulated outliers would degrade a Gaussian baseline.

### API

```rust
impl RobustZScoreF64 {
    pub fn builder() -> RobustZScoreF64Builder;
    pub fn update(&mut self, sample: f64) -> Result<Option<Condition>, DataError>;
    // accessors
}
```

### Example — price outlier detection

```rust
use nexus_stats_detection::detection::RobustZScoreF64;

let mut rz = RobustZScoreF64::builder()
    .warning_threshold(3.0)
    .critical_threshold(6.0)
    .build()
    .unwrap();

for &price in &prices {
    if let Some(cond) = rz.update(price).unwrap() {
        println!("price outlier: {cond:?}");
    }
}
```

### Caveats

- Slightly more expensive than `AdaptiveThreshold` (needs running median or approximation).
- Reaction time is similar — no free lunch.

---

## TrendAlert — Forecast-based detection

**Types:** `TrendAlertF64`, `TrendAlertF32`.

### What it does

Internally maintains a Holt-style level + trend estimate. Fires when the `forecast(h)` function exceeds a threshold — i.e. when you're projected to cross a limit within `h` samples.

### When to use it

- **SLO degradation warnings.** "Latency is increasing — will cross SLO in 30s, page now."
- **Capacity forecasting.** Utilization climbing toward 100%.
- **Anywhere the rate-of-change matters more than the current value.**

### API

```rust
impl TrendAlertF64 {
    pub fn builder() -> TrendAlertF64Builder;
    pub fn update(&mut self, sample: f64) -> Result<Option<Condition>, DataError>;
    // level(), trend(), forecast(steps)
}
```

### Example — SLO degradation

```rust
use nexus_stats_detection::detection::TrendAlertF64;
use nexus_stats_core::Condition;

let mut alert = TrendAlertF64::builder()
    .alpha(0.3)
    .beta(0.05)
    .forecast_horizon(300) // 300 samples ahead
    .warning_threshold(180.0)
    .critical_threshold(200.0)
    .build()
    .unwrap();

for &latency in &stream {
    if let Some(Condition::Critical) = alert.update(latency).unwrap() {
        fire_page();
    }
}
```

### Caveats

- Holt assumes local linearity. Projecting 10000 samples ahead is meaningless.
- Two-parameter tuning (alpha, beta) is harder than one. See [Holt tuning](../../nexus-stats-smoothing/docs/parameter-tuning.md#holt-alpha-and-beta).

---

## MultiGate — Layered severity

**Types:** `MultiGateF64`, `MultiGateF32`.

### What it does

Multiple thresholded "gates" feeding a single state machine. Typically configured with `Warning` and `Critical` levels that each have their own hysteresis and debounce. Emits state *transitions* (`None | Warning | Critical`) rather than per-sample scores.

### When to use it

- **Composite alerts** where "warning" and "critical" need separate debouncing, separate hysteresis, or separate thresholds.
- **Dashboards** that distinguish "degraded" from "failing".

### API

```rust
impl MultiGateF64 {
    pub fn builder() -> MultiGateF64Builder;
    pub fn update(&mut self, sample: f64) -> Result<Option<Condition>, DataError>;
}
```

Configure gates via the builder; see source for the exact parameter set.

---

## Cross-references

- CUSUM and DistributionShift: [`nexus-stats-core::detection`](../../nexus-stats-core/docs/detection.md).
- Smoothing before detection: [`nexus-stats-smoothing`](../../nexus-stats-smoothing/docs/INDEX.md).
- `Condition` and `Direction` enums: `nexus_stats_core::{Condition, Direction}`.
