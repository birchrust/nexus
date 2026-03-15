# TrendAlert — Trend Direction Detection

**"Is this getting worse over time?"** Internally uses Holt's double
exponential smoothing. Signals when the trend component exceeds a threshold.

| Property | Value |
|----------|-------|
| Update cost | ~12 cycles |
| Memory | ~40 bytes |
| Types | `TrendAlertF64`, `TrendAlertF32` |
| Output | `Option<Trend>` — `Stable`, `Rising`, or `Falling` |

## What It Does

```
  Signal with upward trend:

  Value
  150 ┤                        ·  · ·
  130 ┤                  ·  ·
  110 ┤            ·  ·               trend > threshold
   90 ┤      ·  ·                     → Trend::Rising
   70 ┤  ·
      └──────────────────────────────── t

  Stable signal (same mean, noisy):

  Value
  110 ┤  ·     ·     ·     ·
  100 ┤     ·     ·     ·     ·       trend ≈ 0
   90 ┤  ·     ·     ·     ·         → Trend::Stable
      └──────────────────────────────── t
```

Separates the signal into level + trend, then classifies:
- **`Rising`** — trend > threshold (e.g., latency increasing)
- **`Falling`** — trend < -threshold (e.g., throughput declining)
- **`Stable`** — trend within ±threshold

## Configuration

```rust
let mut ta = TrendAlertF64::builder()
    .alpha(0.3)
    .beta(0.1)
    .trend_threshold(0.5)            // absolute: trend > 0.5 per sample
    // or: .trend_threshold_relative(0.02)  // relative: |trend/level| > 2%
    .build();

match ta.update(sample) {
    Some(Trend::Rising) => { /* getting worse */ }
    Some(Trend::Falling) => { /* improving */ }
    _ => {}
}
```

Supports both absolute and relative thresholds.

## Performance

| Operation | p50 | p99 |
|-----------|-----|-----|
| `TrendAlertF64::update` | ~12 cycles | ~18 cycles |
