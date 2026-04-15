# `Flex<T, B>` — inline with heap fallback

`Flex<T, B>` tries to store its value inline in the buffer `B`. If the
concrete value is too big, it falls back to heap allocation transparently.
Same API as [`Flat`](flat.md), but never panics on construction.

## Construction

```rust
use nexus_smartptr::{flex, Flex, B32};
use core::fmt::Display;

// Fits inline — stays inline.
let f: Flex<dyn Display, B32> = flex!(42_u32);
assert!(f.is_inline());

// Too big for B32 — spills to heap.
let big: Flex<dyn Display, B32> = flex!([0u64; 10]);  // 80 bytes
assert!(!big.is_inline());
```

For `Sized` types, use `Flex::new`:

```rust
use nexus_smartptr::{Flex, B32};

let x: Flex<u64, B32> = Flex::new(42);
assert_eq!(*x, 42);
```

## When to choose `Flex` over `Flat`

Use `Flex` when:

- You don't know at design time whether every concrete type will fit.
- You want the common case (small values) to be alloc-free and the
  uncommon case (large values) to "just work" via heap.
- You're building a public API and don't want panics.

Use `Flat` when:

- You *do* know every type fits and want to enforce it.
- You want the type to guarantee "this is exactly `B::CAPACITY` bytes,
  always" for layout reasons.
- You don't want any heap fallback — size pressure should be a visible
  error, not a silent allocation.

## `is_inline`

```rust
use nexus_smartptr::{flex, Flex, B32};
use core::fmt::Debug;

let small: Flex<dyn Debug, B32> = flex!(42_u8);
let large: Flex<dyn Debug, B32> = flex!([0u64; 10]);

assert!(small.is_inline());
assert!(!large.is_inline());
```

Use this to verify that your hot-path values really are staying inline.
It's the canonical way to catch a regression where someone made a type
bigger and silently pushed it onto the heap.

## Size overhead

`Flex<T, B>` is the same size as `Flat<T, B>` in memory — it uses the
same underlying buffer — but reserves one extra pointer-sized word for
the discriminant (inline-vs-heap flag and, when heap-allocated, the
heap pointer).

For `?Sized` targets, the usable inline capacity is therefore `CAPACITY
- 2 * sizeof(usize)` instead of `CAPACITY - sizeof(usize)`. Plan
accordingly: if you need every byte, use `Flat`.

## Drop

`Flex` drops correctly in both modes:

- **Inline:** calls drop on the inline bytes via the stored metadata
  (same as `Flat`).
- **Heap:** frees the heap allocation and drops the contained value.

No leaks, no double-frees, no mode-switch bugs.

## Usage as `dyn Trait`

Just like `Flat`, `Flex` derefs to the target type. Consumer code
doesn't need to know whether the value is inline or heap:

```rust
use nexus_smartptr::{flex, Flex, B32};

trait Handler {
    fn handle(&self, event: u64);
}

struct Small;
impl Handler for Small { fn handle(&self, _: u64) {} }

struct Large([u64; 20]);
impl Handler for Large { fn handle(&self, _: u64) {} }

let handlers: Vec<Flex<dyn Handler, B32>> = vec![
    flex!(Small),            // inline
    flex!(Large([0; 20])),   // heap fallback
];

for h in &handlers {
    h.handle(0);
}
```

## Type aliases

```rust
pub type Flex16<T>  = Flex<T, B16>;
pub type Flex32<T>  = Flex<T, B32>;
pub type Flex48<T>  = Flex<T, B48>;
pub type Flex64<T>  = Flex<T, B64>;
pub type Flex128<T> = Flex<T, B128>;
pub type Flex256<T> = Flex<T, B256>;
```
