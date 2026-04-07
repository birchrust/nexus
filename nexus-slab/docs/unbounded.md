# Unbounded Slab

Growable slab using independent chunks. No copy on growth. Existing
handles remain valid when new chunks are added.

## Construction

```rust
use nexus_slab::unbounded;

// SAFETY: caller upholds the slab contract (see struct docs)

// Direct — start with chunks of 1024 slots each
let slab = unsafe { unbounded::Slab::<MyType>::with_chunk_capacity(1024) };

// Builder — configure chunk size and pre-allocate
let slab = unsafe {
    unbounded::Builder::new()
        .chunk_capacity(1024)
        .initial_chunks(4)   // 4 × 1024 = 4096 slots ready
        .build::<MyType>()
};
```

## Allocation

```rust
// Always succeeds — grows if needed
let slot = slab.alloc(MyType::new());
```

`alloc` never fails. If the current chunk is full, a new chunk is
allocated. The new chunk is independent — no copying, no pointer
invalidation. Existing slots in prior chunks remain valid.

## Growth Behavior

```
Chunk 0: [slot][slot][slot]...[slot]  ← original, still valid
Chunk 1: [slot][slot][slot]...[slot]  ← added when chunk 0 filled
Chunk 2: [slot][slot][slot]...[slot]  ← added when chunk 1 filled
```

Each chunk is a separate heap allocation. Slots within a chunk are
contiguous. Slots across chunks are not.

Growth cost: ~40 cycles p999 (one heap allocation per chunk). Once
allocated, the chunk persists for the slab's lifetime.

## Two-Phase Allocation

Same `claim()` API as bounded:

```rust
let claim = slab.claim();
let slot = claim.write(MyType::new());
```

## Deallocation

Same as bounded — safe, move-only handles:

```rust
slab.free(slot);
let value = slab.take(slot);
```

Freed slots are returned to their chunk's freelist.

## Capacity

```rust
slab.capacity();        // total slots across all chunks
slab.chunk_count();     // number of allocated chunks
slab.reserve_chunks(n); // ensure at least n chunks exist
```
