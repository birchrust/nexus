# no_std Usage

Using nexus-stats in embedded and kernel environments.

## Feature Configuration

```toml
[dependencies]
# Default: uses std for hardware sqrt/exp
nexus-stats = "1.0"

# no_std with libm (pure Rust math):
nexus-stats = { version = "1.0", default-features = false, features = ["libm"] }

# no_std, no alloc, no libm (only algorithms that don't need sqrt/exp):
nexus-stats = { version = "1.0", default-features = false }
```

## What Needs Math Functions

Most algorithms are pure arithmetic — no `sqrt` or `exp` needed:

| Algorithm | Needs `std` or `libm`? |
|-----------|----------------------|
| EMA, CUSUM, Drawdown, RunningMin/Max | No |
| WindowedMin/Max, SlewLimiter, DeadBand | No |
| Debounce, Hysteresis, PeakDetector | No |
| FirstDiff, SecondDiff, LevelCrossing | No |
| BoolWindow, MaxGauge | No |
| MOSUM (integer), Jitter (integer) | No |
| Welford (`std_dev()` query) | Yes (`sqrt`) |
| ShiryaevRoberts | Yes (`exp`) |
| EMA (`halflife()` constructor) | Yes (`exp`) |
| Kalman1D, Spring | No (Padé approximant, no transcendentals) |

If you only use algorithms that don't need math functions, you can
skip both `std` and `libm` features entirely.

## Integer Variants for Embedded

The integer EMA (`EmaI64`, `EmaI32`) uses the Linux kernel's fixed-point
pattern — bit-shift arithmetic with no floating point at all. This is
ideal for Cortex-M and other platforms without FPU.

## Memory Budgets

All primitives are stack-allocated with known sizes. No heap allocation.

| Primitive | Memory |
|-----------|--------|
| EMA, CUSUM | 24-56 bytes |
| Welford | 24 bytes |
| WindowedMin/Max | 48 bytes |
| MOSUM<32> | 256 bytes (ring buffer) |
| WindowedMedian<16> | 256 bytes (2 arrays) |
| Everything else | 8-32 bytes |
