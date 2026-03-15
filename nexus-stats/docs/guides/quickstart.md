# Quick Start

Copy-paste recipes for common patterns.

## Smooth a Noisy Signal

```rust
use nexus_stats::EmaF64;

let mut ema = EmaF64::builder().span(20).build();

for sample in data {
    if let Some(smoothed) = ema.update(sample) {
        use_smoothed_value(smoothed);
    }
}
```

## Detect When Something Changes

```rust
use nexus_stats::{CusumF64, Shift};

let mut cusum = CusumF64::builder(100.0)  // expected baseline
    .slack(5.0)
    .threshold(50.0)
    .build();

for sample in data {
    match cusum.update(sample) {
        Some(Shift::Upper) => println!("value increased!"),
        Some(Shift::Lower) => println!("value decreased!"),
        _ => {}
    }
}
```

## Track Running Statistics

```rust
use nexus_stats::WelfordF64;

let mut stats = WelfordF64::new();

for sample in data {
    stats.update(sample);
}

println!("mean={:.2}, std_dev={:.2}",
    stats.mean().unwrap(),
    stats.std_dev().unwrap());
```

## Filter Bad Data

```rust
use nexus_stats::{MultiGateF64, Verdict};

let mut gate = MultiGateF64::builder()
    .alpha(0.05)
    .hard_limit_pct(0.50)
    .suspect_z(5.0)
    .min_samples(50)
    .build();

for sample in data {
    match gate.update(sample) {
        Some(Verdict::Accept) => process(sample),
        Some(Verdict::Reject) => log_bad(sample),
        _ => {}
    }
}
```

## Monitor a Data Source

```rust
use nexus_stats::LivenessI64;

let mut live = LivenessI64::builder()
    .span(15)
    .deadline_multiple(5)
    .build();

// On each message:
live.record(now_ns);

// On timer tick:
if !live.check(now_ns) {
    reconnect();
}
```

## Track Queue Health

```rust
use nexus_stats::{QueueDelayI64, QueuePressure};

let mut qd = QueueDelayI64::builder()
    .target(10_000)       // 10μs max wait
    .window(100_000_000)  // 100ms observation
    .build();

// At dequeue:
let wait = now - enqueue_time;
if let Some(QueuePressure::Elevated) = qd.update(now as u64, wait) {
    slow_down();
}
```
