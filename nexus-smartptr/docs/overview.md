# Overview

Rust's `Box<dyn Trait>` is the idiomatic way to store a heterogeneous
collection of trait objects. It's also an allocation per element. For
systems where allocations are forbidden on the hot path — and especially
for collections of *small* trait objects — that cost is worth eliminating.

`nexus-smartptr` provides two smart pointers that store values inline in
a fixed-size buffer, with the buffer size encoded in the type:

- **`Flat<T, B>`** — inline only. If the concrete value doesn't fit, you
  get a compile-time or construction-time panic.
- **`Flex<T, B>`** — inline with heap fallback. If it fits, it lives
  inline. If it doesn't, it spills to the heap.

Both are `?Sized`-compatible: `Flat<dyn Trait, B32>` is a 32-byte inline
container for any trait object whose concrete type fits.

## Why not `SmallBox`?

`smallbox` (the crate this design borrows from) parameterizes by an
arbitrary const-generic capacity. `nexus-smartptr` takes a slightly
different approach: capacity is a *type* (`B16`, `B32`, `B64`, `B128`,
...), not a number. The tradeoff:

- **Pro:** `Flat<dyn Trait, B32>` is a distinct type, same across the
  codebase. Two `Flat<dyn Trait, B32>` always have the same size.
- **Pro:** New capacities are added via `define_buffer!` with compile-time
  alignment validation.
- **Con:** You can't parameterize *over* the capacity as easily as
  `SmallBox<T, const N: usize>`.

For the workloads this crate targets — fixed setups where you pick a
capacity at design time and stick with it — the type-based approach is
cleaner.

## Sizing

Capacities for 64-bit targets (pointer size = 8 bytes):

| Marker | Total size | `?Sized` value capacity | `Sized` capacity |
|--------|-----------|------------------------|-----------------|
| `B16`  | 16 bytes  | 8 bytes   | 16 bytes |
| `B32`  | 32 bytes  | 24 bytes  | 32 bytes |
| `B64`  | 64 bytes  | 56 bytes  | 64 bytes |
| `B128` | 128 bytes | 120 bytes | 128 bytes |
| `B256` | 256 bytes | 248 bytes | 256 bytes |

For `?Sized` storage, subtract one pointer-sized word for the metadata
(vtable pointer for `dyn Trait`, length for `[T]`). For `Sized` storage,
the full capacity is available. `Flex` reserves an additional word for
the inline/heap discriminant, so subtract another pointer's worth.

## Constructing

For `Sized` values:

```rust
use nexus_smartptr::{Flat, Flex, B32};

let f: Flat<u64, B32> = Flat::new(42);
let x: Flex<u64, B32> = Flex::new(42);
```

For `?Sized` values (trait objects), use the macros — they perform the
unsizing coercion the compiler needs:

```rust
use nexus_smartptr::{flat, flex, Flat, Flex, B32};

trait Greet {
    fn greet(&self) -> &'static str;
}

struct Hello;
impl Greet for Hello {
    fn greet(&self) -> &'static str { "hello" }
}

let f: Flat<dyn Greet, B32> = flat!(Hello);
assert_eq!(f.greet(), "hello");

let x: Flex<dyn Greet, B32> = flex!(Hello);
assert!(x.is_inline());
```

The return-type annotation is *required* — it tells the compiler to
coerce the concrete type to the `?Sized` target during macro expansion.

## When to use this

- Heterogeneous collections of small trait objects (handlers,
  middleware, effects).
- `dyn Trait` values stored in slabs or pools where boxing would add
  allocation noise.
- No-alloc contexts where `Box` isn't available.

## When *not* to use this

- Everything fits in one type. Use an enum.
- Objects are large and variable in size. Use `Box<dyn Trait>`.
- You need inline storage but not dyn dispatch. Use generics.
- You need weak references, shared ownership, or reference counting.
  Use `Rc`/`Arc`.
