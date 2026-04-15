# `BitField<T>` and `Flag<T>` — runtime primitives

The derive macro is the right tool when you know the layout at
compile time. When you don't — for example, when the layout comes
from a configuration file, a protocol negotiation, or you just
want manual control — use the runtime primitives directly.

## `BitField<T>`

A `BitField<T>` describes a bit range inside a storage integer of
type `T`. `T` can be any of `u8`..`u128`, `i8`..`i128`.

```rust
use nexus_bits::BitField;

// Fields can be const
const KIND:     BitField<u64> = BitField::<u64>::new(0,  4);
const EXCHANGE: BitField<u64> = BitField::<u64>::new(4,  8);
const SYMBOL:   BitField<u64> = BitField::<u64>::new(12, 20);
```

`BitField::new(start, len)` panics at compile time (`const fn` +
`assert!`) if:

- `len == 0`
- `start + len > T::BITS`

On a correctly-placed field, `new` precomputes the mask so that
every subsequent operation is a handful of bit ops.

## API

```rust
use nexus_bits::BitField;

const F: BitField<u64> = BitField::<u64>::new(4, 8);

// Inspect the layout.
assert_eq!(F.start(), 4);
assert_eq!(F.len(), 8);
assert_eq!(F.mask(), 0xFF << 4);
assert_eq!(F.max_value(), 0xFF);

// Pack — returns Err on overflow.
let packed = F.set(0u64, 42).unwrap();

// Read.
assert_eq!(F.get(packed), 42);

// Overwrite — clears the field first, then sets.
let updated = F.set(packed, 7).unwrap();
assert_eq!(F.get(updated), 7);

// Overflow — Err with the offending value and the max.
let err = F.set(0u64, 256).unwrap_err();
assert_eq!(err.value, 256);
assert_eq!(err.max, 0xFF);

// set_unchecked: no bounds check, truncates silently.
let truncated = F.set_unchecked(0u64, 0x1FF);
assert_eq!(F.get(truncated), 0xFF); // 0x1FF & 0xFF

// Clear — zeros the field.
assert_eq!(F.clear(packed), 0);
```

Every method is `const fn`, which means you can compose bit
operations in `const` contexts:

```rust
use nexus_bits::BitField;

const KIND:   BitField<u64> = BitField::<u64>::new(0, 4);
const FLAGS:  BitField<u64> = BitField::<u64>::new(4, 28);

// Build a constant packed value at compile time.
const fn make(kind: u64, flags: u64) -> u64 {
    match KIND.set(0, kind) {
        Ok(v) => match FLAGS.set(v, flags) {
            Ok(w) => w,
            Err(_) => panic!("flags overflow"),
        },
        Err(_) => panic!("kind overflow"),
    }
}

const DEFAULT: u64 = make(3, 0xDEAD);
```

## Masking guarantees

- `set` validates: value must satisfy `value <= max_value()`.
  On signed storage types, the comparison is signed — see
  [signed-fields.md](signed-fields.md) for what that means in
  practice.
- `set_unchecked` **truncates** to the field width. Values larger
  than `max_value()` are silently ANDed into range. Use this only
  when you have already validated the value or when truncation is
  the desired semantics (wrapping counters, packed hashes).
- `set` never touches bits outside the field — the other bits in
  the storage integer are preserved.
- `clear` zeros only the field, preserving all other bits.

## `Flag<T>`

A single-bit version of `BitField`. Simpler API because there is
no overflow check and the type is always `bool`.

```rust
use nexus_bits::Flag;

const IS_BUY:  Flag<u64> = Flag::<u64>::new(0);
const IS_IOC:  Flag<u64> = Flag::<u64>::new(1);

let mut flags: u64 = 0;
flags = IS_BUY.set(flags);
flags = IS_IOC.set(flags);

assert!(IS_BUY.is_set(flags));
assert!(IS_IOC.is_set(flags));

// Toggle flips.
flags = IS_IOC.toggle(flags);
assert!(!IS_IOC.is_set(flags));

// set_to takes a bool and does the right thing.
flags = IS_IOC.set_to(flags, true);
assert!(IS_IOC.is_set(flags));

// Clear zeros the bit.
flags = IS_BUY.clear(flags);
assert!(!IS_BUY.is_set(flags));
```

## When to use runtime vs derive

Use the **derive macro** when:

- The layout is fixed at compile time.
- You want typed accessors and a checked builder.
- You want the compiler to enforce "all fields set before build".

Use **runtime `BitField<T>`** when:

- The layout comes from outside (config file, protocol version).
- You are implementing the derive macro's logic yourself for a
  custom DSL.
- You want the smallest possible API surface and manual control.
- You are writing generic code over the storage type.

You can also mix: use `BitField<T>` constants to manipulate a raw
integer before wrapping it with `FromRaw` of a derived newtype.
