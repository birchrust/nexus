# Saturation — Resource Utilization Threshold

**EMA of utilization + threshold.** The 'S' in Brendan Gregg's USE method.

| Property | Value |
|----------|-------|
| Update cost | ~6 cycles |
| Memory | ~24 bytes |
| Types | `SaturationF64`, `SaturationF32` |
| Output | `Option<Condition>` — `Normal` or `Saturated` |

## What It Does

```
  Utilization with threshold:

  Util%
  100 ┤                        ·  ·
   90 ┤              ·  ·  ·  ·     ·
   80 ┤── ── ── ── ── ── ── ── ── ── ── ── threshold
   70 ┤           ·                    ·
   60 ┤        ·                          ·
   50 ┤  ·  ·                                ·
      └──────────────────────────────────────── t
      Normal  │     Saturated          │ Normal
```

Smooths utilization samples via EMA and signals when the smoothed value
exceeds a threshold. Prevents flapping from momentary spikes.

## Configuration

```rust
let mut sat = SaturationF64::builder()
    .span(20)
    .threshold(0.80)  // saturated above 80%
    .build().unwrap();

match sat.update(cpu_utilization) {
    Some(Condition::Degraded) => shed_load(),
    _ => {}
}
```

## Performance

| Operation | p50 | p99 |
|-----------|-----|-----|
| `SaturationF64::update` | ~6 cycles | ~8 cycles |

One EMA update + one comparison.
