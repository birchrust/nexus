# nexus-stats-detection

Advanced change detection and signal analysis for [nexus-stats](https://crates.io/crates/nexus-stats).

## Detection Types

- **MosumF64** — Moving sum change detection (requires `alloc`)
- **ShiryaevRobertsF64** — Shiryaev-Roberts change detection (requires `std` or `libm`)
- **AdaptiveThresholdF64** — EMA + Welford adaptive threshold (requires `std` or `libm`)
- **RobustZScoreF64** — Median-based robust Z-score anomaly detection
- **TrendAlertF64** — Holt-based trend alerting
- **MultiGateF64** — Multi-threshold gating

## Signal Types

- **AutocorrelationF64** — Lagged autocorrelation (requires `alloc`)
- **CrossCorrelationF64** — Lagged cross-correlation (requires `alloc`)
- **EntropyF64** — Shannon entropy estimation (requires `alloc` + (`std` or `libm`))
- **TransferEntropyF64** — Transfer entropy between series (requires `alloc` + (`std` or `libm`))

## Estimation Types

- **SprtBernoulli** — Sequential Probability Ratio Test (requires `std` or `libm`)
- **SprtGaussian** — Gaussian SPRT (requires `std` or `libm`)

## Usage

Enable the `detection` feature on `nexus-stats` for unified import paths:

```rust
use nexus_stats::detection::RobustZScoreF64;
use nexus_stats::signal::AutocorrelationF64;
```

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or
[MIT license](../LICENSE-MIT) at your option.
