# Patterns

## Exchange symbols

Exchange symbols are ASCII, short (≤ 16 bytes on virtually every venue),
compared constantly, and used as hash-map keys for quote caches and order
books.

```rust
use nexus_ascii::{AsciiString16, AsciiError};

pub type Symbol = AsciiString16;

pub fn parse_symbol(s: &str) -> Result<Symbol, AsciiError> {
    AsciiString16::try_from(s)
}
```

Keep one `Symbol` per instrument at setup time and pass it around by value
(`Copy`). Don't reparse from the wire every tick.

```rust
use nexus_ascii::{AsciiHashMap, AsciiString16};
pub type QuoteCache = AsciiHashMap<16, Quote>;
# #[derive(Clone)] struct Quote { bid: f64, ask: f64 }
```

## Protocol fields (FIX, wire protocols)

FIX tag values, SBE enums, and similar protocol-level strings are almost
always ≤ 8 bytes and never contain non-ASCII. `AsciiString8` is a direct
replacement for `String` or `&'static str` that's `Copy`, hashable, and
cheap to compare.

```rust
use nexus_ascii::AsciiString8;

#[derive(Copy, Clone)]
pub struct NewOrderSingle {
    pub sender_comp_id: AsciiString8,
    pub target_comp_id: AsciiString8,
    pub cl_ord_id: nexus_ascii::AsciiString32,
    // ...
}
```

The whole struct is `Copy` because every field is `Copy`. Pass it through
the stack without move semantics. Clone is free.

## Hot HashMap keys

Any workload where `HashMap::get` shows up in the flame graph is a candidate
for `AsciiHashMap`. The recipe:

1. Pick the smallest capacity that fits.
2. Enable the `nohash` feature.
3. Build keys once at ingress, reuse them.

```rust
use nexus_ascii::{AsciiHashMap, AsciiString32};

pub struct SessionTable {
    // Session ID is a Stripe-style token, max 32 chars
    sessions: AsciiHashMap<32, SessionState>,
}

impl SessionTable {
    pub fn lookup(&self, id: &AsciiString32) -> Option<&SessionState> {
        self.sessions.get(id)
    }
}
# struct SessionState;
```

## Static string interning

When you have a fixed set of strings known at startup (instrument classes,
venue names, order types), store them as `AsciiString*` and compare by
value rather than by string content.

```rust
use nexus_ascii::AsciiString8;

pub struct Venues {
    pub binance: AsciiString8,
    pub coinbase: AsciiString8,
    pub kraken: AsciiString8,
}

impl Venues {
    pub fn new() -> Self {
        Self {
            binance:  AsciiString8::try_from("BINANCE").unwrap(),
            coinbase: AsciiString8::try_from("COINBASE").unwrap(),
            kraken:   AsciiString8::try_from("KRAKEN").unwrap(),
        }
    }
}
```

## Pairing with `nexus-id`'s `TypeId`

`TypeId` stores its prefix as an `AsciiString`, so the prefix benefits from
the same fast equality and hashing. Route dispatch by prefix becomes a
single header compare:

```rust
use nexus_id::TypeId;
use nexus_ascii::AsciiString16;

fn route(id: TypeId<8>) {
    let (prefix, _) = id.into_parts();
    match prefix.as_str() {
        "ord"  => handle_order(),
        "fill" => handle_fill(),
        "pos"  => handle_position(),
        _      => log_unknown(prefix),
    }
}
# fn handle_order() {} fn handle_fill() {} fn handle_position() {}
# fn log_unknown(_: nexus_ascii::AsciiString<8>) {}
```
