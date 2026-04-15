# Caveats and failure modes

## Field width limits

A field's `len` must satisfy `start + len <= repr_bits`. Any
violation is a compile-time error from the derive macro (or a
`const fn` panic in `BitField::new`).

The individual field's type can be wider than the bit range —
`u32` fits into a 4-bit field in the layout, but the valid value
range is 0..=15. The builder rejects out-of-range values at
runtime.

## Alignment and layout

`#[bit_storage]` generates `#[repr(transparent)]` newtypes over a
primitive integer. Alignment and size are **exactly** those of
the storage type:

| Repr    | Size | Align |
|---------|------|-------|
| `u8`    | 1    | 1     |
| `u16`   | 2    | 2     |
| `u32`   | 4    | 4     |
| `u64`   | 8    | 8     |
| `u128`  | 16   | 16*   |

\* `u128` alignment is platform-dependent; most x86-64 systems
align to 16 bytes, but Rust does not guarantee it across targets.
Use `#[repr(C, align(16))]` wrappers for wire formats that need
cross-target layout.

Since the type is `repr(transparent)`, you can freely `transmute`
between the newtype and its storage integer. FFI, memory-mapped
files, and ring buffers all "just work".

## Endianness

The bit layout is specified **at the LSB = bit 0** level. When
you serialize the storage integer to bytes (e.g., with
`to_le_bytes()` or `to_be_bytes()`), the on-wire byte order
depends on the serialization you choose, not on `nexus-bits`.

For network protocols: pick one endianness (usually
little-endian for x86-native, big-endian for network byte order)
and convert explicitly at the encode/decode boundary.

## Overflow semantics

Runtime `BitField::set`:

- Unsigned: rejects values exceeding `max_value()`.
- Signed storage: the check is signed — a negative value
  mathematically is less than any positive `max_value`, so it
  passes. This is why the derive macro generates an additional
  range check for narrow signed fields. See
  [signed-fields.md](signed-fields.md).

`BitField::set_unchecked` **silently truncates** to the field
width. Never use this with untrusted input.

Derive macro builders: reject out-of-range values at `build()`
and return `FieldOverflow` with the field name. Missing fields
are a runtime error, not a compile-time one — the Rust type
system cannot express "builder fluent chain must call every
setter".

## No overlap checking across enum variants

Within a single `#[bit_storage]` struct or variant, field
overlap is a compile error. Across variants of the same enum,
overlap is allowed (and normal — different variants reuse the
same bits for different payloads).

The macro does not cross-check a variant field's range against
the enum's discriminant range. If you define
`discriminant(start = 0, len = 4)` and then put a variant field
at `start = 2, len = 8`, your variant payload will stomp the
discriminant bits and decode will be nonsensical. This is caught
by the generic overlap check within the variant (the variant's
payload overlaps the parent's discriminant field, which is
reported) but you should double-check the layout by hand for
novel formats.

## Const limitations

- Accessors are `const fn`. Reading a field from a const is fine.
- `from_raw` and `raw` are `const fn`.
- Builders are **not** `const fn` — they use `Option<T>`
  internally and the `Option::unwrap_or_else` path isn't
  const-stable in all forms.
- For compile-time construction, hand-pack the raw integer and
  call `from_raw`.

## IntEnum discriminant holes

`IntEnum` derives `try_from_repr` as an exhaustive match on
explicitly declared variant values. Holes (sparse variant
values) are allowed — unknown discriminants return `None`.

Beware: reading a field of an `IntEnum` type from wire data
produced by a newer version of your protocol is the common case
for `UnknownDiscriminant` errors. This is not a bug; it's the
defined forward-compatibility behavior. Log and drop the
message (or upgrade the reader).

## When NOT to use bit packing

Bit packing is the wrong answer when:

- **Fields change size at runtime.** Use tagged unions or
  length-prefixed messages.
- **Payloads include strings, arrays, or variable-length data.**
  Use a real serialization format: postcard, bincode, protobuf,
  flatbuffers.
- **You need to cross languages.** Bit packing is a Rust-native
  layout; interoperating with C, Go, or Java means each side
  maintains its own encoder. Feasible, but friction — consider
  if a protobuf schema would save more engineering hours.
- **Profile shows no benefit.** If your code is I/O bound or
  cache-cold on larger structures anyway, replacing a `struct`
  with a packed `u64` won't help and will make the code harder
  to read. Measure first.

## Debugging packed values

The derived `Debug` prints the raw integer (because that's what
the newtype is). You see `Order(0xC8D7E0B100000000)` rather than
the unpacked fields. For human-readable dumps, write a short
`Display` impl that calls the typed accessors:

```rust,ignore
use nexus_bits::bit_storage;
use core::fmt;

#[bit_storage(repr = u64)]
pub struct Order {
    #[field(start = 0, len = 16)] id: u16,
    #[field(start = 16, len = 32)] qty: u32,
}

impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Order {{ id: {}, qty: {} }}", self.id(), self.qty())
    }
}
```

Worth the 6 lines.

## Performance: branch-predictability of overflow checks

`BitField::set` has a single branch: `if value > max`. In
production code, this branch is either never taken (valid
inputs) or always taken (invalid inputs from a specific broken
producer). Either extreme is perfectly predicted. The cost of
the check on the happy path is one compare and one never-taken
branch — on x86-64 that's ~1 cycle amortized over a batch.

Use `set_unchecked` only when profiling shows the check is
actually on the critical path, **and** the value is guaranteed
to fit by construction.
