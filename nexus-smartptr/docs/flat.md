# `Flat<T, B>` — fixed inline storage

`Flat<T, B>` stores a value of type `T` inline in a buffer of type `B`.
No heap allocation, no fallback. If the concrete value doesn't fit, you
get a panic at construction time.

## Construction

For `Sized` targets:

```rust
use nexus_smartptr::{Flat, B32};

let f: Flat<u64, B32> = Flat::new(42);
assert_eq!(*f, 42);
```

For `?Sized` targets (trait objects, slices), use the `flat!` macro:

```rust
use nexus_smartptr::{flat, Flat, B32};
use core::fmt::Display;

let f: Flat<dyn Display, B32> = flat!(42_u32);
assert_eq!(format!("{}", &*f), "42");
```

The return-type annotation triggers the compiler's unsizing coercion
inside the macro.

## Capacity and panics

The concrete value must fit in `B::CAPACITY` bytes (minus one pointer
word for `?Sized` metadata). If it doesn't:

```rust,ignore
use nexus_smartptr::{flat, Flat, B16};

trait Big {}
struct Giant([u8; 256]);  // way too big for B16
impl Big for Giant {}

// Panics: concrete type doesn't fit in B16's 8-byte ?Sized capacity.
let _: Flat<dyn Big, B16> = flat!(Giant([0; 256]));
```

This is why `Flat` is the right choice when you *know* all your types
fit and you want to enforce that invariant at runtime. If you want a
graceful fallback, use [`Flex`](flex.md).

## Usage

`Flat<T, B>` derefs to `T` — all trait-object methods are available
directly:

```rust
use nexus_smartptr::{flat, Flat, B32};

trait Shape {
    fn area(&self) -> f64;
}

struct Circle { r: f64 }
impl Shape for Circle { fn area(&self) -> f64 { 3.14 * self.r * self.r } }

let s: Flat<dyn Shape, B32> = flat!(Circle { r: 2.0 });
let a: f64 = s.area();  // deref to &dyn Shape, call method
# assert!((a - 12.56).abs() < 0.01);
```

## Size and layout

`size_of::<Flat<T, B>>() == B::CAPACITY`. This is the primary guarantee
`Flat` provides: the type *is* the size. Two `Flat<dyn Trait, B32>`
values are always 32 bytes, regardless of what they contain.

Storage inside the buffer:

```text
[ value bytes | metadata word ]
 <---- B::CAPACITY bytes ---->
```

For `?Sized` targets, the metadata word at the tail holds the vtable
pointer (for `dyn Trait`) or length (for `[T]`). For `Sized` targets,
the whole buffer is available for the value.

## Drop

`Flat<T, B>` drops its inner value correctly via the stored metadata.
For `dyn Trait`, this goes through the vtable's drop slot; for plain
types, it's a direct drop. No leaks, no double-frees.

## Send/Sync

`Flat<T, B>: Send` iff `T: Send`, and similarly for `Sync`. The buffer
`B` is always `Send + Sync` on its own, so auto-trait inheritance works
naturally.

## Type aliases

Pre-built aliases for common sizes:

```rust
pub type Flat16<T>  = Flat<T, B16>;
pub type Flat32<T>  = Flat<T, B32>;
pub type Flat48<T>  = Flat<T, B48>;
pub type Flat64<T>  = Flat<T, B64>;
pub type Flat128<T> = Flat<T, B128>;
pub type Flat256<T> = Flat<T, B256>;
```

Use them when you don't need the explicit buffer type in your signature:

```rust
use nexus_smartptr::{flat, Flat32};
use core::fmt::Debug;

let f: Flat32<dyn Debug> = flat!(42_u64);
```
