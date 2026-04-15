# Patterns — cookbook

Real wire-format shapes that people actually use. Copy, adapt,
benchmark.

## Snowflake ID

A 64-bit monotonic ID with an embedded timestamp, worker ID, and
per-worker sequence. Classic Twitter layout.

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct SnowflakeId {
    #[field(start = 0,  len = 12)] sequence: u16,  // 0..=4095 per ms
    #[field(start = 12, len = 10)] worker: u16,    // 0..=1023 workers
    #[field(start = 22, len = 42)] timestamp: u64, // ms since epoch
}

let id = SnowflakeId::builder()
    .sequence(4095)
    .worker(1023)
    .timestamp((1u64 << 42) - 1)
    .build()
    .unwrap();

let unpacked = SnowflakeId::from_raw(id.raw());
assert_eq!(unpacked.sequence(), 4095);
assert_eq!(unpacked.worker(), 1023);
```

Overflow of `sequence` within the same millisecond is a signal to
bump `timestamp` by one or spin-wait. The builder's error tells
you exactly which field ran out of room.

## Order flags — side, TIF, post-only, iceberg

```rust
use nexus_bits::{bit_storage, IntEnum};

#[derive(IntEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Side { Buy = 0, Sell = 1 }

#[derive(IntEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TimeInForce { Day = 0, Gtc = 1, Ioc = 2, Fok = 3 }

#[bit_storage(repr = u64)]
pub struct OrderFlags {
    #[field(start = 0, len = 1)]  side: Side,
    #[field(start = 1, len = 2)]  tif: TimeInForce,
    #[flag(3)]                    post_only: bool,
    #[flag(4)]                    iceberg: bool,
    #[flag(5)]                    reduce_only: bool,
    #[field(start = 8, len = 32)] quantity: u32,
}

let o = OrderFlags::builder()
    .side(Side::Buy)
    .tif(TimeInForce::Ioc)
    .post_only(true)
    .iceberg(false)
    .reduce_only(false)
    .quantity(1_000_000)
    .build()
    .unwrap();

assert_eq!(o.side().unwrap(), Side::Buy);
assert_eq!(o.tif().unwrap(), TimeInForce::Ioc);
assert!(o.post_only());
```

Packing the side, TIF, and boolean flags into one `u64` with the
quantity means your order header fits in a single L1 cache line
entry and can ride alongside a price in the same cache line.

## Instrument ID with asset class and exchange

```rust
use nexus_bits::{bit_storage, IntEnum};

#[derive(IntEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AssetClass { Equity = 0, Future = 1, Option = 2, Forex = 3 }

#[derive(IntEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Exchange { Nasdaq = 0, Nyse = 1, Cboe = 2, Cme = 3 }

#[bit_storage(repr = u64)]
pub struct InstrumentId {
    #[field(start = 0, len = 4)]   asset_class: AssetClass,
    #[field(start = 4, len = 4)]   exchange: Exchange,
    #[field(start = 8, len = 24)]  symbol: u32,       // index into a symbol table
    #[field(start = 32, len = 31)] metadata: u32,     // expiry, strike, etc.
    #[flag(63)]                    is_test: bool,
}

let id = InstrumentId::builder()
    .asset_class(AssetClass::Option)
    .exchange(Exchange::Cboe)
    .symbol(123_456)
    .metadata(0)
    .is_test(false)
    .build()
    .unwrap();

assert_eq!(id.symbol(), 123_456);
```

The whole instrument identifier fits in 8 bytes and is
hash-friendly — `HashMap<InstrumentId, T>` is a straightforward
primitive hash.

## Packed market data tick

Top-of-book snapshot in a single `u128`. Stores a price (via
`nexus-decimal::Decimal::to_raw()`), a size, a venue, a sequence,
and a side flag.

```rust,ignore
use nexus_bits::{bit_storage, IntEnum};
use nexus_decimal::Decimal;

type D64 = Decimal<i64, 8>;

#[derive(IntEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Venue { Nasdaq = 0, Nyse = 1, Cboe = 2, Iex = 3 }

