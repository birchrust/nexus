# The `#[bit_storage]` derive macro

`#[bit_storage]` is an attribute macro that turns a struct (or an
enum — see [enum-variants.md](enum-variants.md)) into a packed,
typed, checked newtype. This page documents the full syntax for
structs.

Enable the `derive` feature (on by default):

```toml
[dependencies]
nexus-bits = { version = "0.3", features = ["derive"] }
```

## Basic form

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct Packed {
    #[field(start = 0, len = 8)]   a: u8,
    #[field(start = 8, len = 16)]  b: u16,
    #[field(start = 24, len = 32)] c: u32,
}
```

The macro generates:

- A `#[repr(transparent)]` newtype: `pub struct Packed(pub u64);`
- `Packed::from_raw(raw: u64) -> Self`
- `Packed::raw(self) -> u64`
- A typed accessor per field (`a() -> u8`, `b() -> u16`, `c() -> u32`)
- A `PackedBuilder` struct and `Packed::builder() -> PackedBuilder`
- `PackedBuilder::build() -> Result<Packed, FieldOverflow<u64>>`
- `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash` derives

## Attribute reference

### `#[bit_storage(repr = T)]`

Required. `T` is the storage integer type — one of `u8`, `u16`,
`u32`, `u64`, `u128`, `i8`, `i16`, `i32`, `i64`, `i128`.

### `#[bit_storage(repr = T, discriminant(start = N, len = M))]`

Enums only. See [enum-variants.md](enum-variants.md).

### `#[field(start = N, len = M)]`

Required on every non-flag struct field. `N` is the starting bit
(LSB = 0). `M` is the field width in bits (must be `> 0`). The
field must fit in the storage type: `N + M <= repr_bits`.

The field's Rust type determines how the value is interpreted:

- **Unsigned primitive**: zero-extended on read.
- **Signed primitive**: sign-extended on read if `M < type_bits`.
- **IntEnum type**: discriminant packed, accessor returns
  `Result<Enum, UnknownDiscriminant<_>>`.

### `#[flag(N)]`

Shortcut for a 1-bit field storing a `bool`. `N` is the bit
position. The accessor returns `bool` directly, the builder takes
a `bool`, and there is no overflow check.

## Generated API — struct

For a struct named `Foo` with storage `u64`:

```rust,ignore
impl Foo {
    pub const fn from_raw(raw: u64) -> Self;
    pub const fn raw(self) -> u64;
    pub const fn builder() -> FooBuilder;
    // One accessor per field, return type matches the field type
    pub const fn field_name(&self) -> FieldType;
    // For IntEnum fields:
    pub fn enum_field(&self) -> Result<EnumType, UnknownDiscriminant<u64>>;
}
```

Builder API:

```rust,ignore
impl FooBuilder {
    pub fn field_name(mut self, value: FieldType) -> Self;
    pub fn build(self) -> Result<Foo, FieldOverflow<u64>>;
}
```

Every call to a setter on the builder is required before `build()`;
missing fields are a compile-time error (the builder is
option-typed internally and `build()` checks presence).

## Example: basic fields

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct BasicFields {
    #[field(start = 0,  len = 8)]  a: u8,
    #[field(start = 8,  len = 16)] b: u16,
    #[field(start = 24, len = 32)] c: u32,
}

let packed = BasicFields::builder()
    .a(1)
    .b(2)
    .c(3)
    .build()
    .unwrap();

assert_eq!(packed.raw(), 1 | (2 << 8) | (3 << 24));
assert_eq!(packed.a(), 1);
assert_eq!(packed.b(), 2);
assert_eq!(packed.c(), 3);
```

## Example: narrow fields with overflow check

The Rust type is wider than the bit field. The builder accepts the
type's full range, but rejects values that don't fit in the field:

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct Narrow {
    #[field(start = 0, len = 4)] narrow: u8, // u8 type, 4-bit field (max 15)
}

// In range — ok.
assert!(Narrow::builder().narrow(15).build().is_ok());

// Out of range — FieldOverflow.
let err = Narrow::builder().narrow(16).build().unwrap_err();
assert_eq!(err.field, "narrow");
```

See [builder-pattern.md](builder-pattern.md) for full error
handling.

## Example: flags

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct Flags {
    #[flag(0)]  a: bool,
    #[flag(1)]  b: bool,
    #[flag(63)] high: bool,
}

let f = Flags::builder()
    .a(true)
    .b(false)
    .high(true)
    .build()
    .unwrap();

assert!(f.a());
assert!(!f.b());
assert!(f.high());
assert_eq!(f.raw(), 1 | (1u64 << 63));
```

## Example: IntEnum fields

An `IntEnum` field packs the variant's discriminant into the bit
range. Reading returns `Result` because the raw bits might hold an
unknown discriminant (for example, from a wire message produced by
a newer version of the protocol):

```rust
use nexus_bits::{bit_storage, IntEnum};

#[derive(IntEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Exchange { Nasdaq = 0, Nyse = 1, Cboe = 2 }

#[bit_storage(repr = u64)]
pub struct SymbolId {
    #[field(start = 0, len = 4)]  exchange: Exchange,
    #[field(start = 4, len = 24)] symbol_idx: u32,
}

let id = SymbolId::builder()
    .exchange(Exchange::Nyse)
    .symbol_idx(12345)
    .build()
    .unwrap();

assert_eq!(id.exchange().unwrap(), Exchange::Nyse);
assert_eq!(id.symbol_idx(), 12345);

// If a wire message arrives with a discriminant that isn't in
// our enum, the accessor returns UnknownDiscriminant:
let raw = SymbolId::from_raw(3); // exchange = 3, not a known variant
assert!(raw.exchange().is_err());
```

See [enum-variants.md](enum-variants.md) for the IntEnum derive
syntax.

## Validation — compile-time errors

The macro checks:

- `start + len <= repr_bits` — field must fit in storage.
- No two fields overlap (simple O(n²) sweep).
- `len > 0`.
- Field types are primitives or `IntEnum` implementors.
- Tuple structs are rejected (named fields only).
- Unions are rejected.

Violations produce standard `syn::Error` compile failures with
spans pointing at the offending field.

## What is NOT generated

- `Default` — the library does not assume `0` is a valid state.
  If you want it, add `impl Default for Foo { ... }` yourself.
- `Display` — wire formats are binary, so there is no canonical
  text form. Write your own.
- Serde impls — there is no single correct serialization. Use
  `raw()` / `from_raw()` and wrap them how you like.
- `From<T>` / `Into<T>` — `raw()` / `from_raw()` are explicit on
  purpose.

See [runtime-bitfield.md](runtime-bitfield.md) for the non-macro
primitives.
