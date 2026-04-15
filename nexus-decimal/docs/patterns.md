# Patterns — cookbook

Composed recipes. All examples use `Decimal<i64, 8>` as `D64` unless
noted; swap in your own domain alias.

## Order pricing with tick rounding

Reject an order that cannot be expressed on the venue's tick grid:

```rust
use nexus_decimal::Decimal;
use core::str::FromStr;
type D64 = Decimal<i64, 8>;

#[derive(Debug, PartialEq)]
enum OrderError {
    OffTick,
    CrossedMarket,
}

fn check_limit(
    price: D64,
    tick: D64,
    bid: D64,
    ask: D64,
) -> Result<D64, OrderError> {
    // Snap to the nearest tick and compare. Off-tick orders are rejected.
    let snapped = price.round_to_tick(tick).ok_or(OrderError::OffTick)?;
    if snapped != price {
        return Err(OrderError::OffTick);
    }
    if bid > ask {
        return Err(OrderError::CrossedMarket);
    }
    Ok(snapped)
}

let tick = D64::from_str("0.01").unwrap();
let bid  = D64::from_str("100.00").unwrap();
let ask  = D64::from_str("100.01").unwrap();

assert!(check_limit(D64::from_str("100.00").unwrap(), tick, bid, ask).is_ok());
assert_eq!(
    check_limit(D64::from_str("100.005").unwrap(), tick, bid, ask),
    Err(OrderError::OffTick),
);
```

## Realized P&L on a fill stream

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

#[derive(Copy, Clone)]
enum Side { Buy, Sell }

struct Fill { side: Side, qty: D64, price: D64 }

struct Position {
    net_qty: D64,    // positive long, negative short
    avg_cost: D64,   // weighted-average cost basis
    realized: D64,   // realized P&L
}

impl Position {
    fn apply(&mut self, fill: &Fill) {
        let signed_qty = match fill.side {
            Side::Buy => fill.qty,
            Side::Sell => -fill.qty,
        };
        let new_qty = self.net_qty + signed_qty;

        if self.net_qty.is_zero() || (self.net_qty.signum() == signed_qty.signum()) {
            // Opening or adding — recompute average cost.
            // avg = (old_qty * old_avg + fill_qty * fill_px) / new_qty
            let old_notional = self.net_qty.mul_div(self.avg_cost, D64::ONE).unwrap();
            let add_notional = signed_qty.mul_div(fill.price, D64::ONE).unwrap();
            let total = old_notional + add_notional;
            self.avg_cost = total.try_div(new_qty).unwrap_or(D64::ZERO);
        } else {
            // Reducing or crossing — realize P&L on the closed portion.
            let closed = if signed_qty.signum() != self.net_qty.signum() {
                // Same-magnitude close — cap at current net.
                let cap = self.net_qty.checked_abs().unwrap();
                if fill.qty > cap { cap } else { fill.qty }
            } else {
                D64::ZERO
            };
            let pnl_per_share = match fill.side {
                Side::Sell => fill.price - self.avg_cost,
                Side::Buy  => self.avg_cost - fill.price,
            };
            self.realized += pnl_per_share.mul_div(closed, D64::ONE).unwrap();
        }
        self.net_qty = new_qty;
    }
}
```

The point is: with `Decimal`, `avg_cost` is bit-exact. Two runs on
the same fill sequence produce identical ledger entries. The same
code in `f64` drifts.

## Spread capture as a percentage of mid

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

fn spread_bps(bid: D64, ask: D64) -> Option<D64> {
    let spread = ask.spread(bid)?;       // None if crossed
    let mid = bid.midpoint(ask);
    if mid.is_zero() { return None; }
    // spread / mid * 10_000, single rounding
    spread.mul_div(D64::from_i32(10_000)?, mid)
}

let bid = D64::new(100, 0);
let ask = D64::new(100, 50_000_000);
let bps = spread_bps(bid, ask).unwrap();
// 50 bps
```

Note there is intentionally no `spread_bps` method on `Decimal` —
this is two lines and makes the call site explicit about the
formula.

## Tick-size handling for venues with price-dependent ticks

Many equities venues use larger ticks for higher-priced stocks.
Compute the tick lazily:

```rust
use nexus_decimal::Decimal;
use core::str::FromStr;
type D64 = Decimal<i64, 8>;

fn tick_for(price: D64) -> D64 {
    if price < D64::from_str("1.00").unwrap() {
        D64::from_str("0.0001").unwrap()
    } else if price < D64::from_str("100.00").unwrap() {
        D64::from_str("0.01").unwrap()
    } else {
        D64::from_str("0.05").unwrap()
    }
}

let px = D64::from_str("123.4567").unwrap();
let tick = tick_for(px);
let snapped = px.floor_to_tick(tick).unwrap();
assert_eq!(snapped.to_string(), "123.45");
```

## Fee model: maker / taker rebates in basis points

```rust
use nexus_decimal::Decimal;
type D64 = Decimal<i64, 8>;

#[derive(Copy, Clone)]
enum Role { Maker, Taker }

fn fee(notional: D64, role: Role) -> D64 {
    let bps = match role {
        Role::Maker => D64::from_bps(-2).unwrap(), // 2 bps rebate
        Role::Taker => D64::from_bps(5).unwrap(),  // 5 bps fee
    };
    // notional * bps (bps is already a fraction, not a percent)
    notional.mul_div(bps, D64::ONE).unwrap()
}

let notional = D64::new(10_000, 0);
let maker_fee = fee(notional, Role::Maker); // -2.0
let taker_fee = fee(notional, Role::Taker); // 5.0
assert_eq!(maker_fee.to_string(), "-2");
assert_eq!(taker_fee.to_string(), "5");
```

## Wire format: packing a price into a bit field

For compact on-the-wire representations, a `Decimal`'s raw backing
integer can be packed into a bit-field alongside other metadata.
`Decimal` is `#[repr(transparent)]`, so `to_raw()` gives you the
exact bit pattern you need.

```rust,ignore
use nexus_decimal::Decimal;
use nexus_bits::bit_storage;

type D64 = Decimal<i64, 8>;

#[bit_storage(repr = u128)]
pub struct Quote {
    #[field(start = 0, len = 64)]
    price: i64,         // D64::to_raw()
    #[field(start = 64, len = 32)]
    size: u32,
    #[field(start = 96, len = 24)]
    seq: u32,
    #[flag(120)]
    is_bid: bool,
}

fn pack(px: D64, size: u32, seq: u32, is_bid: bool) -> Quote {
    Quote::builder()
        .price(px.to_raw())
        .size(size)
        .seq(seq)
        .is_bid(is_bid)
        .build()
        .unwrap()
}
```

On the decode side, call `D64::from_raw(quote.price())` to recover
the typed value. The precision constant `D` is part of the wire
contract between both sides — if one side uses `D = 6` and the
other `D = 8`, prices will be off by 100×. Encode `D` into your
protocol version or choose a single precision for all venues.

See [nexus-bits patterns](../../nexus-bits/docs/patterns.md) for
more wire-format examples.
