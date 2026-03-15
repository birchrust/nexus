# MaxGauge — Reset-on-Read Maximum

**"What's the worst since I last checked?"** Tracks maximum value,
resets when read. Designed for periodic scraping and alerting.

| Property | Value |
|----------|-------|
| Update cost | ~5 cycles |
| Memory | ~8 bytes |
| Types | `MaxGaugeF64`, `MaxGaugeF32`, `MaxGaugeI64`, `MaxGaugeI32` |

## What It Does

```
  Updates:    5  12  3  8  15  2  7     ← samples arrive

  take():                          → 15  (returns max, resets)

  Updates:    4  9  6               ← more samples

  take():                     → 9   (max since last take)

  peek():                     → 9   (reads WITHOUT resetting)
```

## When to Use It

- Periodic metric scraping (Prometheus-style `max_over_interval`)
- "Worst case since last report" alerting
- Not for continuous tracking → use [RunningMax](running-min-max.md) or [WindowedMax](windowed-min-max.md)

## API

```rust
let mut gauge = MaxGaugeF64::new();

gauge.update(10.0);
gauge.update(25.0);
gauge.update(15.0);

assert_eq!(gauge.peek(), Some(25.0));  // reads without reset
assert_eq!(gauge.take(), Some(25.0));  // reads AND resets
assert_eq!(gauge.take(), None);        // nothing since last take
```

Netflix's Atlas telemetry uses this pattern for per-scrape max metrics.

## Performance

| Operation | p50 | p99 |
|-----------|-----|-----|
| `MaxGaugeF64::update` | ~5 cycles | ~5 cycles |
| `MaxGaugeF64::take` | ~5 cycles | ~5 cycles |

One comparison per update. `take()` reads + resets in one operation.
