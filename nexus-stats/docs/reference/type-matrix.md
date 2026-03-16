# Type Matrix

Full type availability for all algorithms. ✓ = available, — = not applicable.

## Numeric Types

| Algorithm | f32 | f64 | i32 | i64 | i128 |
|-----------|:---:|:---:|:---:|:---:|:----:|
| **Change Detection** | | | | | |
| CUSUM | ✓ | ✓ | ✓ | ✓ | ✓ |
| MOSUM | ✓ | ✓ | ✓ | ✓ | ✓ |
| ShiryaevRoberts | — | ✓ | — | — | — |
| MultiGate | ✓ | ✓ | — | — | — |
| RobustZScore | ✓ | ✓ | — | — | — |
| AdaptiveThreshold | ✓ | ✓ | — | — | — |
| **Smoothing** | | | | | |
| EMA | ✓ | ✓ | ✓ | ✓ | — |
| AsymmetricEMA | ✓ | ✓ | ✓ | ✓ | — |
| KAMA | ✓ | ✓ | — | — | — |
| Kalman1D | ✓ | ✓ | — | — | — |
| Holt | ✓ | ✓ | — | — | — |
| Spring | ✓ | ✓ | — | — | — |
| SlewLimiter | ✓ | ✓ | ✓ | ✓ | ✓ |
| WindowedMedian | ✓ | ✓ | ✓ | ✓ | — |
| **Statistics** | | | | | |
| Welford | ✓ | ✓ | — | — | — |
| EwmaVariance | ✓ | ✓ | — | — | — |
| Covariance | ✓ | ✓ | — | — | — |
| HarmonicMean | ✓ | ✓ | — | — | — |
| **Monitoring** | | | | | |
| Drawdown | ✓ | ✓ | ✓ | ✓ | ✓ |
| RunningMin/Max | ✓ | ✓ | ✓ | ✓ | ✓ |
| WindowedMin/Max | ✓ | ✓ | ✓ | ✓ | ✓ |
| PeakHoldDecay | ✓ | ✓ | ✓ | ✓ | ✓ |
| MaxGauge | ✓ | ✓ | ✓ | ✓ | ✓ |
| Liveness | ✓ | ✓ | ✓ | ✓ | — |
| EventRate | ✓ | ✓ | ✓ | ✓ | — |
| QueueDelay | — | — | ✓ | ✓ | ✓ |
| Saturation | ✓ | ✓ | — | — | — |
| ErrorRate | ✓ | ✓ | — | — | — |
| TrendAlert | ✓ | ✓ | — | — | — |
| Jitter | ✓ | ✓ | ✓ | ✓ | — |
| **Frequency** | | | | | |
| TopK | generic key | | u64 count | | |
| FlexProportion | — | ✓ | — | — | — |
| DecayAccum | — | ✓ | — | — | — |
| **Utilities** | | | | | |
| Debounce | — | — | u32 | u64 | — |
| DeadBand | ✓ | ✓ | ✓ | ✓ | ✓ |
| Hysteresis | ✓ | ✓ | ✓ | ✓ | ✓ |
| BoolWindow | — | — | runtime-sized | | |
| PeakDetector | ✓ | ✓ | ✓ | ✓ | ✓ |
| LevelCrossing | ✓ | ✓ | ✓ | ✓ | ✓ |
| FirstDiff | ✓ | ✓ | ✓ | ✓ | ✓ |
| SecondDiff | ✓ | ✓ | ✓ | ✓ | ✓ |

## Integer vs Float Decision

**Use integer types when:**
- Signal is naturally integer (nanoseconds, ticks, counts)
- No floating point is available (embedded `no_std`)
- You want deterministic results (no float rounding)

**Use float types when:**
- Signal is naturally continuous (prices, rates, percentages)
- Algorithm requires division or transcendentals (Welford, Kalman, Holt)
- You need exponential smoothing with fine-grained alpha

## Feature-Gated Types

| Feature | What it enables |
|---------|----------------|
| `std` (default) | Hardware `sqrt`/`exp` for Welford, ShiryaevRoberts, etc. |
| `libm` | Pure Rust `sqrt`/`exp` for `no_std` environments |
| `alloc` (planned) | Runtime-sized windows for MOSUM, WindowedMedian, KAMA, BoolWindow |
