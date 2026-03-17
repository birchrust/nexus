# Bounded Slab

Fixed-capacity slab. Allocates once at init, never grows. Zero allocation
jitter after setup.

## Construction

```rust
use nexus_slab::bounded;

// Pre-allocate 1024 slots
let slab = bounded::Slab::<MyType>::with_capacity(1024);
```

All 1024 slots are allocated upfront. No further heap allocation ever.

## Allocation

```rust
// Infallible — panics if full
let slot = slab.alloc(MyType::new());

// Fallible — returns error with value back
match slab.try_alloc(MyType::new()) {
    Ok(slot) => { /* use slot */ },
    Err(Full(value)) => { /* slab is full, value returned */ },
}
```

`alloc` returns a `RawSlot<T>` — a raw pointer wrapper. You must free it
explicitly or convert to a `BoxSlot` for RAII.

## Two-Phase Allocation (Claim)

For placement-new optimization — claim the slot first, write the value
directly into it:

```rust
if let Some(claim) = slab.claim() {
    let slot = claim.write(MyType::new());  // value constructed in-place
    // ...
}
```

The `Claim` reserves a slot from the freelist. If you drop the claim
without calling `write()`, the slot is returned to the freelist.

## Deallocation

```rust
// Free — drops the value and returns the slot to the freelist
unsafe { slab.free(slot); }

// Take — moves the value out and returns the slot to the freelist
let value = unsafe { slab.take(slot); }
```

Both are `unsafe` because the caller must guarantee:
- The slot was allocated from this slab
- No references to the slot's value exist

## Capacity

```rust
assert_eq!(slab.capacity(), 1024);
assert!(slab.is_initialized());
```
