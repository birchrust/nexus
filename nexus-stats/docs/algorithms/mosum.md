# MOSUM — Moving Sum Change Detector

**Windowed complement to CUSUM.** Detects transient spikes rather than
persistent shifts. Uses a ring buffer of deviations — anomalies clear
automatically when they leave the window.

| Property | Value |
|----------|-------|
| Update cost | ~6 cycles |
| Memory | ~N×8 bytes (ring buffer) |
| Types | `MosumF64`, `MosumF32`, `MosumI64`, `MosumI32` |
| Requires | `alloc` feature (runtime window size) |
| Priming | After N samples (window full) |
| Output | `Option<Direction>` — same as CUSUM |

## What It Does

```
  CUSUM vs MOSUM on a transient spike:

  Signal:
  Value
  120 ┤                 · · ·
  100 ┤──·──·──·──·──·──·──·──·──·──·──·──·──
      └──────────────────────────────────────── t
                        ↑ spike

  CUSUM (accumulates forever):
       ┤                    ╱───── stays elevated
       ┤                 ╱╱
    0 ─┤────────────────╱
       └──────────────────────────────────────── t

  MOSUM (windowed — spike exits window):
       ┤                 ╱╲
       ┤              ╱╱    ╲╲
    0 ─┤─────────────╱         ╲──────────────
       └──────────────────────────────────────── t
                     ↑           ↑
                  spike enters   spike leaves
                  window         window — back to normal
```

CUSUM would stay elevated after the spike (accumulated evidence doesn't
decay). MOSUM naturally clears when the spike samples leave the window.

## When to Use It

**Use MOSUM when:**
- You want to detect transient anomalies (spikes, bursts)
- The anomaly is temporary — you don't need to remember it forever
- You want self-clearing detection

**Use CUSUM instead when:**
- You want to detect persistent mean shifts that don't clear

## Configuration

```rust
let mut mosum = MosumF64::builder(100.0)  // target
    .window_size(64)                       // window size
    .threshold(200.0)
    .min_samples(64)  // primes after window fills
    .build().unwrap();
```

## Examples

### Trading — Latency Spike Detection
```rust
// Detect 10-sample latency spikes, auto-clear after
let mut spike = MosumF64::builder(baseline_latency)
    .window_size(10)
    .threshold(spike_threshold)
    .build().unwrap();
```

### Networking — Burst Detection
```rust
// Detect traffic bursts over a 32-sample window
let mut burst = MosumI64::builder(normal_rate)
    .window_size(32)
    .threshold(burst_threshold)
    .build().unwrap();
```

## Performance

| Operation | p50 | p99 |
|-----------|-----|-----|
| `MosumF64::update` (window=64) | 6 cycles | 7 cycles |

O(1) per update: add new deviation, subtract evicted deviation from
running sum, compare against threshold. Stack-allocated ring buffer.
