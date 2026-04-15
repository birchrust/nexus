# Tagged enums with `#[bit_storage]`

`#[bit_storage]` supports two kinds of enums:

1. **`#[derive(IntEnum)]` enums** — plain C-style enums that pack
   as a discriminant into a struct field. See [overview.md](overview.md)
   and [derive-macro.md](derive-macro.md).
2. **`#[bit_storage(repr = T, discriminant(...))]` enums** — full
   tagged unions packed into a single integer, with per-variant
   fields and a generated builder per variant. This page is about
   the second kind.

## `IntEnum` — the simple case

```rust
use nexus_bits::IntEnum;

#[derive(IntEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Exchange {
    Nasdaq = 0,
    Nyse   = 1,
    Cboe   = 2,
}

let e = Exchange::Nyse;
assert_eq!(e.into_repr(), 1);
assert_eq!(Exchange::try_from_repr(1), Some(Exchange::Nyse));
assert_eq!(Exchange::try_from_repr(99), None);
```

Requirements:

- Must have a `#[repr(u8 | u16 | u32 | u64 | u128 | i8..i128)]`.
- Variants must be unit (no fields).
- Variant values can be sparse (`A = 0, B = 2, C = 5`).

Use `IntEnum` types as **field types** inside a `#[bit_storage]`
struct. The discriminant is packed; the accessor returns
`Result<Enum, UnknownDiscriminant>`.

## Tagged-union enums

When a message has variant-specific payload, you want a full
tagged union. The `bit_storage` macro supports this directly:

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64, discriminant(start = 0, len = 4))]
pub enum Message {
    #[variant(0)]
    Empty,

    #[variant(1)]
    Short {
        #[field(start = 4, len = 8)]
        value: u8,
    },

    #[variant(2)]
    Long {
        #[field(start = 4, len = 16)]
        value: u16,
    },
}
```

The macro generates:

- A "parent" newtype: `Message` — wraps the raw `u64`, represents
  *any* variant, possibly with an unknown discriminant.
- A companion `MessageKind` unit-variant enum for matching.
- One "child" newtype per variant: `MessageEmpty`, `MessageShort`,
  `MessageLong`.
- One builder per variant, launched via `Message::empty()`,
  `Message::short()`, `Message::long()`.
- `as_parent()` on each child newtype.
- `kind() -> Result<MessageKind, UnknownDiscriminant<u64>>` on the
  parent.
- `is_empty()`, `is_short()`, `is_long()` boolean queries on the
  parent.
- `as_empty()`, `as_short()`, `as_long()` extraction methods on
  the parent, each returning `Result<VariantNewtype, UnknownDiscriminant<u64>>`.

## Building a variant

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64, discriminant(start = 0, len = 4))]
pub enum Message {
    #[variant(0)]
    Empty,
    #[variant(1)]
    Short {
        #[field(start = 4, len = 8)]
        value: u8,
    },
    #[variant(2)]
    Long {
        #[field(start = 4, len = 16)]
        value: u16,
    },
}

// Build an Empty — no fields to set.
let empty = Message::empty().build().unwrap();
assert_eq!(empty.raw(), 0);

// Build a Short — single u8 payload.
let short = Message::short().value(42).build().unwrap();
// discriminant=1 at bits 0-3, value=42 at bits 4-11
assert_eq!(short.raw(), 1 | (42 << 4));
assert_eq!(short.value(), 42);

// Build a Long — single u16 payload.
let long = Message::long().value(1000).build().unwrap();
assert_eq!(long.raw(), 2 | (1000 << 4));
```

## Reading a variant from the wire

```rust,ignore
let raw: u64 = receive();
let msg = Message::from_raw(raw);

// Dispatch via kind().
match msg.kind() {
    Ok(MessageKind::Empty) => handle_empty(),
    Ok(MessageKind::Short) => {
        let short = msg.as_short().unwrap();
        handle_short(short.value());
    }
    Ok(MessageKind::Long) => {
        let long = msg.as_long().unwrap();
        handle_long(long.value());
    }
    Err(e) => {
        // Unknown discriminant — upstream produced a variant
        // we don't know about. Log and drop, or drain.
        tracing::warn!(disc = e.value, "unknown message variant");
    }
}
```

Note that the `as_*` methods return `Result`: if the caller
misidentifies the variant (calls `as_short` on a `Long` message),
the return is `Err(UnknownDiscriminant)` with the actual
discriminant the payload had. That makes round-tripping and
replay-debugging straightforward.

## Multiple fields per variant

Each variant can have any number of `#[field]` and `#[flag]`
attributes, subject to the usual layout validation (no overlap,
within storage bounds):

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64, discriminant(start = 0, len = 4))]
pub enum Packet {
    #[variant(0)]
    Heartbeat {
        #[field(start = 4, len = 32)]
        seq: u32,
    },
    #[variant(1)]
    Fill {
        #[field(start = 4, len = 16)]
        symbol_id: u16,
        #[field(start = 20, len = 32)]
        qty: u32,
        #[flag(60)]
        is_buy: bool,
    },
}
```

## Field overlap across variants

Different variants can reuse the same bit range. In the `Message`
example above, `Short::value` (bits 4–11) and `Long::value` (bits
4–19) overlap — which is fine, because only one variant is
"active" at a time as identified by the discriminant. The macro
validates overlap within a variant but not across variants.

## Unknown discriminant handling

The discriminant field has its own width (`len` on the
`discriminant(...)` attribute). If the packed value's discriminant
doesn't match any defined `#[variant(N)]`, the parent's
`kind()` returns `UnknownDiscriminant`. The `is_*` queries return
`false`. The `as_*` extractors return `Err`.

This is how you safely handle wire messages from a newer protocol
version: old code silently ignores unrecognized variants instead
of crashing or misinterpreting bits.

## Constraints

- The discriminant bit range must be specified once in the
  `#[bit_storage]` attribute.
- Every variant needs a `#[variant(N)]` attribute with a unique
  integer discriminant.
- Variant discriminants must fit in the discriminant field
  (`N < 2^disc_len`).
- The storage repr applies to the whole enum — all variants pack
  into the same integer type.
