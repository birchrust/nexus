# nexus-decimal Documentation

Fixed-point decimal arithmetic with compile-time precision.
Built for financial systems that cannot tolerate floating-point error.

## Reading order

1. [overview.md](overview.md) — Why fixed-point, what the crate gives you
2. [types.md](types.md) — `Decimal<B, D>`, picking a backing type and precision
3. [arithmetic.md](arithmetic.md) — Add/sub/mul/div, overflow variants
4. [parsing.md](parsing.md) — String parsing, Display, serde integration
5. [comparison.md](comparison.md) — Equality, ordering, `approx_eq`
6. [conversion.md](conversion.md) — Integer/float/backing conversions
7. [financial.md](financial.md) — Midpoint, spread, tick rounding, bps, percent
8. [patterns.md](patterns.md) — Cookbook: order pricing, P&L, tick handling
9. [caveats.md](caveats.md) — Precision loss, overflow, when to use f64 instead

## Quick reference

| Alias (suggested)          | Backing | Decimals | Domain                |
|----------------------------|---------|----------|-----------------------|
| `type Usd = Decimal<i64, 2>`       | `i64`   | 2        | USD cents             |
| `type EqPrice = Decimal<i64, 4>`   | `i64`   | 4        | Equities (per NMS)    |
| `type FxRate = Decimal<i64, 6>`    | `i64`   | 6        | FX spot               |
| `type BtcPrice = Decimal<i64, 8>`  | `i64`   | 8        | Bitcoin, sats         |
| `type EthWei = Decimal<i128, 18>`  | `i128`  | 18       | Ethereum wei          |

## Related crates

- [nexus-bits](../../nexus-bits/docs/INDEX.md) — pack a `Decimal`'s raw
  value into a bit-field wire format.
