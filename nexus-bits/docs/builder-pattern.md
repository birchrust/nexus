# Builders and overflow errors

For each struct or enum variant with `#[bit_storage]`, the macro
generates a companion builder. The builder is a plain `Copy`
struct that holds an `Option<T>` per field. `build()` validates
that every field is present and every value fits, then returns
the packed newtype.

## Basic usage

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct Packed {
    #[field(start = 0, len = 8)]   a: u8,
    #[field(start = 8, len = 16)]  b: u16,
    #[field(start = 24, len = 32)] c: u32,
}

let p = Packed::builder()
    .a(1)
    .b(2)
    .c(3)
    .build()
    .unwrap();

assert_eq!(p.a(), 1);
```

Each setter takes `self` and returns `Self` so calls can be
chained. The generated builder derives `Debug`, `Clone`, `Copy`,
`Default`, so you can also build partially, store the partial
state, and finish later:

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct Packed {
    #[field(start = 0, len = 8)]   a: u8,
    #[field(start = 8, len = 16)]  b: u16,
    #[field(start = 24, len = 32)] c: u32,
}

let partial = Packed::builder().a(1).b(2);
// ... later ...
let done = partial.c(3).build().unwrap();
assert_eq!(done.a(), 1);
```

## Error type: `FieldOverflow`

```rust
use nexus_bits::{FieldOverflow, Overflow};

// Defined in the crate as:
// pub struct Overflow<T>       { pub value: T, pub max: T }
// pub struct FieldOverflow<T>  { pub field: &'static str, pub overflow: Overflow<T> }
```

`FieldOverflow` carries the field name (as a `&'static str`, so
logging is allocation-free) and the offending value plus the
field's maximum. `Display` produces messages like:

```text
field 'sequence': value 4096 exceeds max 4095
```

## Handling overflow

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct SnowflakeId {
    #[field(start = 0,  len = 12)] sequence: u16,  // 0..=4095
    #[field(start = 12, len = 10)] worker: u16,    // 0..=1023
    #[field(start = 22, len = 42)] timestamp: u64,
}

let result = SnowflakeId::builder()
    .sequence(4096) // over 12-bit max
    .worker(0)
    .timestamp(0)
    .build();

let err = result.unwrap_err();
assert_eq!(err.field, "sequence");
assert_eq!(err.overflow.value, 4096);
assert_eq!(err.overflow.max, 4095);
```

The error is `Copy`, so you can log it and keep processing:

```rust,ignore
match builder.build() {
    Ok(packed) => dispatch(packed),
    Err(e) => {
        tracing::warn!(
            field = e.field,
            value = %e.overflow.value,
            max = %e.overflow.max,
            "dropped message: field overflow"
        );
    }
}
```

Use it as a circuit breaker signal — if `sequence` overflows, your
upstream counter wrapped and you should alert, not silently
truncate.

## Missing-field handling

`build()` requires every field to have been set. If you forget
one:

```rust,ignore
let result = Packed::builder().a(1).b(2).build();
// result is Err(FieldOverflow { field: "c", ... })
```

The error reports the **first** unset field as a special "missing
field" variant (the macro uses `FieldOverflow` as a tagged
failure type; check the generated code for the exact discriminant
used for missing vs overflow). In practice, you either use all
setters or you don't — the Rust type system doesn't prevent
missing calls, but the runtime check catches them.

## Builder vs `from_raw`

Two ways to construct:

| Constructor   | When to use                                      |
|---------------|--------------------------------------------------|
| `builder()`   | Producing a packed value from typed inputs       |
| `from_raw(u)` | Decoding an already-packed integer from the wire |

`from_raw` performs no validation — it is a zero-cost cast. If
the raw bits hold a value that would fail the builder's checks
(e.g., a 4-bit enum discriminant of 15 when only 0–2 are valid),
the field accessor will return an error at read time.

This split is intentional. Packing is a write-time concern;
decoding untrusted bytes is a read-time concern. Separating them
lets the fast path stay fast.

## Const builders

Builder calls are not currently `const fn` (they use `Option`
internally which has `const` limitations). If you need to build a
packed value at compile time, use `from_raw` with hand-packed
bits:

```rust
use nexus_bits::bit_storage;

#[bit_storage(repr = u64)]
pub struct Packed {
    #[field(start = 0, len = 8)]  a: u8,
    #[field(start = 8, len = 16)] b: u16,
}

const DEFAULT: Packed = Packed::from_raw(0x0001_00FF); // a=255, b=1
```

This is a `const fn` because the newtype's `from_raw` is `const`.
The tradeoff is you lose overflow checking — you're responsible
for the raw layout.
