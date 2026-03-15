# Which Algorithm Do I Need?

Start with your problem. Follow the tree to find the right primitive.

## "I want to smooth a noisy signal"

**How noisy? What matters more — responsiveness or stability?**

- Simple smoothing, good defaults → [**EMA**](../algorithms/ema.md)
  - Float signals: `EmaF64`
  - Integer signals (no float): `EmaI64`
- Need to react fast when trending, slow when noisy → [**KAMA**](../algorithms/kama.md)
  - Adapts its smoothing factor based on signal efficiency ratio
- Need to increase fast but decrease slow (or vice versa) → [**AsymmetricEMA**](../algorithms/asym-ema.md)
  - Different alpha for each direction. TCP RTT uses this pattern.
- Need to track a moving target without overshoot → [**CriticallyDampedSpring**](../algorithms/spring.md)
  - Better than EMA when chasing a target — has velocity, anticipates
- Need to remove impulse noise / outlier spikes → [**WindowedMedian**](../algorithms/windowed-median.md)
  - Median ignores outliers entirely; EMA smears them
- Need level + trend (is it getting worse?) → [**Holt**](../algorithms/holt.md)
  - Double exponential smoothing. Separates level from trend.
- Need adaptive smoothing that gets smarter over time → [**Kalman1D**](../algorithms/kalman1d.md)
  - Like Holt but with principled adaptive gain. Better after restarts.
- Need to chase a moving target without overshoot → [**CriticallyDampedSpring**](../algorithms/spring.md)
  - Has velocity tracking, anticipates rather than lags. Unity's `SmoothDamp`.
- Need to hard-limit how fast the output can change → [**SlewLimiter**](../algorithms/slew.md)
  - Not smoothing — hard clamp on rate of change

## "I want to detect a change in behavior"

**What kind of change?**

- Mean shifted up or down permanently → [**CUSUM**](../algorithms/cusum.md)
  - "Exchange latency increased 20μs and stayed there"
  - Reports direction: `Shift::Upper` or `Shift::Lower`
- Temporary spike, then back to normal → [**MOSUM**](../algorithms/mosum.md)
  - "Latency spiked for 10 seconds, then recovered"
  - Windowed — forgets the spike after the window passes
- Need best possible detection speed → [**ShiryaevRoberts**](../algorithms/shiryaev-roberts.md)
  - Theoretically optimal detection delay. Costs one `exp()` per update.
- Is this value trending upward/downward? → [**TrendAlert**](../algorithms/trend-alert.md)
  - "Not just high — getting worse over time"
- Is this single sample anomalous? → [**AdaptiveThreshold**](../algorithms/adaptive-threshold.md)
  - Z-score against recent baseline. Tells you High/Low direction.

## "I want to detect bad data / outliers"

**How much do you need?**

- Quick, cheap, O(1) → [**RobustZScore**](../algorithms/robust-z-score.md)
  - MAD-based z-score, freezes baseline on reject
- Multi-level graded response → [**MultiGate**](../algorithms/multi-gate.md)
  - Accept / Unusual / Suspect / Reject. Doesn't corrupt baseline.
- Robust statistics (median, MAD, IQR, Tukey fences) → [**WindowedMedian**](../algorithms/windowed-median.md)
  - O(N) per update but gives you everything: median, quartiles, MAD
- Just clamp impossible rate of change → [**SlewLimiter**](../algorithms/slew.md)
  - "Output can't change by more than X per sample"

## "I want to monitor system health"

**What are you monitoring?**

- Is a data source alive? → [**Liveness**](../algorithms/liveness.md)
  - EMA of inter-arrival times + deadline. Call `check(now)` periodically.
- Is a queue building up? → [**QueueDelay**](../algorithms/queue-delay.md)
  - CoDel-inspired. Detects standing queues before buffers fill.
- Is a resource running hot? → [**Saturation**](../algorithms/saturation.md)
  - EMA of utilization + threshold. `Normal` / `Saturated`.
- Is the error rate too high? → [**ErrorRate**](../algorithms/error-rate.md)
  - EMA of success/failure outcomes. Supports weighted severity.
