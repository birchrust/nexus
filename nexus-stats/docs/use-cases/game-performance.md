# Use Case: Game Performance Monitoring

Frame timing, stutter detection, and adaptive quality.

## Recipe: Frame Time Dashboard

```rust
use nexus_stats::*;

// Smooth FPS display (don't show every fluctuation)
let mut frame_ema = EmaF64::builder().alpha(0.05).build();

// Detect stutters (frame time spikes)
let mut stutter = AdaptiveThresholdF64::builder()
    .span(60)           // ~1 second at 60fps
    .z_threshold(2.0)   // 2σ above recent average
    .min_samples(30)
    .build();

// Peak frame time with decay
let mut worst_frame = PeakHoldF64::builder()
    .hold_samples(60)   // hold for 1 second
    .decay_rate(0.97)
    .build();

// Each frame:
let dt_ms = frame_time_ms;

let smooth_dt = frame_ema.update(dt_ms).unwrap_or(dt_ms);
let fps = 1000.0 / smooth_dt;

let is_stutter = matches!(stutter.update(dt_ms), Some(Anomaly::High));
let worst = worst_frame.update(dt_ms);

// Display: fps, worst recent frame, stutter indicator
```

## Recipe: Adaptive Quality

```rust
use nexus_stats::*;

// Track frame budget utilization
let mut budget = SaturationF64::builder()
    .span(30)
    .threshold(0.90)  // 90% of 16.67ms budget = 15ms
    .build();

// Track trend (is performance getting worse?)
let mut trend = TrendAlertF64::builder()
    .alpha(0.2).beta(0.1).trend_threshold(0.1).build();

let utilization = frame_time_ms / target_frame_time_ms;

match (budget.update(utilization), trend.update(frame_time_ms)) {
    (Some(Pressure::Saturated), Some(Trend::Rising)) => {
        reduce_quality();  // overloaded and getting worse
    }
    (Some(Pressure::Normal), Some(Trend::Falling)) => {
        increase_quality(); // headroom and improving
    }
    _ => {} // hold current quality
}
```

## Primitives Used

| Primitive | Role |
|-----------|------|
| [EMA](../algorithms/ema.md) | Smooth FPS display |
| [AdaptiveThreshold](../algorithms/adaptive-threshold.md) | Stutter detection |
| [PeakHoldDecay](../algorithms/peak-hold.md) | Worst recent frame |
| [Saturation](../algorithms/saturation.md) | Budget utilization |
| [TrendAlert](../algorithms/trend-alert.md) | Performance trending |
| [Spring](../algorithms/spring.md) | Smooth camera/UI value transitions |
