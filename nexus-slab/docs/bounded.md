# Bounded Slab

Fixed-capacity slab. Allocates once at construction, never grows. Zero
allocation jitter after setup.

## Construction

```rust
use nexus_slab::bounded;

// SAFETY: caller upholds the slab contract (see struct docs)
let slab = unsafe { bounded::Slab::<MyType>::with_capacity(1024) };
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

`alloc` returns a `Slot<T>` — a move-only pointer handle. You must free
it explicitly via `slab.free(slot)`.

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
slab.free(slot);

// Take — moves the value out and returns the slot to the freelist
let value = slab.take(slot);
```

`free()` and `take()` are safe — the safety contract was accepted at
construction time. `Slot` is move-only and consumed on free, preventing
double-free at the type level.

## Capacity

```rust
assert_eq!(slab.capacity(), 1024);
```