- What's the worst since I last checked? → [**MaxGauge**](../algorithms/max-gauge.md)
  - Reset-on-read maximum. For periodic scraping/alerting.
- Has N consecutive failures occurred? → [**Debounce**](../algorithms/debounce.md)
  - "3 timeouts in a row = dead." Simpler than CUSUM for discrete events.
- How many events per second? → [**EventRate**](../algorithms/event-rate.md)
  - Smoothed rate from timestamps.

## "I want to track statistics"

- Running mean + variance + std dev → [**Welford**](../algorithms/welford.md)
  - Numerically stable. Supports parallel merge (Chan's algorithm).
- Exponentially weighted variance (recent volatility) → [**EwmaVariance**](../algorithms/ewma-var.md)
  - More responsive than Welford. RiskMetrics pattern.
- Are two signals correlated? → [**Covariance**](../algorithms/covariance.md)
  - Online Pearson correlation. Supports merge.
- Average of rates/throughputs → [**HarmonicMean**](../algorithms/harmonic-mean.md)
  - Arithmetic mean of rates is wrong. Harmonic mean is correct.
- Signal variability / jitter → [**Jitter**](../algorithms/jitter.md)
  - EMA of consecutive absolute differences. `jitter_ratio()` for context.

## "I want to track extrema"

- All-time min or max → [**RunningMin / RunningMax**](../algorithms/running-min-max.md)
- Min/max over a time window → [**WindowedMin / WindowedMax**](../algorithms/windowed-min-max.md)
  - Nichols' algorithm, 24 bytes, from TCP BBR
- Peak with hold + decay → [**PeakHoldDecay**](../algorithms/peak-hold.md)
  - "Worst recent spike, fading over time." Smooth envelope.
- Peak-to-trough decline → [**Drawdown**](../algorithms/drawdown.md)
  - Tracks max drawdown from peak. Risk circuit breaker.

## "I want to count / classify"

- Top K most frequent items → [**TopK**](../algorithms/topk.md)
  - Space-Saving algorithm. Fixed memory.
- What % of total is each entity? → [**FlexibleProportions**](../algorithms/flex-proportion.md)
  - Per-entity fraction with lazy decay. Shard balancing.
- How active is this entity recently? → [**DecayingAccumulator**](../algorithms/decay-accum.md)
  - Event-driven score with time decay. Activity/heat scoring.
- Pass/fail rate over last N events → [**BoolWindow**](../algorithms/bool-window.md)
  - Ring of boolean outcomes. Circuit breaker input.

## "I want signal processing utilities"

- Rate of change (first derivative) → [**FirstDiff**](../algorithms/diff.md)
- Acceleration (second derivative) → [**SecondDiff**](../algorithms/diff.md)
- Detect peaks/troughs in a signal → [**PeakDetector**](../algorithms/peak-detector.md)
- Count threshold crossings → [**LevelCrossing**](../algorithms/level-crossing.md)
- Binary decision from noisy signal → [**Hysteresis**](../algorithms/hysteresis.md)
  - Schmitt trigger. Different thresholds for rising vs falling.
- Suppress changes below threshold → [**DeadBand**](../algorithms/dead-band.md)
  - "Don't tell me unless it changed by at least 5%."

## Still not sure?

Common combinations:

| Problem | Primitives |
|---------|-----------|
| "Is latency degrading?" | CUSUM on latency samples |
| "Is latency degrading AND getting worse?" | CUSUM + TrendAlert |
| "Filter bad ticks, track stats on good ones" | MultiGate → Welford (only on Accept) |
| "Monitor queue health with early warning" | QueueDelay + Saturation |
| "Smooth a signal, detect when it goes anomalous" | EMA for smoothing, AdaptiveThreshold for detection |
| "Track failure rate, trip a circuit breaker" | ErrorRate or BoolWindow → Debounce |
| "Load-balance across shards" | FlexibleProportions per shard |
| "Display smoothed latency with worst-case envelope" | EMA (display) + PeakHoldDecay (envelope) |
