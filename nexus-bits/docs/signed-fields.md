# Signed field packing

A `#[field]` can have a signed Rust type (`i8`, `i16`, `i32`,
`i64`, `i128`). The derive macro handles sign-extension on read so
a negative value written with the builder comes back intact.

## How it works

For an N-bit field with a signed type that is wider than N, the
macro generates:

```text
read:  raw = (storage >> start) & mask           // N bits, zero-extended
       signed = (raw << (type_bits - N)) >> (type_bits - N)  // arithmetic shift
```

The left/right shift pair sign-extends the top bit of the N-bit
value across the remaining `type_bits - N` bits of the target
type. If the field is the same width as the type, no extension is
needed.

The write path stores the two's complement bit pattern, truncated
to the field width.

## Full-width signed fields

When the field width matches the signed type, no extension
happens — the stored pattern and the value are identical:

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u32)]
pub struct SignedFields {
    #[field(start = 0, len = 8)]  signed_byte: i8,   // full i8
    #[field(start = 8, len = 16)] signed_short: i16, // full i16
}

let s = SignedFields::builder()
    .signed_byte(-1)
    .signed_short(-1)
    .build()
    .unwrap();

assert_eq!(s.signed_byte(), -1);
assert_eq!(s.signed_short(), -1);

// Min values round-trip.
let m = SignedFields::builder()
    .signed_byte(i8::MIN)
    .signed_short(i16::MIN)
    .build()
    .unwrap();
assert_eq!(m.signed_byte(), i8::MIN);
assert_eq!(m.signed_short(), i16::MIN);
```

## Narrow signed fields

When the field width is narrower than the type, the valid range
shrinks. A 4-bit signed field can hold `-8..=7`. A 12-bit signed
field holds `-2048..=2047`.

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u32)]
pub struct NarrowSigned {
    #[field(start = 0, len = 4)]  narrow_i8: i8,   // -8..=7
    #[field(start = 4, len = 12)] narrow_i16: i16, // -2048..=2047
}

let s = NarrowSigned::builder()
    .narrow_i8(-8)
    .narrow_i16(-2048)
    .build()
    .unwrap();

assert_eq!(s.narrow_i8(), -8);
assert_eq!(s.narrow_i16(), -2048);

// All 16 values of a 4-bit signed field round-trip cleanly.
for v in -8i8..=7 {
    let s = NarrowSigned::builder()
        .narrow_i8(v)
        .narrow_i16(0)
        .build()
        .unwrap();
    assert_eq!(s.narrow_i8(), v);
}
```

## Overflow behavior

The builder for narrow signed fields accepts the full type range
and rejects values outside the field's representable range. The
rejection happens at `build()` time and returns a
`FieldOverflow<StorageType>` with the field name.

However — and this is important — the underlying `BitField::set`
treats signed storage as a simple signed integer compare. If you
are using the **runtime** `BitField<i64>` API directly, a negative
value will pass the `value <= max_value()` check (because negative
< positive) and then be truncated on `set_unchecked`. This is
documented on the `BitField::set` doc comment.

For range-checked signed field packing (rejecting values outside
the signed N-bit range), use the derive macro's builder. The
macro generates the extra range validation the runtime API
doesn't perform.

## Sign extension in detail

Given a 4-bit signed field storing the value `-1`:

```text
  two's complement in 4 bits:  0b1111
  stored at bit position 0 in u32: 0x0000000F
```

On read:

```text
  masked:    0x0000000F
  cast i8:   0x0F = 15
  << 4:      0xF0
  >>a 4:     0xFF = -1  (arithmetic shift preserves the sign bit)
```

The sign bit (bit 3 of the 4-bit field) is propagated upward so
the final `i8` is the correct negative value.

## When to use signed fields

- **Delta encodings**: price changes, sequence gaps, signed offsets
  from a reference.
- **Signed identifiers**: rare, but some protocols use signed IDs
  for "tombstone" entries.
- **Packed P&L**: as long as the magnitude fits, pack net positions
  or realized P&L into a narrow signed field.

Prefer unsigned storage whenever possible. Signed packing adds a
shift-and-shift on read and is slightly harder to reason about.
Use it when the domain value is inherently signed.