#[bit_storage(repr = u128)]
pub struct TickRaw {
    #[field(start = 0,  len = 64)] price: i64,       // D64::to_raw()
    #[field(start = 64, len = 32)] size: u32,
    #[field(start = 96, len = 4)]  venue: Venue,
    #[field(start = 100, len = 24)] seq: u32,
    #[flag(127)]                   is_bid: bool,
}

pub struct Tick {
    pub price: D64,
    pub size: u32,
    pub venue: Venue,
    pub seq: u32,
    pub is_bid: bool,
}

impl Tick {
    pub fn pack(&self) -> TickRaw {
        TickRaw::builder()
            .price(self.price.to_raw())
            .size(self.size)
            .venue(self.venue)
            .seq(self.seq)
            .is_bid(self.is_bid)
            .build()
            .expect("fields pre-validated")
    }

    pub fn unpack(raw: TickRaw) -> Option<Self> {
        Some(Self {
            price: D64::from_raw(raw.price()),
            size: raw.size(),
            venue: raw.venue().ok()?,
            seq: raw.seq(),
            is_bid: raw.is_bid(),
        })
    }
}
```

`TickRaw` is `#[repr(transparent)]` on `u128`, which means:

- An array of ticks has zero header overhead.
- Memory-mapping a file of ticks is a transmute (with appropriate
  invariants).
- Ring-buffering is just `memcpy` of 16 bytes per slot.

Pairing `nexus-bits` with `nexus-decimal::Decimal::to_raw()` is
the canonical way to ship prices over the wire without floats.
See [nexus-decimal patterns](../../nexus-decimal/docs/patterns.md)
for the Decimal side.

## Wire protocol header (FIX-ish)

A 4-byte binary header for a custom exchange protocol: version,
message type, body length, flags.

```rust
use nexus_bits::{bit_storage, IntEnum};

#[derive(IntEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    Heartbeat = 0,
    Logon     = 1,
    NewOrder  = 2,
    OrderAck  = 3,
    Fill      = 4,
    Reject    = 5,
}

#[bit_storage(repr = u32)]
pub struct WireHeader {
    #[field(start = 0,  len = 4)]  version: u8,   // 0..=15
    #[field(start = 4,  len = 4)]  msg_type: MsgType,
    #[field(start = 8,  len = 16)] body_len: u16,
    #[flag(24)]                    compressed: bool,
    #[flag(25)]                    encrypted: bool,
    // bits 26..=31 reserved for future use — decoder ignores.
}

let h = WireHeader::builder()
    .version(1)
    .msg_type(MsgType::NewOrder)
    .body_len(128)
    .compressed(false)
    .encrypted(true)
    .build()
    .unwrap();

// Decode with a match.
match h.msg_type() {
    Ok(MsgType::NewOrder) if !h.compressed() => { /* fast path */ }
    Ok(_) => { /* slow path */ }
    Err(_) => { /* unknown msg type, drop */ }
}
```

Reserved bits are simply unclaimed bit positions. The decoder
ignores them because no accessor touches them; a future version
can claim them without breaking old readers as long as old
producers leave them zero.

## Bitset state tracking

A session state machine with per-feature enabled flags:

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct SessionState {
    #[flag(0)]  connected: bool,
    #[flag(1)]  authenticated: bool,
    #[flag(2)]  logged_on: bool,
    #[flag(3)]  market_data: bool,
    #[flag(4)]  orders_enabled: bool,
    #[flag(5)]  heartbeats_ok: bool,
    #[flag(6)]  under_throttle: bool,
    #[flag(7)]  draining: bool,
    // reserved for future use
    #[field(start = 32, len = 32)] seq: u32,
}

let state = SessionState::builder()
    .connected(true)
    .authenticated(true)
    .logged_on(true)
    .market_data(true)
    .orders_enabled(true)
    .heartbeats_ok(true)
    .under_throttle(false)
    .draining(false)
    .seq(0)
    .build()
    .unwrap();

assert!(state.connected() && state.logged_on());
```

This is cheaper to share across threads than a struct of
`AtomicBool`s — load the whole `u64` atomically and query the
bits locally. (You'd wrap `SessionState` in an `AtomicU64` for
real concurrency.)
