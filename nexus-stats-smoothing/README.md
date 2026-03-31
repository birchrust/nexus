# nexus-stats-smoothing

Advanced smoothing algorithms for [nexus-stats](https://crates.io/crates/nexus-stats).

## Types

- **HoltF64 / HoltF32** — Double exponential smoothing (level + trend)
- **SpringF64 / SpringF32** — Critically damped spring (chase without overshoot)
- **Kalman1dF64 / Kalman1dF32** — 1D Kalman filter (position + velocity)
- **KamaF64 / KamaF32** — Kaufman Adaptive Moving Average (requires `alloc`)
- **WindowedMedianF64 / WindowedMedianF32** — Running median (requires `alloc`)

## Usage

Enable the `smoothing` feature on `nexus-stats` for unified import paths:

```rust
use nexus_stats::smoothing::HoltF64;
```

Or depend on this crate directly:

```rust
use nexus_stats_smoothing::HoltF64;
```

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or
[MIT license](../LICENSE-MIT) at your option.
