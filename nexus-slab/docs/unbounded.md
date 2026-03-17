# Unbounded Slab

Growable slab using independent chunks. No copy on growth. Existing
handles remain valid when new chunks are added.

## Construction

```rust
use nexus_slab::unbounded;

// Start with chunks of 1024 slots each
let slab = unbounded::Slab::<MyType>::with_chunk_capacity(1024);

// Pre-allocate multiple chunks upfront
let slab = unbounded::Slab::<MyType>::with_chunk_capacity(1024);
slab.reserve_chunks(4);  // 4 × 1024 = 4096 slots ready
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
let (claim, chunk_idx) = slab.claim();
let slot = claim.write(MyType::new());
```

The unbounded claim also returns the chunk index.

## Deallocation

Same as bounded:

```rust
unsafe { slab.free(slot); }
let value = unsafe { slab.take(slot); }
```

Freed slots are returned to their chunk's freelist.

## Capacity

```rust
slab.capacity();      // total slots across all chunks
slab.chunk_count();   // number of allocated chunks
slab.reserve_chunks(n); // ensure at least n chunks exist
```
