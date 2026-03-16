# nexus-stats Documentation

## Getting Started

- [Which algorithm do I need?](guides/choosing.md) — Decision tree for common problems
- [Quick examples](guides/quickstart.md) — Copy-paste recipes for common patterns

## Algorithm Reference

Each algorithm has a deep-dive doc covering what it does, how it works,
when to use it (and when not to), configuration guidance, and cross-domain
examples.

### Change Detection
- [CUSUM](algorithms/cusum.md) — Persistent mean shift detection
- [MOSUM](algorithms/mosum.md) — Transient spike detection
- [Shiryaev-Roberts](algorithms/shiryaev-roberts.md) — Optimal change detection
- [MultiGate](algorithms/multi-gate.md) — Layered anomaly filter with graded severity
- [RobustZScore](algorithms/robust-z-score.md) — MAD-based anomaly scoring
- [AdaptiveThreshold](algorithms/adaptive-threshold.md) — Z-score anomaly detection

### Smoothing & Filtering
- [EMA](algorithms/ema.md) — Exponential moving average (float and integer)
- [AsymmetricEMA](algorithms/asym-ema.md) — Different smoothing for rising vs falling
- [KAMA](algorithms/kama.md) — Kaufman adaptive moving average
- [Kalman1D](algorithms/kalman1d.md) — 1D Kalman filter with velocity
- [Holt](algorithms/holt.md) — Double exponential smoothing with trend
- [CriticallyDampedSpring](algorithms/spring.md) — Smooth target chasing
- [SlewLimiter](algorithms/slew.md) — Hard rate-of-change clamp
- [WindowedMedian](algorithms/windowed-median.md) — Robust median filter

### Statistics
- [Welford](algorithms/welford.md) — Online mean, variance, standard deviation
- [EwmaVariance](algorithms/ewma-var.md) — Exponentially weighted variance
- [Covariance](algorithms/covariance.md) — Online covariance and Pearson correlation
- [HarmonicMean](algorithms/harmonic-mean.md) — Correct average for rates

### Monitoring
- [Drawdown](algorithms/drawdown.md) — Peak-to-trough decline tracking
- [RunningMin/Max](algorithms/running-min-max.md) — All-time extrema
- [WindowedMin/Max](algorithms/windowed-min-max.md) — Sliding window extrema
- [PeakHoldDecay](algorithms/peak-hold.md) — Peak envelope with decay
- [MaxGauge](algorithms/max-gauge.md) — Reset-on-read maximum
- [Liveness](algorithms/liveness.md) — Source alive/dead detection
- [EventRate](algorithms/event-rate.md) — Smoothed events per unit time
- [CoDel](algorithms/codel.md) — Controlled Delay queue monitor
- [Saturation](algorithms/saturation.md) — Resource utilization threshold
- [ErrorRate](algorithms/error-rate.md) — Failure rate tracking
- [TrendAlert](algorithms/trend-alert.md) — Trend direction detection
- [Jitter](algorithms/jitter.md) — Signal variability measurement

### Frequency & Counting
- [TopK](algorithms/topk.md) — Space-Saving top-K frequent items
- [FlexibleProportions](algorithms/flex-proportion.md) — Per-entity fraction tracking
- [DecayingAccumulator](algorithms/decay-accum.md) — Event-driven score with time decay

### Utilities
- [Debounce](algorithms/debounce.md) — N consecutive events before triggering
- [DeadBand](algorithms/dead-band.md) — Change suppression below threshold
- [Hysteresis](algorithms/hysteresis.md) — Binary decision with different thresholds
- [BoolWindow](algorithms/bool-window.md) — Sliding pass/fail rate
- [PeakDetector](algorithms/peak-detector.md) — Local maxima/minima detection
- [LevelCrossing](algorithms/level-crossing.md) — Threshold crossing counter
- [FirstDiff / SecondDiff](algorithms/diff.md) — Discrete derivative and acceleration

## Use Cases

Domain-specific recipes showing how to compose primitives for real problems.

- [Latency Monitoring](use-cases/latency-monitoring.md) — Tracking, alerting, and diagnosing latency
- [Backpressure Detection](use-cases/backpressure.md) — Early warning before buffers fill
- [Anomaly Detection](use-cases/anomaly-detection.md) — Bad data, outliers, impossible events
- [Feed Health](use-cases/feed-health.md) — Monitoring data sources for degradation
- [Rate Management](use-cases/rate-management.md) — Tracking and adapting to rate limits
- [Capacity Planning](use-cases/capacity-planning.md) — Load distribution and trend analysis
- [Game Performance](use-cases/game-performance.md) — Frame timing, stutter, adaptive quality
- [Network Quality](use-cases/network-quality.md) — RTT, jitter, packet loss, bandwidth
- [Industrial Monitoring](use-cases/industrial-monitoring.md) — Sensor validation, process control
- [SRE Observability](use-cases/sre-observability.md) — SLOs, error budgets, resource tracking

## Guides

- [Choosing an Algorithm](guides/choosing.md) — Decision tree
- [Quick Start](guides/quickstart.md) — Copy-paste recipes
- [Composing Primitives](guides/composition.md) — Building complex monitors from simple parts
- [Parameter Tuning](guides/parameter-tuning.md) — How to set alpha, threshold, slack, etc.
- [Integer vs Float](guides/integer-vs-float.md) — When to use which variant
- [no_std Usage](guides/no-std.md) — Embedded and kernel use cases
- [Runtime vs Const Generic](guides/runtime-vs-const.md) — alloc feature flag explained

## Reference

- [Type Matrix](reference/type-matrix.md) — Full type availability table
- [Performance](reference/performance.md) — Cycle counts, memory usage, benchmarks
- [Glossary](reference/glossary.md) — ARL, z-score, MAD, EMA, etc.
