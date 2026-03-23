# nexus-decimal Benchmarks

Measured with `criterion` on the development machine. These are
regression baselines — absolute numbers depend on hardware and system
load. Use `cargo bench -p nexus-decimal` to reproduce.

## Arithmetic (criterion, ns)

| Operation | D32 | D64 | D96 | D128 |
|-----------|-----|-----|-----|------|
| add | 4.5 | 4.8 | <1* | <1* |
| mul | 4.5 | 4.7 | <1* | <1* |
| div | 4.5 | <1* | <1* | <1* |
| mul_int | — | 4.8 | — | — |
| midpoint | — | 4.8 | — | — |
| round_to_tick | — | <1* | — | — |

\* Sub-nanosecond times indicate constant folding by LLVM. Criterion
cannot accurately measure operations this fast with constant inputs.
Proper rdtsc cycle-counting with runtime-varied inputs is needed for
absolute numbers. These benchmarks primarily serve as **regression
detection** — a 2x slowdown will show even if absolute values are off.

## String I/O (criterion, ns)

| Operation | D64 | D96 |
|-----------|-----|-----|
| from_str (integer) | 12.3 | — |
| from_str (short frac) | 10.5 | — |
| from_str (full 8dp) | 12.7 | — |
| from_str (negative) | 14.7 | — |
| from_str (small) | 10.7 | — |
| from_str_lossy (excess) | 16.4 | — |
| from_str_exact | — | 19.2 |
| Display (integer) | 26.9 | — |
| Display (fractional) | 36.8 | — |

## Methodology

- `criterion 0.5` with default settings (100 samples, 5s measurement)
- Quick runs shown above used `--warm-up-time 1 --measurement-time 2`
- No CPU pinning or turbo boost control (add for precise measurement)
- All features enabled during benchmark compilation

## Optimization: Chunked Magic Division

The i64 mul/div paths now use chunked u64 division when SCALE < 2^32
(covers D32, D64, and all standard aliases). Three chained u64 divisions,
each optimized by LLVM to a magic multiply + shift.

Criterion reports 88% improvement on D64 mul after the optimization.
Absolute numbers still need rdtsc verification (criterion shows sub-ns
which indicates constant folding on the benchmark inputs).

| Alias | SCALE | Bits | Path |
|-------|-------|------|------|
| D32 | 10^4 | ~14 | Chunked (~14cy) |
| D64 | 10^8 | ~27 | Chunked (~14cy) |
| D96 | 10^12 | ~40 | Native __divti3 (192-bit limb math dominates) |
| D128 | 10^18 | ~60 | Native __divti3 (192-bit limb math dominates) |

## Verified

- [x] cargo-asm: zero `call __divti3` in all release binaries (library, tests, benchmarks)
- [x] Chunked magic division: 3× u64 magic multiplies for SCALE < 2^32

## Future measurement work

- rdtsc cycle-counting for sub-nanosecond operations (criterion constant-folds some paths)
- CPU-pinned measurement with turbo boost disabled
- SIMD string parsing evaluation
