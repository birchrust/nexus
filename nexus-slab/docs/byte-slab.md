# Byte Slab

Type-erased slab storage for heterogeneous types. The slab stores raw
bytes (`AlignedBytes<N>`) instead of typed `SlotCell<T>`. Values are
written as bytes and reconstructed via pointer casts.

## When to Use

- Multiple types in the same slab (all must fit within `N` bytes)
- Type is not known at slab construction time
- Need `dyn Trait` storage without heap allocation per object

## Usage

```rust
mod alloc {
    nexus_slab::bounded_byte_allocator!(MyTrait, 64);
    // Stores any type implementing MyTrait that fits in 64 bytes
}
```

The byte slab allocates `AlignedBytes<64>` slots. Values are written as
bytes and read back via pointer casts. The `byte::BoxSlot` wraps a raw
pointer to the value (potentially fat pointer for `dyn Trait`).

## Drop Behavior

Byte slabs handle `Drop` differently from typed slabs:

- **Typed slab `free()`** — calls `drop_in_place::<T>()` then returns slot
- **Byte slab `free()`** — only returns slot. Does NOT drop the value.
- **`byte::BoxSlot::drop()`** — calls `drop_in_place::<T>()` then `free()`

This means: if you use raw `free()` on a byte slab, you must drop the
value yourself first. `byte::BoxSlot` handles this automatically.

## Alignment

`AlignedBytes<N>` is aligned to `N` bytes (up to the platform's max
alignment). Values whose alignment exceeds `N` cannot be stored.
