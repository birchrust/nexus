# CoDel — Controlled Delay Queue Monitor

**CoDel-inspired backpressure detection.** Detects standing queues before
buffers fill by tracking the minimum time items spend waiting.

| Property | Value |
|----------|-------|
| Update cost | ~7 cycles |
| Memory | ~80 bytes |
| Types | `CoDelI64`, `CoDelI32` |
| Priming | Configurable via `min_samples` |
| Output | `Option<Condition>` — `Normal` or `Degraded` |

## What It Does

```
  Queue sojourn time over time:

  Sojourn (μs)
  500 ┤                    ·  ·
  400 ┤                 · ·  ·  · ·
  300 ┤              ·  ·  ·  ·  ·  ·  ·
  200 ┤── ── ── ── ── ── ── ── ── ── ── ── ── target
  150 ┤           ·
  100 ┤  · ·  ·  ·                          ·  ·
   50 ┤    ·  ·                                ·  · ·  ·
      └───────────────────────────────────────────────────── t
       Normal     │   Elevated (standing queue)  │  Normal
                  │                              │
                  │  even the MIN is above       │
                  │  target for a full window    │

  Windowed minimum:
  500 ┤
  400 ┤
  300 ┤                 ╱────────────╲
  200 ┤── ── ── ── ──╱── ── ── ── ──╲── ── ── target
  100 ┤──────────────╱                ╲─────────
   50 ┤                                  ╲──────
      └───────────────────────────────────────── t
       min below    │ min ABOVE target  │ min below
       target       │ → Elevated        │ → Normal
```

The key insight from CoDel: if even the *minimum* sojourn time in a
window exceeds your target, the queue has a **standing backlog** — not
just a temporary burst. A burst causes a few high sojourn times, but
the minimum stays low (some items still get through quickly). A standing
queue means *every* item waits too long.

## When to Use It

**Use CoDel when:**
- You have a producer-consumer queue and want early warning
- You want to detect congestion before buffers fill and you hit hard backpressure
- You can timestamp items at enqueue time

**Don't use CoDel when:**
- You don't have timestamps on queued items → use [Saturation](saturation.md) with queue depth instead
- You want to detect individual slow items → use [AdaptiveThreshold](adaptive-threshold.md) on sojourn times
- You want to detect mean shifts in processing time → use [CUSUM](cusum.md)

## How It Works

On each dequeue, compute `sojourn = now - enqueue_time` and feed it to
CoDel along with the current timestamp.

Internally:
1. **WindowedMin** (Nichols' algorithm) tracks the minimum sojourn
   time over the observation window
2. If the windowed minimum exceeds the target for an entire window
   duration → `Condition::Degraded`
3. When the minimum drops below target → `Condition::Normal`

This is the **measurement layer** of CoDel. CoDel itself is a *policy*
(drop packets when congested). CoDel gives you the signal — you
decide the response (slow producers, shed load, alert, etc.).

## Configuration

```rust
let mut qd = CoDelI64::builder()
    .target(100_000)       // 100μs target sojourn time
    .window(1_000_000_000) // 1 second observation window (in nanoseconds)
    .min_samples(10)
    .build().unwrap();
```

### Parameters

| Parameter | What | Default | Guidance |
|-----------|------|---------|----------|
| `target` | Max acceptable sojourn time | Required | Set to your latency budget |
| `window` | Observation window (ticks) | Required | How long min must exceed target |
| `min_samples` | Warmup | 0 | Set to 10+ for noisy startup |

**Choosing target:** Your latency budget for queue wait time. If your
end-to-end budget is 100μs and processing takes 60μs, the queue budget
is 40μs. Set target to 40μs (40,000 ns).

**Choosing window:** How long a standing queue must persist before you
act. Shorter = faster detection, more false alarms from bursts. Longer =
more confident, slower response. 100ms-1s is typical.

## Examples by Domain

### Trading — Aeron Publication Backpressure

```rust
// Detect backpressure before Aeron buffers fill
let mut qd = CoDelI64::builder()
    .target(10_000)         // 10μs max queue wait
    .window(100_000_000)    // 100ms observation window
    .min_samples(20)
    .build().unwrap();

// At dequeue from ring buffer:
let sojourn_ns = now_ns - message.enqueue_timestamp;
match qd.update(now_ns as u64, sojourn_ns) {
    Some(Condition::Degraded) => {
        // Standing queue detected — slow down strategies
        throttle_strategies();
    }
    Some(Condition::Normal) => {
        // Queue is healthy
    }
    None => {} // not primed yet
}
```

### Networking — Request Queue Health

```rust
// Web server request queue monitoring
let mut qd = CoDelI64::builder()
    .target(5_000_000)       // 5ms max queue wait
    .window(10_000_000_000)  // 10s window
    .build().unwrap();

// On each request completion:
let wait_ms = request.started_processing - request.received;
if let Some(Condition::Degraded) = qd.update(now_ns, wait_ms) {
    shed_load();  // start rejecting requests
}
```

### Gaming — Command Queue Monitoring

```rust
// Detect when the render command queue backs up
let mut qd = CoDelI64::builder()
    .target(2_000)           // 2ms max command wait
    .window(100_000_000)     // 100ms window
    .build().unwrap();
```

## Composition Patterns

### CoDel + Saturation

"Comprehensive resource monitoring":

```rust
// CoDel catches standing queues (latency-based)
// Saturation catches high utilization (throughput-based)
let mut qd = CoDelI64::builder().target(t).window(w).build().unwrap();
let mut sat = SaturationF64::builder().alpha(0.1).threshold(0.8).build().unwrap();

// On dequeue:
let sojourn = now - enqueue_time;
let queue_pressure = qd.update(now, sojourn);
let util_pressure = sat.update(utilization);

match (queue_pressure, util_pressure) {
    (Some(Condition::Degraded), _) => { /* queue backing up */ }
    (_, Some(Condition::Degraded))     => { /* resource maxed out */ }
    _                                   => { /* healthy */ }
}
```

### CoDel + CUSUM

"Detect both transient backpressure and permanent degradation":

CoDel catches standing queues. CUSUM on the sojourn time detects
when the mean processing time has shifted (e.g., downstream got slower).

## Performance

| Operation | p50 | p99 |
|-----------|-----|-----|
| `CoDelI64::update` | 7 cycles | 10 cycles |

Internally uses WindowedMin (Nichols' algorithm) — 3 samples, O(1)
amortized. The integer path avoids float conversion entirely.

## Background: CoDel

CoDel (Controlled Delay) was developed by Kathleen Nichols and Van
Jacobson in 2012 for active queue management in network routers. The
key insight: **good queues** (transient bursts) have low minimum delay,
while **bad queues** (standing backlog) have high minimum delay.

CoDel's policy is to drop packets when the queue is "bad." CoDel
provides the measurement without imposing a policy — your system decides
what to do when pressure is elevated.

Reference: Nichols, K. and Jacobson, V. "Controlling Queue Delay."
*ACM Queue* 10.5 (2012): 20-34.

Linux kernel implementation: `include/net/codel_impl.h`
