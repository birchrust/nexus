# Comparison and ordering

`Decimal<B, D>` derives `PartialEq`, `Eq`, `PartialOrd`, `Ord`, and
`Hash`. Because the backing is an integer and there is no NaN,
`Eq` and `Ord` are total — unlike `f64`.

## Equality

Two `Decimal`s are equal iff their raw backing integers are equal.
This means equality respects trailing zeros in the source string:

```rust
use nexus_decimal::Decimal;
use core::str::FromStr;
type D64 = Decimal<i64, 8>;

let a = D64::from_str("1.5").unwrap();
let b = D64::from_str("1.50000000").unwrap();
assert_eq!(a, b); // both store 150_000_000
```

Two `Decimal`s with **different type parameters** are different
types. `Decimal<i64, 2>::new(1, 50)` and `Decimal<i64, 8>::new(1, 50_000_000)`
cannot be compared with `==`. You must convert explicitly.

## Ordering

`PartialOrd` and `Ord` compare the raw integers. No surprises. This
makes `Decimal` a valid key for `BTreeMap` and `BinaryHeap` without
any workaround.

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let a = D64::new(100, 0);
let b = D64::new(100, 50_000_000);
let c = D64::new(-1,   0);

assert!(a < b);
assert!(c < a);

let mut prices = [b, a, c];
prices.sort();
assert_eq!(prices, [c, a, b]);
```

## `approx_eq` — tolerance-based comparison

Tick-rounded prices from different sources can differ by a tick or
two even when "the same". `approx_eq` returns `true` if the absolute
difference is at most a tolerance:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let exchange_a = D64::new(50000, 12_345_000);
let exchange_b = D64::new(50000, 12_346_000);

let tol = D64::new(0, 10_000); // 0.0001
assert!(exchange_a.approx_eq(exchange_b, tol));
```

`approx_eq` is overflow-safe: if the difference would overflow
(e.g., comparing `MIN` to `MAX` with a small tolerance) it returns
`false` rather than panicking.

## NaN-free guarantees

The backing is a signed integer. There is no NaN, no `+∞`, no `-∞`,
no `-0`. Every `Decimal` is comparable with every other `Decimal`
of the same type, and sorting is stable.

This matters for:

- **Sorted books**: `BTreeMap<Price, Level>` works without wrapping
  in `OrderedFloat` or similar.
- **Consistent hashing**: `HashMap<Price, _>` is safe because `Eq`
  and `Hash` agree.
- **Min/max aggregations**: `.iter().min()` is infallible.

## Clamping

`clamp_price(min, max)` restricts a value to a range:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let limit = D64::new(100, 0);
let floor = D64::new(99,  0);
let ceil  = D64::new(101, 0);

assert_eq!(D64::new(50,  0).clamp_price(floor, ceil), floor);
assert_eq!(D64::new(150, 0).clamp_price(floor, ceil), ceil);
assert_eq!(limit.clamp_price(floor, ceil), limit);
```

Use this for circuit-breaker bands, DLPs, and other "hard bounds"
checks. Unlike `std::cmp::Ord::clamp`, it panics nowhere and is
`const fn`.
