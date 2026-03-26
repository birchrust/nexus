# WindowedMin / WindowedMax — Sliding Window Extrema

**Nichols' algorithm.** Tracks min or max over a sliding time window
using only 3 stored samples. From Linux kernel `win_minmax.h` (TCP BBR).

| Property | Value |
|----------|-------|
| Update cost | ~9 cycles (amortized O(1)) |
| Memory | ~48 bytes |
| Types | `WindowedMaxF64`, `WindowedMinF64`, and F32/I64/I32 variants |
| Output | Current window extremum |
| Error handling | Returns `Result<_, DataError>` on NaN/Inf input |

## What It Does

```
  WindowedMax with 10-tick window:

  Value
  100 ┤        ·
   80 ┤     ·     ·
   60 ┤  ·           ·        ·
   40 ┤                 ·  ·     ·
      └──────────────────────────────────── t
      │←── window ──→│

  WindowedMax output:
  100 ┤        ┌──────┐
   80 ┤     ┌──┘      └──┐
   60 ┤──┌──┘             └──────────
   40 ┤
      └──────────────────────────────────── t
               ↑ peak enters  ↑ peak exits window
               window         → next best promoted
```

The algorithm maintains 3 candidates spanning sub-windows of `window/3`
ticks. When the best candidate expires, the next is promoted. Only
24-48 bytes of state regardless of window size.

## When to Use It

- Min RTT tracking (BBR-style baseline estimation)
- Max throughput over a measurement window
- Best/worst latency in the last N seconds

## When to Use Something Else

- All-time extrema → [RunningMin/Max](running-min-max.md)
- Peak with smooth decay → [PeakHoldDecay](peak-hold.md)
- Max since last check → [MaxGauge](max-gauge.md)

## Configuration

```rust
let mut max = WindowedMaxF64::new(1_000_000_000);  // 1-second window in nanoseconds

let current_max = max.update(now_ns, sample).unwrap();
```

Timestamps are `u64`. The user defines what the units mean (nanoseconds,
ticks, milliseconds, etc.).

## Performance

| Operation | p50 | p99 |
|-----------|-----|-----|
| `WindowedMaxF64::update` | 9 cycles | 12-34 cycles |

The p99 tail is higher when sub-window promotions occur (shuffling
3 samples). p50 is a simple comparison.

## Background

Ported from Linux kernel `include/linux/win_minmax.h` and `lib/win_minmax.c`.
Used by TCP BBR for `BtlBw` (bottleneck bandwidth) estimation over
10 RTT rounds, and `RTprop` (round-trip propagation time) over 10 seconds.

Kathleen Nichols' algorithm. Also described in:
Cardwell, Cheng, Gunn, Yeganeh, Jacobson. "BBR: Congestion-Based
Congestion Control." *ACM Queue* 14.5 (2016).
