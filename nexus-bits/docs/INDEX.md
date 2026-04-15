# nexus-bits Documentation

Bit-packed integer newtypes via derive macros. Zero-cost building
blocks for wire protocols, packed IDs, and compact state.

## Reading order

1. [overview.md](overview.md) — Why bit packing, performance model
2. [derive-macro.md](derive-macro.md) — `#[bit_storage]` full syntax
3. [runtime-bitfield.md](runtime-bitfield.md) — `BitField<T>` for runtime layouts
4. [signed-fields.md](signed-fields.md) — Signed field packing + sign extension
5. [builder-pattern.md](builder-pattern.md) — Generated builders, `FieldOverflow`
6. [enum-variants.md](enum-variants.md) — Tagged enums via `bit_storage`
7. [patterns.md](patterns.md) — Snowflake IDs, order flags, wire headers
8. [caveats.md](caveats.md) — Limits, alignment, when not to use this

## Quick start

```rust
use nexus_bits::{bit_storage, IntEnum};

#[derive(IntEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Side { Buy = 0, Sell = 1 }

#[bit_storage(repr = u64)]
pub struct OrderFlags {
    #[field(start = 0, len = 1)]  side: Side,
    #[field(start = 1, len = 2)]  tif: u8,
    #[field(start = 3, len = 32)] qty: u32,
    #[flag(63)]                   post_only: bool,
}

let o = OrderFlags::builder()
    .side(Side::Buy)
    .tif(2)
    .qty(100)
    .post_only(true)
    .build()
    .unwrap();
assert_eq!(o.qty(), 100);
```

## Related crates

- [nexus-decimal](../../nexus-decimal/docs/INDEX.md) — pack a
  `Decimal`'s `to_raw()` into a `bit_storage` field for zero-copy
  wire formats.
