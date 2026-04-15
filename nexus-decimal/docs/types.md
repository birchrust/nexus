# Types and type selection

`nexus-decimal` exposes one generic type:

```rust
pub struct Decimal<B: Backing, const DECIMALS: u8> { /* ... */ }
```

You pick the backing integer and the number of fractional digits
yourself, then define a domain alias:

```rust
use nexus_decimal::Decimal;

type Usd     = Decimal<i64, 2>;   // U.S. dollars and cents
type EqPrice = Decimal<i64, 4>;   // NMS-style equity prices
type FxRate  = Decimal<i64, 6>;   // FX spot rates
type BtcPx   = Decimal<i64, 8>;   // Bitcoin satoshis
type EthWei  = Decimal<i128, 18>; // Ethereum 1e18 base units
```

The crate intentionally ships no predefined aliases. Your domain,
your types, your names.

## Backing types

| `B`     | Bytes | Max `D` | Integer range (at `D`)             |
|---------|-------|---------|-------------------------------------|
| `i32`   | 4     | 9       | ±(2.1 × 10^9) / SCALE              |
| `i64`   | 8     | 18      | ±(9.2 × 10^18) / SCALE             |
| `i128`  | 16    | 38      | ±(1.7 × 10^38) / SCALE             |

`SCALE` is `10^D`. As `D` increases, the representable integer range
shrinks. `Decimal<i64, 8>` can hold values in roughly
`±9.2 × 10^10` — fine for any realistic crypto price, but you cannot
store a notional like `$10^15` in it.

## Choosing `D`

Pick `D` so that:

1. It matches the smallest unit your domain actually reports.
2. Intermediate multiplications do not overflow.

### USD cash ledger — `D = 2`

Cents are the canonical unit. `Decimal<i64, 2>` gives you a range of
roughly ±9.2 × 10^16 USD — more than the total money supply. Use it
for ledgers, fills, settlement balances.

```rust
use nexus_decimal::Decimal;
use core::str::FromStr;
type Usd = Decimal<i64, 2>;

let balance = Usd::from_str("12345.67").unwrap();
assert_eq!(balance.to_raw(), 1_234_567);
```

### Equities / FX — `D = 4` to `D = 6`

Reg NMS equities quote in 4 decimal places below $1 and 2 decimal
places above. Use `D = 4` if you want a uniform type.

FX spot rates are usually 5–6 decimal places (`EUR/USD = 1.07525`).
`Decimal<i64, 6>` is a clean fit.

### Crypto — `D = 8` to `D = 18`

- Bitcoin: 8 decimal places (satoshi).
- Most altcoins: 8 decimals.
- Ethereum wei: 18 decimals — must use `i128`.

```rust
use nexus_decimal::Decimal;
use core::str::FromStr;
type BtcPx  = Decimal<i64, 8>;
type EthWei = Decimal<i128, 18>;

let btc = BtcPx::from_str("50000.12345678").unwrap();
let wei = EthWei::from_str("0.000000001000000000").unwrap(); // 1 gwei
assert!(!wei.is_zero());
```

### Mixing precisions in the same system

Different instruments can use different types — the compiler will
keep them separate and you cannot accidentally add cents to satoshis.
You are responsible for conversion at boundaries.

## Constants and basic queries

Every concrete `Decimal<B, D>` exposes:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let _ = D64::ZERO;        // 0
let _ = D64::ONE;         // 1.00000000
let _ = D64::NEG_ONE;     // -1.00000000
let _ = D64::MIN;         // minimum representable
let _ = D64::MAX;         // maximum representable
let _ = D64::SCALE;       // 10^D as the backing integer (100_000_000)
let _ = D64::DECIMALS;    // D as u8

let x = D64::ONE;
assert!(!x.is_zero());
assert!(x.is_positive());
assert!(!x.is_negative());
assert_eq!(x.signum(), 1);
```

## Constructors

```rust
use nexus_decimal::Decimal;
use core::str::FromStr;
type D64 = Decimal<i64, 8>;

// const — for compile-time values
const PRICE: D64 = D64::new(100, 50_000_000);      // 100.50
const RAW:   D64 = D64::from_raw(12_345_000_000);  // 123.45

// runtime — string, integer, float
let from_str = D64::from_str("123.45").unwrap();
let from_i32 = D64::from_i32(42).unwrap();
let from_i64 = D64::from_i64(42).unwrap();
let from_flt = D64::from_f64(1.5).unwrap();        // requires `std`

// signed construction — handles -0.5 correctly
let neg_half = D64::from_parts(0, 50_000_000, true).unwrap();
assert_eq!(neg_half.to_raw(), -50_000_000);
```

`Decimal::new(integer, fractional)` panics on overflow. If you need
a fallible const-friendly constructor, use `from_parts`.

See [parsing.md](parsing.md) for the full string-parsing story.
