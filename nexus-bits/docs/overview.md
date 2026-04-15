# Overview

`nexus-bits` is a small crate for packing multiple values into a
single integer. It provides two layers:

1. **Runtime primitives** — `BitField<T>` and `Flag<T>`, which
   describe a bit range in a storage integer and provide
   get/set/clear operations. Useful when the layout is decided at
   runtime or when you want a bit of manual control.
2. **Derive macro** (`#[bit_storage]`) — generates a zero-cost
   `#[repr(transparent)]` newtype around a storage integer, plus a
   typed builder, typed accessors, and overflow-detecting setters.
   This is what you want for 95% of use cases.

Both layers are `no_std`, zero-allocation, and operate entirely on
primitive integers.

## When to reach for this

Bit packing is the right tool when:

- You have a **wire protocol** that fixes a binary layout — FIX
  binary encodings, ITCH/OUCH, custom exchange feeds, L2 bookdata.
- You need a **packed ID** — snowflake, instrument ID,
  timestamp-plus-sequence.
- You want a **compact state mask** — order flags, connection
  state, feature toggles.
- You want **cache-friendly** storage for large arrays of small
  records — packing 4–8 fields into one `u64` cuts cache footprint
  significantly.

It is the wrong tool when:

- Fields change size at runtime.
- You need anything that isn't byte-aligned integer packing (floats,
  strings, variable-length data — reach for a real serialization
  format).
- Your fields overlap more than one storage word.

## Performance model

A `#[bit_storage]` newtype compiles to the same code you would
write by hand with shifts and masks. There is no runtime
representation beyond the storage integer:

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct Packed {
    #[field(start = 0, len = 32)]  a: u32,
    #[field(start = 32, len = 32)] b: u32,
}

// sizeof(Packed) == sizeof(u64) == 8
assert_eq!(core::mem::size_of::<Packed>(), 8);
```

Accessors are `const fn`, `#[inline]`, and boil down to a shift-
and-mask. On x86-64 a typical accessor is 2–3 instructions.
Builders generate the `or` chain at compile time and the whole
`build()` reduces to constant-folded bitmath when all inputs are
constants.

The overflow-checked setter (`BitField::set`) is a compare, a
branch, and the same shift-and-or. The branch is predictable
(packed correctly, it never fires) and the compiler can often
eliminate the check when the input type is narrower than the field.

## Signed vs unsigned storage

You can use any of `u8`, `u16`, `u32`, `u64`, `u128`, `i8`, `i16`,
`i32`, `i64`, `i128` as the storage type. Most wire formats use
unsigned storage. Signed storage is supported for cases where the
packed value itself is signed (e.g., sequence deltas).

## Field types

A `#[field]` in a `#[bit_storage]` struct can be:

- **An unsigned primitive** (`u8`..`u128`): value is stored as-is,
  zero-extended on read.
- **A signed primitive** (`i8`..`i128`): value is stored in two's
  complement, sign-extended on read. See [signed-fields.md](signed-fields.md).
- **An `IntEnum`**: the enum's discriminant is packed; the
  accessor returns `Result<Enum, UnknownDiscriminant<_>>`.

A `#[flag]` is a single-bit boolean with a simpler API (no
overflow possible).

See [derive-macro.md](derive-macro.md) for the full syntax.
