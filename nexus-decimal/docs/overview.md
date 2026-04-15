# Overview

`nexus-decimal` is a fixed-point decimal library for financial
workloads. It provides a single generic type, `Decimal<B, D>`, where
`B` is a signed integer backing type (`i32`, `i64`, `i128`) and `D`
is a `const` number of fractional digits.

Under the hood each `Decimal` is just `B` scaled by `10^D`. A
`Decimal<i64, 8>` holding the value `100.5` stores `10_050_000_000`.

## Why not `f64`?

`f64` has 53 bits of mantissa — about 15–17 significant decimal
digits. For prices and P&L that sounds like plenty until you write
code like this:

```text
let a = 0.1_f64;
let b = 0.2_f64;
assert_eq!(a + b, 0.3); // fails — result is 0.30000000000000004
```

Cash ledgers, exchange matching, and accounting cannot round-trip
floats reliably. Every aggregation you do introduces fresh error. The
errors are small per operation but they bias cumulative totals in
directions you cannot predict.

Fixed-point decimals avoid all of this. `100.50 + 0.25` stores exactly
`100_75_000_000` in a `Decimal<i64, 8>`. No rounding, no surprises,
no "epsilon comparisons" scattered through the codebase.

## Why not `rust_decimal` / `bigdecimal`?

Those libraries trade away speed and predictability for arbitrary
precision or wide dynamic range. `nexus-decimal` is intentionally
narrower:

- Single fixed backing integer, chosen at compile time.
- No heap allocation. Ever.
- `const fn` where possible — prices can live in `const`s.
- Overflow is explicit: `checked_`, `saturating_`, `wrapping_`, `try_`.
- Hot-path multiplication is hand-written (chunked magic division for
  `i64`, 192-bit wide arithmetic for `i128`).
- `no_std` compatible.

If you need arbitrary precision, use `rust_decimal`. If you need to
price a million orders per second and guarantee the answer is
bit-exact, use `nexus-decimal`.

## Design guarantees

| Property                | Status                                       |
|-------------------------|----------------------------------------------|
| No allocation           | All operations are stack-only                |
| `no_std`                | Yes, `default-features = false`              |
| `const fn` constructors | Yes — `Decimal::new`, `from_raw`             |
| NaN-free                | Backing is integer; no NaN or Inf possible   |
| Deterministic           | Same input → same output, always             |
| `#[repr(transparent)]`  | Same layout as the backing integer           |

The `repr(transparent)` property matters for FFI and wire protocols —
you can transmute a `Decimal<i64, 8>` to/from `i64` when you need to
store it or send it over the network. See
[../../nexus-bits/docs/patterns.md](../../nexus-bits/docs/patterns.md)
for packing patterns.

## The `D` const generic

`D` is a `u8` decimal place count. It is validated at compile time:

- `i32` allows `D = 0..=9`
- `i64` allows `D = 0..=18`
- `i128` allows `D = 0..=38`

Using `Decimal<i32, 10>` compiles but panics as soon as you touch
`SCALE` or any arithmetic — the scale factor `10^10` does not fit in
an `i32`.

See [types.md](types.md) for choosing the right backing type.
