# Financial operations

Things you always end up writing once and then debugging for a year.
The library ships them so you don't.

## Midpoint

`midpoint(other)` returns the exact average of two prices, computed
in a way that cannot overflow (even for `MAX.midpoint(MAX)`). It
uses the bit-manipulation identity
`avg(a, b) = (a & b) + ((a ^ b) >> 1)`.

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let bid = D64::new(100, 0);
let ask = D64::new(100, 50_000_000); // 100.50
let mid = bid.midpoint(ask);
assert_eq!(mid.to_string(), "100.25");
```

This is a `const fn`. Use it in a compile-time table if you want.

## Spread

`spread(other)` returns `self - other` but only if `self >= other`
(otherwise the market is crossed and the answer is meaningless).

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let bid = D64::new(100, 0);
let ask = D64::new(100, 50_000_000);

assert_eq!(ask.spread(bid), Some(D64::new(0, 50_000_000))); // 0.50
assert_eq!(bid.spread(ask), None);                          // crossed
```

For `spread_bps` there is no dedicated method — compose it from
`spread`, `mul_div`, and your mid-price. The library deliberately
does not add 1-line wrappers.

## Tick rounding

Three flavors:

- `round_to_tick(tick)` — nearest, banker's rounding on the half
- `floor_to_tick(tick)` — round down
- `ceil_to_tick(tick)` — round up

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let raw  = D64::new(1, 23_700_000); // 1.237
let tick = D64::new(0,  5_000_000); // 0.05

assert_eq!(raw.round_to_tick(tick), Some(D64::new(1, 25_000_000))); // 1.25
assert_eq!(raw.floor_to_tick(tick), Some(D64::new(1, 20_000_000))); // 1.20
assert_eq!(raw.ceil_to_tick(tick),  Some(D64::new(1, 25_000_000))); // 1.25
```

`tick` must be positive — otherwise the method panics. This is a
programmer error, not a data error.

Exact-half inputs are resolved with banker's rounding (round half
to even). For a price of `1.025` with a `0.05` tick, the result is
`1.00`, not `1.05` — the even multiple wins. This matches the
convention used by most exchange-facing OMS code and IEEE 754.

## Basis points and percentage

| Method            | Meaning                     |
|-------------------|-----------------------------|
| `to_bps()`        | `self * 10_000`             |
| `from_bps(bps)`   | `bps / 10_000`              |
| `percent_of(p)`   | `self * p / 100` (i32/i64)  |

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let rate = D64::from_bps(25).unwrap(); // 25 bps = 0.0025
assert_eq!(rate.to_string(), "0.0025");

let notional = D64::new(10_000, 0);
let fee_rate = D64::new(0, 5_000_000); // 0.05 (5%)
let fee = notional.percent_of(fee_rate).unwrap();
assert_eq!(fee.to_string(), "5");
```

For `i128` backing there is no `percent_of` — use `mul_div`
explicitly so the rounding policy is visible at the call site.

## `mul_div` — fused multiply-divide

`(self * a) / b` with a single rounding event instead of two. This
is the primitive you reach for when computing cross-rates, fees,
and VWAP contributions.

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

// Cross rate: EUR/USD × USD/JPY → EUR/JPY
let eur_usd = D64::from_raw(108_750_000);   // 1.08750000
let usd_jpy = D64::from_raw(15_042_000_000); // 150.42000000
let eur_jpy = eur_usd.mul_div(usd_jpy, D64::ONE).unwrap();
// 163.58175000
```

For `i64`, `mul_div` keeps the full `i128` intermediate; for `i128`
the implementation delegates to `checked_mul` followed by
`checked_div` (two rounding events — truly fused `i128` needs a
256-bit intermediate, which is not yet implemented). If you are
writing i128 fee math and rounding drift matters, do the math in
`Decimal<i128, D>` and validate against a reference.

## Division helpers

`halve`, `div10`, `div100` are trivial wrappers that divide the raw
value by 2, 10, 100 respectively. They exist so the compiler can
optimize to a shift + sign-bit adjustment instead of emitting a
full integer divide on `/`.

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let bid = D64::new(100, 0);
let ask = D64::new(100, 50_000_000);
// Same as bid.midpoint(ask) but cheaper when you know the sum fits:
let mid = (bid + ask).halve();
assert_eq!(mid, D64::new(100, 25_000_000));
```

Prefer `midpoint` for prices — it is overflow-safe for any inputs.
Prefer `halve` for intermediate results that you have already
bounded.

## `approx_eq`

Covered in [comparison.md](comparison.md). Use it for cross-venue
price comparisons, fill reconciliation, and anywhere a tick of
drift is acceptable.

## `clamp_price`

Covered in [comparison.md](comparison.md). Circuit-breaker bands,
DLPs, and collateral floors.
