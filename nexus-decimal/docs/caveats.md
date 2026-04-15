# Caveats and failure modes

Things that will trip you up if you don't know they exist.

## Division truncates

`Decimal` division truncates toward zero — the same semantics as
native integer division. There is no implicit rounding.

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let a = D64::new(10, 0);
let b = D64::new(3,  0);
let q = a / b;
assert_eq!(q.to_string(), "3.33333333"); // last digit dropped
```

The final digit is lost. If you need banker's rounding or round-
half-up semantics, do it explicitly: compute the remainder, inspect
it, and add or subtract a single tick.

For fee calculations that must match the exchange's rounding, use
`mul_div` so you round once instead of twice, and test against a
reference set of known fills.

## Multiplication precision

`a * b` rescales exactly once, truncating. For two `D64` values
(`D = 8`), the intermediate product is computed in `i128`, divided
by `SCALE = 10^8`, and narrowed back to `i64`. That means you lose
up to one tick per multiplication — the same as hand-written
integer math, no hidden precision.

When you chain multiplications, chain losses compound. Prefer
`mul_div` or `mul_add` when the library provides them so the
rescaling happens once.

## Overflow at extreme `D` values

`SCALE = 10^D`. As `D` approaches the backing type's maximum:

- `Decimal<i32, 9>`: SCALE = 10^9, integer range ≈ ±2.14
- `Decimal<i64, 18>`: SCALE = 10^18, integer range ≈ ±9.22
- `Decimal<i128, 38>`: SCALE = 10^38, integer range ≈ ±1.7

`Decimal<i128, 38>` can store `1.0` but not `2.0`. This is a valid
configuration for wei-sized values, but it is not a general-purpose
number type.

**Rule of thumb**: if your integer part can exceed the
`backing_max / SCALE` boundary, pick a smaller `D` or a larger
backing.

```rust
use nexus_decimal::Decimal;

// D=18 on i64 — any integer part overflows SCALE.
type D18 = Decimal<i64, 18>;
assert_eq!(D18::from_i32(10), None); // 10 * 10^18 > i64::MAX
```

## Operator overflow always panics

`+`, `-`, `*`, `/`, `%`, and unary `-` panic on overflow in both
debug and release builds. This is a deliberate choice: the Rust
default (`i64::MAX + 1` wraps in release) is the wrong answer for
money.

If you are doing math inside a tight loop where you have already
proven via invariants that overflow cannot happen, use operators.
Otherwise use `checked_` or `try_` and handle the failure.

## `MIN` cannot be negated

`-Decimal::MIN` panics (and `checked_neg` returns `None`) because
`-(i64::MIN)` does not fit in an `i64`. The same for
`abs()` / `checked_abs()`.

Most code paths never hit this because `MIN` is an astronomically
negative number. But sanitize inputs from untrusted sources — a
deserialized `Decimal::MIN` handed to an unchecked operator crashes
your process.

## Float round-trip is not exact

`from_f64(x).to_f64() != x` in general. `f64` has 53 bits of
mantissa; `i64` has 63 bits of precision. Conversions round to the
nearest representable value and lose the last 1–3 digits for
high-magnitude prices.

Do not use `f64` as a ledger format, a serialization format, or a
cross-process exchange format. Use the `Decimal` type directly, or
a string, or the raw backing integer.

## Serde JSON gotchas

The `serde` impl writes `Decimal` as a JSON **string** for a
reason: JSON numbers are decoded as `f64` by most libraries, which
corrupts any value with more than 15 significant digits. If you
deliberately want JSON numbers (e.g., for a schema you don't
control), you need to write your own adapter — the default is
correct and will stay correct.

## When to use `f64` instead

There are legitimate uses for `f64` even in a `Decimal`-centric
system:

- **Model output**: a regression that predicts an expected spread
  lives naturally in floats. Convert to `Decimal` only when you
  cross into order entry.
- **Statistics / telemetry**: p50, p99, rolling averages of any
  quantity that isn't a direct ledger entry.
- **Plotting and display**: charts, dashboards, human-facing
  tooling.

Use `Decimal` for anything that touches the cash ledger, the
exchange, risk limits, or audit logs. Use `f64` for analytics that
summarize those decisions after the fact.

## `no_std` notes

In `no_std` mode (`default-features = false`):

- `from_f64` / `from_f32` are unavailable (they use `f64::round`).
- `std::error::Error` impls are unavailable.
- Everything else — arithmetic, parsing, display, serde (binary
  formats), `num-traits` — is available.

## Performance cliff: chunked division path

For `Decimal<i64, D>` the multiplication hot path splits at
`D = 9`:

- `D ≤ 9` uses chunked magic-multiply division (~14 cycles).
- `D ≥ 10` falls back to native `i128` division (~25 cycles).

Both are fast. But if you benchmark `D64` (`D = 8`) and then decide
to "upgrade" to `D = 10` for extra precision, you will see a
cliff. Measure before you pay for extra digits.

## What is not implemented

- No true-fused `mul_div` for `i128` (needs 256-bit intermediate).
- No scientific notation parsing.
- No locale-aware parsing (thousands separators, comma decimal).
- No square root, log, exp, or transcendental functions. These
  don't have bit-exact decimal answers; use `f64` for model math
  and convert at the ledger boundary.
- No big-integer mode. If you need arbitrary precision, use
  `rust_decimal` or `num-bigint` instead.
