# Conversions

## From primitive integers

All backing types provide the same set of fallible integer constructors.
They return `Option<Self>` and are `const fn`:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let from_i32 = D64::from_i32(42).unwrap();
let from_i64 = D64::from_i64(42).unwrap();
let from_u32 = D64::from_u32(42).unwrap();
let from_u64 = D64::from_u64(42).unwrap();

// Overflow
assert_eq!(D64::from_i64(i64::MAX), None);
```

These compute `value * SCALE` in `i128` and then narrow. Pass an
integer that is "too big once scaled" and you get `None`.

## To primitive integers

There is no `to_i64` — you read the raw value and choose how to
unscale. If you want the integer part, divide by `SCALE`:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let price = D64::new(123, 45_000_000);
let int_part = price.to_raw() / D64::SCALE;
assert_eq!(int_part, 123);
```

Do not write a general-purpose `to_i64` on top of your app. Think
about what truncation means for the particular value you hold.

## Float conversions

Available only when the `std` feature is enabled (the default).

```rust
use nexus_decimal::{Decimal, ConvertError};
type D64 = Decimal<i64, 8>;

// To f64 — exact for values with ≤15 significant digits
let d = D64::new(1, 23_456_789);
let f: f64 = d.to_f64();
assert_eq!(f, 1.23456789);

// From f64 — rounds to the nearest representable Decimal
let d = D64::from_f64(50_000.12345678).unwrap();

// NaN / Inf / out-of-range → error
assert_eq!(D64::from_f64(f64::NAN),      Err(ConvertError::Overflow));
assert_eq!(D64::from_f64(f64::INFINITY), Err(ConvertError::Overflow));
```

**Round-trip is not guaranteed.** `Decimal → f64 → Decimal` can
change the last digit for values with more than ~15 significant
digits. Avoid float as an intermediate format in settlement code —
use it only for display, charts, and human-facing tooling.

## Backing-type conversions

`Decimal<i32, D>` and `Decimal<i64, D>` are distinct types. To move
between them, go through raw values and the `from_raw` /
`from_i64` / integer constructors. There is intentionally no
blanket `From` impl between backings: a conversion from
`Decimal<i128, 8>` to `Decimal<i64, 8>` is lossy in range, and you
should state which failure policy you want.

```rust
use nexus_decimal::Decimal;

type D128 = Decimal<i128, 8>;
type D64  = Decimal<i64,  8>;

let wide = D128::from_raw(12_345_000_000);
let raw = wide.to_raw();

let narrow: D64 = if raw >= i64::MIN as i128 && raw <= i64::MAX as i128 {
    D64::from_raw(raw as i64)
} else {
    panic!("out of range for i64 backing");
};
assert_eq!(narrow.to_raw(), 12_345_000_000);
```

## Precision conversions

Converting between different `D` values on the same backing is
common — for example, taking an `EqPrice` (`D = 4`) and embedding it
in a USD notional (`D = 2`):

```rust
use nexus_decimal::Decimal;

type EqPx = Decimal<i64, 4>;
type Usd  = Decimal<i64, 2>;

let price = EqPx::from_raw(1_234_500); // 123.4500

// Widen: D=4 -> D=2 -> divide raw by 100
let usd = Usd::from_raw(price.to_raw() / 100);
assert_eq!(usd.to_raw(), 12_345); // 123.45 (truncated)
```

This is the kind of operation you should wrap in a domain function
and test — the library deliberately doesn't hide the arithmetic.
Automated cross-precision conversion would mask a real semantic
decision (truncate? round? banker's round?).

## `num_traits` interoperability

Enable the `num-traits` feature to get `Zero`, `One`, `Num`,
`Signed`, `Bounded`, `CheckedAdd`, `CheckedSub`, `CheckedMul`,
`CheckedDiv`, and `ToPrimitive`. This lets you use `Decimal` with
generic numeric code written against `num-traits`.

```toml
[dependencies]
nexus-decimal = { version = "0.1", features = ["num-traits"] }
```
