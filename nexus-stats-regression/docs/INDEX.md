# nexus-stats-regression Documentation

Online regression, adaptive learning, and Bayesian estimation. Three submodules:

- `regression` — closed-form and EW regressors.
- `learning` — gradient-based and recursive adaptive filters.
- `estimation` — Kalman filters and Bayesian conjugate estimators.

All types are streaming, O(1) or O(N) per update in feature dimension, and zero-allocation on the hot path (after construction).

## Start Here

- [Overview](overview.md) — Design conventions, feature flags, the regression API shape.
- [Regression](regression.md) — Linear, polynomial, EW-polynomial, transformed, logistic, lagged predictor, Kyle lambda, signal decay.
- [Learning (adaptive filters)](learning.md) — LMS, NLMS, RLS, Huber regression, online K-Means, OnlineGD / AdaGrad / Adam.
- [Estimation](estimation.md) — Kalman2d, Kalman3d, BetaBinomial, GammaPoisson.

## Algorithms

### regression (closed-form + EW)

| Type | Fits |
|------|------|
| `LinearRegressionF64` / `F32` | y = a + bx (ordinary least squares, closed-form, streaming) |
| `EwLinearRegressionF64` / `F32` | EW variant of above |
| `PolynomialRegressionF64` / `F32` | y = a0 + a1 x + a2 x² + ... + ad x^d |
| `EwPolynomialRegressionF64` / `F32` | EW variant |
| `TransformedRegressionF64` | Exp / log / power fits via linearization |
| `LogisticRegressionF64` | Binary classifier via online gradient steps |
| `LaggedPredictor` | y_t = a + b * x_{t-lag} — trading-specific |
| `KyleLambdaF64` | Price impact slope from volume (market microstructure) |
| `SignalDecayCurve` | Fits an exponential decay to IC vs horizon |

### learning (adaptive filters + optimizers)

| Type | Algorithm |
|------|-----------|
| `LmsFilterF64` / `F32` | Least Mean Squares — simplest adaptive filter |
| `NlmsFilterF64` / `F32` | Normalized LMS — step-size invariant to input power |
| `RlsFilterF64` / `F32` | Recursive Least Squares — fast-converging, matrix-inversion-style |
| `HuberRegressionF64` | Outlier-robust linear regression via Huber loss |
| `OnlineKMeansF64` | Streaming k-means cluster assignment |
| `OnlineGdF64` | Plain gradient descent |
| `AdaGradF64` | Per-feature adaptive learning rate |
| `AdamF64` | Adam / AdamW optimizer |

### estimation

| Type | What it is |
|------|------------|
| `Kalman2dF64` / `F32` | 2D Kalman filter with customizable dynamics |
| `Kalman3dF64` / `F32` | 3D Kalman filter |
| `BetaBinomialF64` | Bayesian rate estimation (success/failure) |
| `GammaPoissonF64` / `F32` | Bayesian rate estimation (Poisson count / exposure) |

## Cross-references

- Scalar Kalman: [`Kalman1dF64`](../../nexus-stats-smoothing/docs/kalman1d.md) in `nexus-stats-smoothing`.
- Half-life / Hurst / variance ratio (complement regression for regime classification): [`nexus-stats-core::statistics`](../../nexus-stats-core/docs/statistics.md).
- Umbrella long-form: [`nexus-stats/docs`](../../nexus-stats/docs/INDEX.md).
