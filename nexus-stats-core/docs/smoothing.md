# Smoothing

Base smoothing primitives. Module path: `nexus_stats_core::smoothing`. For anything beyond these — Holt, KAMA, Kalman1d, Spring, Huber, Hampel, WindowedMedian — see [`nexus-stats-smoothing`](../../nexus-stats-smoothing/docs/INDEX.md).

## At a glance

| Type | What it does | Notes |
|------|--------------|-------|
| `EmaF64` / `EmaF32` | Classic exponentially-weighted moving average | Builder, halflife or alpha |
| `EmaI64` / `EmaI32` | Integer-fixed-point EMA | No NaN check, no Result on update |
| `AsymEmaF64` / `AsymEmaI64` | Different alpha for rising vs falling samples | Drawdown-aware |
| `SlewLimiterF64` / `SlewLimiterI64` | Hard rate-of-change clamp | Not statistical — deterministic |

---

## EMA

The classic `y_t = alpha * x_t + (1 - alpha) * y_{t-1}`.

```rust
use nexus_stats_core::smoothing::EmaF64;

let mut ema = EmaF64::builder().halflife(50.0).build().unwrap();
for sample in stream { ema.update(sample).unwrap(); }
let smoothed = ema.value().unwrap();
```

**Use for:** the default single-signal smoother. Parameter selection is about half-life — see [umbrella parameter guide](../../nexus-stats/docs/guides/parameter-tuning.md).

**Caveats:** lags trending signals. If your signal has drift, reach for [`HoltF64`](../../nexus-stats-smoothing/docs/holt.md) instead.

---

## AsymEMA

Two different alphas: one for `sample > current`, one for `sample < current`. Useful when you want to react fast to degradation but smooth slowly on recovery (or vice versa).

```rust
use nexus_stats_core::smoothing::AsymEmaF64;

let mut aema = AsymEmaF64::builder()
    .up_halflife(5.0)     // react fast to latency going up
    .down_halflife(50.0)  // smooth the recovery
    .build()
    .unwrap();

for us in latencies { aema.update(us).unwrap(); }
```

**Use for:** SLO monitoring (stay on top of degradations, don't flap on recoveries), drawdown tracking, queue-depth monitoring.

---

## SlewLimiter

A hard clamp on rate of change: output can change by at most `max_rate` per update.

```rust
use nexus_stats_core::smoothing::SlewLimiterF64;

let mut slew = SlewLimiterF64::new(0.01).unwrap(); // max change of 0.01 per step
for setpoint in targets {
    let output = slew.update(setpoint).unwrap();
    // `output` tracks `setpoint` but never moves more than 0.01 per step.
}
```

**Use for:** rate-limited control setpoints, slew-limited UI, guaranteed-bounded step size for safety.

**Caveats:** deterministic, not statistical — noise passes straight through, just slowly. Combine with an EMA if the input is noisy.

---

## Integer variants

`EmaI64` / `EmaI32` / `AsymEmaI64` / `SlewLimiterI64` use fixed-point arithmetic for hot paths where you can't afford float ops (hard real-time budgets, SIMD-heavy loops). Interface is the same, but:

- `update` does not return `Result` (no NaN to guard against).
- Internal precision is governed by a shift factor, documented on each type.
- Parameters are still converted from half-life at builder time.

---

## Cross-references

- Advanced smoothers — Holt, KAMA, Kalman1d, Spring, Huber, Hampel, WindowedMedian — all in [`nexus-stats-smoothing`](../../nexus-stats-smoothing/docs/INDEX.md).
- Streaming variance around an EMA: [`EwmaVarF64`](statistics.md#ewmavarf64--exponentially-weighted-variance).
