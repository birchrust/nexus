# Parsing and formatting

## String parsing

`Decimal` provides three string constructors:

| Method              | Behavior                                          |
|---------------------|---------------------------------------------------|
| `from_str_exact`    | Rejects any input with more fractional digits than `D` |
| `from_str_lossy`    | Rounds excess digits using banker's rounding      |
| `from_utf8_bytes`   | Same as `from_str_exact`, but takes `&[u8]`       |

The `FromStr` impl delegates to `from_str_exact`.

```rust
use nexus_decimal::{Decimal, ParseError};
use core::str::FromStr;
type D64 = Decimal<i64, 8>;

// Exact — input fits the precision
let price = D64::from_str_exact("123.45").unwrap();
assert_eq!(price.to_raw(), 12_345_000_000);

// Exact — rejects excess precision
assert_eq!(
    D64::from_str_exact("1.123456789"),   // 9 frac digits, D=8
    Err(ParseError::PrecisionLoss),
);

// Lossy — banker's rounding
let rounded = D64::from_str_lossy("1.2345678951").unwrap();
assert_eq!(rounded, D64::new(1, 23_456_790));

// FromStr trait — delegates to from_str_exact
let p: D64 = "0.00000001".parse().unwrap();
assert_eq!(p.to_raw(), 1);
```

### Parser details

The parser is SWAR-based: it processes 8 ASCII digits at a time in
~6 shifts and adds on any 64-bit platform, without SIMD intrinsics.
The scalar tail handles the remaining 0–7 digits. Sign (`+` / `-`)
is permitted. A leading or trailing `.` is an error.

Accumulation happens in `i128` for uniform overflow handling across
backing types, then narrows to `B`.

### `ParseError` variants

| Variant         | When                                       |
|-----------------|--------------------------------------------|
| `InvalidFormat` | Non-digit character, empty string, bare `.`|
| `Overflow`      | Value does not fit in the backing type     |
| `PrecisionLoss` | Excess fractional digits (exact mode only) |

## Display and Debug

`Decimal` implements `Display` via an `itoa`-style digit-pair lookup
table. It prints exactly the stored value and strips trailing zeros
after the decimal point:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

assert_eq!(D64::new(100, 50_000_000).to_string(), "100.5");
assert_eq!(D64::ZERO.to_string(), "0");
assert_eq!(D64::new(-1, 5_000_000).to_string(), "-1.05");
```

### `write_to_buf` — zero-allocation formatting

For wire protocols and hot-path logging, use `write_to_buf` to write
into a pre-allocated 64-byte buffer:

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

let price = D64::new(123, 45_000_000);
let mut buf = [0u8; 64];
let n = price.write_to_buf(&mut buf);
assert_eq!(&buf[..n], b"123.45");
```

This is substantially faster than the `fmt` machinery for hot
serialization paths (order entry, market data responses).

## Serde

Enable the `serde` feature:

```toml
[dependencies]
nexus-decimal = { version = "0.1", features = ["serde"] }
```

The provided impl serializes as a **string** for human-readable
formats (JSON, YAML, TOML) and as the **raw backing integer** for
binary formats (bincode, CBOR, postcard).

```rust,ignore
use nexus_decimal::Decimal;
use serde_json;

type D64 = Decimal<i64, 8>;

#[derive(serde::Serialize, serde::Deserialize)]
struct Fill {
    price: D64,
    qty: D64,
}

let fill = Fill {
    price: D64::from_str_exact("50000.12345678").unwrap(),
    qty:   D64::from_str_exact("0.5").unwrap(),
};

let json = serde_json::to_string(&fill).unwrap();
// {"price":"50000.12345678","qty":"0.5"}
```

Storing as a string in JSON avoids the JavaScript number-precision
trap: `50000.12345678` as an IEEE-754 `f64` survives, but
`0.1 + 0.2 != 0.3`, so any decoder that reads a JSON number into
`f64` will corrupt the value. Quoting sidesteps the issue entirely.

## Edge cases

- **Empty string** → `InvalidFormat`
- **`"."`** → `InvalidFormat` (no digits)
- **`"-0"`** → `ZERO` (two's-complement has no -0)
- **`"-0.5"`** → works correctly via the sign-aware parser
- **Leading zeros** → preserved as part of the integer
- **Trailing zeros** after the decimal → preserved in raw value, stripped on display
- **Scientific notation (`1.5e3`)** → not supported
- **Thousands separators (`1,000.50`)** → not supported

If you need locale-aware parsing, do it before handing the normalized
digit string to `Decimal`.
