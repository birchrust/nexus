# Macro Allocators

The macro allocators generate thread-local slab instances wrapped in
zero-sized allocator types. This is the primary API for most users.

## Four Macros

| Macro | Slab Type | Storage |
|-------|-----------|---------|
| `bounded_allocator!(T)` | Fixed capacity | Typed `SlotCell<T>` |
| `unbounded_allocator!(T)` | Growable chunks | Typed `SlotCell<T>` |
| `bounded_byte_allocator!(T)` | Fixed capacity | Byte-erased `AlignedBytes<N>` |
| `unbounded_byte_allocator!(T)` | Growable chunks | Byte-erased `AlignedBytes<N>` |

## Usage

```rust
// In a module (macro generates types inside it)
mod orders {
    nexus_slab::bounded_allocator!(Order);
}

fn main() {
    // Initialize — once per thread
    orders::Allocator::builder().capacity(1024).build().unwrap();

    // Allocate via BoxSlot (RAII)
    let order = orders::BoxSlot::try_new(Order::default()).unwrap();
    // order auto-frees on drop

    // Or allocate raw (manual free)
    let slot = orders::Allocator::try_alloc(Order::default()).unwrap();
    // ... use slot ...
    unsafe { orders::Allocator::free(slot); }
}
```

## What Gets Generated

The macro generates inside your module:

- **`Allocator`** — zero-sized unit struct implementing `Alloc` + `BoundedAlloc` (or `UnboundedAlloc`)
- **`Builder`** — configures capacity / chunk size
- **`BoxSlot`** — type alias for `nexus_slab::BoxSlot<T, Allocator>`
- **Thread-local slab** — the actual storage, hidden

## Builder Configuration

### Bounded
```rust
orders::Allocator::builder()
    .capacity(1024)     // required — total slot count
    .build()
    .unwrap();
```

### Unbounded
```rust
orders::Allocator::builder()
    .chunk_size(512)       // optional — slots per chunk (default: 256)
    .initial_chunks(4)     // optional — pre-allocate N chunks
    .build()
    .unwrap();
```

## Thread Safety

Macro-generated allocators are **thread-local**. Each thread gets its own
slab. The `Allocator` type is `!Sync` — you can't share it across threads.
`RawSlot` and `BoxSlot` are `!Send` — they must stay on the thread that
allocated them.

For cross-thread return, use `nexus-pool`'s `sync::Pool` pattern instead.

## The Alloc Trait

The generated `Allocator` implements the `Alloc` trait:

```rust
pub unsafe trait Alloc: Sized + 'static {
    type Item;
    fn is_initialized() -> bool;
    fn capacity() -> usize;
    unsafe fn free(slot: RawSlot<Self::Item>);
    unsafe fn take(slot: RawSlot<Self::Item>) -> Self::Item;
}
```

Plus `BoundedAlloc::try_alloc` or `UnboundedAlloc::alloc` depending
on the macro used.
