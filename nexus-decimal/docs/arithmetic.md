# Arithmetic

Every arithmetic operation on `Decimal<B, D>` comes in four variants.
Pick the variant that matches the failure policy you want for that
call site — the library does not decide for you.

## Variant matrix

| Variant       | Return type                | On overflow / div-by-zero       |
|---------------|-----------------------------|----------------------------------|
| `checked_*`   | `Option<Self>`              | `None`                           |
| `try_*`       | `Result<Self, Error>`       | Typed error                      |
| `saturating_*`| `Self`                      | Clamps to `MIN` / `MAX`          |
| `wrapping_*`  | `Self`                      | Wraps modulo `2^BITS` of backing |
| operator      | `Self` (panics on overflow) | Panics in debug AND release      |

Operators (`+`, `-`, `*`, `/`, `%`, unary `-`) always panic on
overflow or division by zero. This is deliberate — in release builds
native integer arithmetic silently wraps, which is the worst possible
outcome for money. `nexus-decimal` uses a `#[cold]` panic helper so
the hot path stays the inline-success branch.

## Add, subtract, negate, absolute value

These are trivial on the backing integer. All variants exist
symmetrically:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let a = D64::new(100, 50_000_000); // 100.50
let b = D64::new(0,   25_000_000); //   0.25

// Checked — returns Option
assert_eq!(a.checked_add(b), Some(D64::new(100, 75_000_000)));

// Try — returns Result<_, OverflowError>
assert!(a.try_sub(b).is_ok());

// Saturating — clamps at MIN/MAX
let huge = D64::MAX;
assert_eq!(huge.saturating_add(D64::ONE), D64::MAX);

// Wrapping — modular
let _ = huge.wrapping_add(D64::ONE);

// Operator — panics on overflow
let sum = a + b;
assert_eq!(sum.to_string(), "100.75");
```

`checked_neg` / `try_neg` fail on `MIN` because `-MIN` does not fit
in a two's-complement signed integer.

## Multiplication

Multiplication is where fixed-point gets interesting. `a * b`
conceptually computes `(a.raw * b.raw) / SCALE`. The implementation
differs per backing:

- **`i32`**: widen to `i64`, multiply, divide by `SCALE`. Exact,
  always fits.
- **`i64`**: widen to `i128`. For `D ≤ 9` the scale fits in a `u32`
  and the division uses a chunked magic-multiply path
  (~14 cycles, 3 `u64` multiplications). For `D ≥ 10` it falls back
  to native `i128` division (~25 cycles).
- **`i128`**: 192-bit wide multiply-and-divide using a hand-written
  `div_192_by_const`. A 64-bit fast path catches the common case
  where both operands already fit in 64 bits.

All of this is invisible to you:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let price = D64::new(100, 50_000_000); // 100.50
let qty   = D64::from_i32(10).unwrap(); // 10.0

assert_eq!(price.checked_mul(qty), Some(D64::new(1005, 0)));
assert_eq!(price * qty, D64::new(1005, 0));
```

### `mul_int` — multiplying by a plain integer

When the right-hand side is a dimensionless count (`shares`,
`contracts`) you do not want to pay for the rescaling step. Use
`mul_int`:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let price = D64::new(100, 50_000_000);
let notional = price.mul_int(100).unwrap(); // price * 100
assert_eq!(notional.to_string(), "10050");
```

This skips the divide-by-SCALE step entirely. It is strictly faster
and strictly safer when you mean "scale by a count", not "multiply
two prices".

### `mul_add` — fused multiply-add

`(self * mul) + add` with a single rescaling step. Useful for fee
calculations and running sums:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let price  = D64::new(100, 0);
let qty    = D64::new(5, 0);
let fees   = D64::new(0, 25_000_000); // 0.25
let total  = price.mul_add(qty, fees).unwrap(); // 500.25
assert_eq!(total.to_string(), "500.25");
```

## Division

`a / b` computes `(a.raw * SCALE) / b.raw`. Division is truncating
toward zero — the same semantics as native integer division.
`wrapping_div` and `saturating_div` assert that the divisor is
nonzero and panic if it is not. `checked_div` and `try_div` return
an option/result instead.

`try_div` uses the dedicated [`DivError`] enum so the caller can
distinguish overflow from division-by-zero:

```rust
use nexus_decimal::{Decimal, DivError};
type D64 = Decimal<i64, 8>;

let a = D64::new(10, 0);
let b = D64::new(3,  0);

let q = a.try_div(b).unwrap();
assert_eq!(q.to_string(), "3.33333333"); // truncated, not rounded

assert_eq!(D64::ONE.try_div(D64::ZERO), Err(DivError::DivisionByZero));
```

See [caveats.md](caveats.md) for why division loses precision.

## Remainder

`%` and `checked_rem` / `wrapping_rem` / `saturating_rem` follow the
sign of the dividend, matching Rust integers. Useful for tick
rounding (see [financial.md](financial.md)).

## Overflow policy recommendations

- **Operators (`+`, `-`, `*`)**: use inside a critical section where
  the invariants already prove overflow cannot happen. Panic is a
  correctness signal — don't suppress it.
- **`checked_*`**: when you want to react to overflow with
  `Option::unwrap_or` or `?` through another `Option`.
- **`try_*`**: when you want typed errors and `?` propagation.
- **`saturating_*`**: for gauges, telemetry, rolling aggregates
  where clamping is the natural semantics.
- **`wrapping_*`**: essentially never, unless you are deliberately
  implementing modular arithmetic.

Never silently convert `None` to `ZERO`. That is the exact bug
fixed-point is supposed to prevent.
