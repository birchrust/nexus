# EventRate — Smoothed Events Per Unit Time

**EMA of inter-arrival times, inverted to give rate.** "How many events
per second is this source producing?"

| Property | Value |
|----------|-------|
| Update cost | ~6 cycles |
| Memory | ~32 bytes |
| Types | `EventRateF64`, `EventRateF32`, `EventRateI64`, `EventRateI32` |

## API

```rust
let mut rate = EventRateF64::builder().span(20).build();

rate.tick(timestamp);  // record an event
if let Some(r) = rate.rate() {
    println!("{r} events per time unit");
}
```

Internally: EMA of time between events, rate = 1 / smoothed_interval.

**Note:** Integer variants (`EventRateI64`, `EventRateI32`) provide
`tick()` and `interval()` only — no `rate()` method, since `1 / interval`
truncates to zero for integer intervals > 1. Use float variants if you
need the rate directly, or compute it yourself with appropriate scaling.

## When to Use Something Else

- Need to detect silence → [Liveness](liveness.md) (adds deadline check)
- Need throughput in bytes/sec → compute bytes/interval and use [EMA](ema.md)
- Need exact count in window → use a sliding window counter
