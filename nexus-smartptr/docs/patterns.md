# Patterns

## Heterogeneous handler collection

The canonical use case: a `Vec` of things that implement the same trait
but have different concrete types, none of them large.

```rust
use nexus_smartptr::{flat, Flat, B32};

trait EventHandler {
    fn handle(&mut self, event: u64);
}

struct Logger { count: u64 }
impl EventHandler for Logger {
    fn handle(&mut self, _: u64) { self.count += 1; }
}

struct Metrics { last: u64 }
impl EventHandler for Metrics {
    fn handle(&mut self, ev: u64) { self.last = ev; }
}

let mut handlers: Vec<Flat<dyn EventHandler, B32>> = vec![
    flat!(Logger { count: 0 }),
    flat!(Metrics { last: 0 }),
];

for h in handlers.iter_mut() {
    h.handle(42);
}
```

Each entry is exactly 32 bytes, stored inline in the `Vec`. No
per-handler boxing, no allocation as handlers are added — just `Vec`
growth. For a handful of small handler types, this is noticeably
faster than `Vec<Box<dyn EventHandler>>` in cache-sensitive contexts.

## No-alloc trait object storage

For contexts where allocation is forbidden entirely (nested in a
no_std scope, or on a hot path where you want to preserve the
allocation budget), use `Flat` with an upper bound you can enforce at
construction:

```rust
use nexus_smartptr::{flat, Flat, B64};

trait Task {
    fn run(&mut self);
}

struct SmallTask;
impl Task for SmallTask { fn run(&mut self) {} }

// Compile-time guarantee: this slot is 64 bytes, no heap.
let mut slot: Flat<dyn Task, B64> = flat!(SmallTask);
slot.run();
```

If a new task type comes along that doesn't fit, you'll find out at
construction time — panic with a clear error — rather than silently
allocating. This is usually what you want in a trading system where
unexpected allocation is a bug.

## Opportunistic inline with `Flex`

When you have a mix of small and occasional large types, use `Flex` so
the common case is alloc-free while the uncommon case still works:

```rust
use nexus_smartptr::{flex, Flex, B64};

trait Pipeline {
    fn process(&mut self, buf: &mut [u8]);
}

struct PassThrough;
impl Pipeline for PassThrough { fn process(&mut self, _: &mut [u8]) {} }

struct BigFilter { state: [u64; 32] }  // 256 bytes — too big for B64
impl Pipeline for BigFilter { fn process(&mut self, _: &mut [u8]) {} }

let stages: Vec<Flex<dyn Pipeline, B64>> = vec![
    flex!(PassThrough),                          // inline
    flex!(BigFilter { state: [0; 32] }),        // heap
];

let inline_count = stages.iter().filter(|s| s.is_inline()).count();
assert_eq!(inline_count, 1);
```

Use `is_inline()` in tests or debug assertions to verify that the
hot-path stages are staying inline.

## Pairing with `nexus-rt` handler storage

`nexus-rt` exposes `FlatVirtual<E, B>` / `FlexVirtual<E, B>` (type
aliases over `Flat` / `Flex`) for its handler storage behind the
`smartptr` feature. The pattern there is:

```rust,ignore
// Simplified — see nexus-rt docs for the actual traits.
use nexus_smartptr::{flat, Flat, B64};

trait Handler<E> {
    fn run(&mut self, world: &mut World, event: E);
}

pub type InlineHandler<E> = Flat<dyn Handler<E>, B64>;

let h: InlineHandler<MarketEvent> = flat!(MyHandler::new());
# struct World; struct MarketEvent; struct MyHandler;
# impl MyHandler { fn new() -> Self { Self } }
# impl Handler<MarketEvent> for MyHandler {
#     fn run(&mut self, _: &mut World, _: MarketEvent) {}
# }
```

Pick a buffer size (`B32`, `B64`, ...) that comfortably fits your
handler types, and the entire runtime registry becomes one `Vec` with
no per-handler boxing.

## Checking sizes at compile time

If you want to enforce "this type fits in `B32`" as part of your API
contract, add a compile-time assertion:

```rust
use nexus_smartptr::{Flat, B32, Buffer};

pub struct MyType { pub x: u64, pub y: u64 }

// This const block fires at build time if MyType outgrows B32.
const _: () = {
    assert!(core::mem::size_of::<MyType>() <= B32::CAPACITY);
};
```

This turns the "concrete type too big" runtime panic into a compile
error, which is usually what you want for types under your control.
