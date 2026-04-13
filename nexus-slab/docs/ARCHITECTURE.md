# Architecture

nexus-slab provides pre-allocated memory pools with O(1) alloc/free and
deterministic latency. No system allocator calls on the hot path. Pay at
startup, never at message 1,000,000.

## Design Decisions

**Why not just Box?** Box calls `malloc`/`free` which contend on the global
allocator lock, fragment memory, and have unpredictable tail latency
(OS may need to map new pages). A slab pre-allocates a contiguous block
and manages a freelist — O(1), no lock, no syscall, no fragmentation.

**Why not a pool (like nexus-pool)?** Pools return values to a shared
collection. Slabs return raw slots — the caller manages the value lifetime.
Slabs are lower-level: collections use slabs, pools use a different
abstraction. The slab doesn't know what's stored in each slot.

**Why SLUB-style unions?** Each slot is either a freelist link OR a value.
No header byte, no tag, no sentinel. The `Slot` RAII handle IS the proof
of occupancy. Writing a value implicitly transitions the slot from vacant
to occupied by overwriting the freelist pointer. Maximum density — every
byte is either data or freelist link, never wasted.

## Slab Types

```
                        ┌──────────────────┐
                        │   User chooses   │
                        └────────┬─────────┘
                                 │
                    ┌────────────┴────────────┐
                    │                         │
             Fixed capacity?            Shared ownership?
                    │                         │
              ┌─────┴─────┐             ┌─────┴─────┐
              │           │             │           │
          Bounded     Unbounded     rc::Bounded  rc::Unbounded
          ~20 cy      ~20 cy        ~26 cy       ~26 cy
          (panics     (grows by     (refcount    (refcount
           if full)    chunks)       + borrow)    + borrow)
              │           │             │           │
              └─────┬─────┘             └─────┬─────┘
                    │                         │
              byte::Bounded            (no byte rc yet)
              byte::Unbounded
              (type-erased)
```

**Bounded:** Fixed capacity. Panics or returns `Err(Full)` when exhausted.
Use when you know the maximum count (order book levels, connections).

**Unbounded:** Grows by allocating new chunks. No copy on growth (unlike
Vec). Chunks are independent blocks linked by a freelist-of-freelists.
~40 cycle p999 during growth (chunk allocation) vs ~2000+ for Vec-based.

**Rc variants:** Reference-counted slots for shared ownership. Used by
nexus-collections where a node may be referenced from multiple data
structures simultaneously (e.g., a list node that's also in a hashmap).

**Byte variants:** Type-erased. Store any type that fits in `N` bytes.
Used by nexus-async-rt for slab-allocated tasks where the task type is
erased behind a function pointer.

## SlotCell: The Core Primitive

```rust
#[repr(C)]
pub union SlotCell<T> {
    next_free: *mut SlotCell<T>,    // when vacant
    value: ManuallyDrop<MaybeUninit<T>>,  // when occupied
}
```

The union occupies `max(8, size_of::<T>())` bytes. When vacant, the first
8 bytes are a freelist pointer. When occupied, all bytes are the value.
No discriminant — the `Slot` handle's existence proves occupancy.

**Lifecycle:**
```
Vacant:  [next_free ptr] [... padding ...]
           │
  claim()  │  (pop from freelist)
           ▼
Claimed: [next_free ptr] [... padding ...]  ← Claim handle
           │
  write(v) │  (overwrite with value bytes)
           ▼
Occupied: [    value T    ] [... padding ...]  ← Slot handle
           │
  free()   │  (drop value, push to freelist)
           ▼
Vacant:  [next_free ptr] [... padding ...]
```

## Freelist Design

Intrusive singly-linked list through the slots themselves. No separate
allocation for freelist nodes — the vacant slot's memory IS the node.

```
free_head → [slot 3] → [slot 7] → [slot 1] → NULL
             next=7     next=1     next=NULL
```

**Alloc (pop):** Read `free_head`, advance to `next_free`, return slot.
One pointer read, one pointer write. ~20 cycles.

**Free (push):** Set slot's `next_free` to current `free_head`, update
`free_head` to point to slot. One pointer read, two pointer writes.
~20 cycles.

## Pointer Provenance

All freelist pointers are derived from `UnsafeCell::get()` with write
provenance. The backing `Vec<SlotCell<T>>` is wrapped in
`UnsafeCell<Vec<SlotCell<T>>>` and the `free_head` pointer is derived
AFTER the UnsafeCell wrapping — not before (which would give stale
read-only provenance under stacked borrows).

The `value_ptr_mut` method on `SlotCell` returns `*mut T` from a raw
`*mut SlotCell<T>` without creating an intermediate `&SlotCell` reference.
This preserves write provenance for consumers (nexus-collections, nexus-async-rt)
that need to mutate through the pointer.

## Two-Phase Allocation

```rust
// Phase 1: Claim a slot (reserves it from freelist)
let claim = slab.claim().expect("full");

// Phase 2: Write the value (transitions to occupied)
let slot = claim.write(MyValue { ... });

// Or: drop the claim to return it without writing
drop(claim); // slot goes back to freelist
```

The two-phase API enables placement optimization — the compiler can construct
the value directly in the slab slot via `copy_nonoverlapping`, avoiding a
stack copy. The `Claim` handle is an RAII guard: if dropped without calling
`write()`, the slot is returned to the freelist. No slot leak on error paths.

## Unbounded Growth

Unbounded slabs grow by allocating new `BoundedSlab` chunks:

```
┌─────────┐   ┌─────────┐   ┌─────────┐
│ Chunk 0 │   │ Chunk 1 │   │ Chunk 2 │
│ 32 slots│   │ 32 slots│   │ 32 slots│
│ [freelist]   [freelist]   [freelist] │
└─────────┘   └─────────┘   └─────────┘
      │
  head_with_space → chunk with free slots
```

**No copy on growth.** Vec-based pools must reallocate and copy when growing.
Slab chunks are independent — existing pointers stay valid. The only cost of
growth is the chunk allocation itself (~40 cycles for the `Box::new`).

**Chunk capacity:** Configurable via `with_chunk_capacity()`. Larger chunks
= fewer growth events. Smaller chunks = less wasted memory if the slab is
oversized. Default balances these.

## Rc Slab

Reference-counted variant for shared ownership:

```rust
let slab = unsafe { rc::bounded::Slab::with_capacity(100) };
let slot: RcSlot<T> = slab.alloc(value);
let clone: RcSlot<T> = slot.clone(); // refcount: 2
drop(slot);  // refcount: 1, value alive
drop(clone); // refcount: 0, value dropped, slot freed
```

Each `RcCell<T>` wraps the value in `UnsafeCell` and stores a refcount +
borrow state in a `Cell<u64>`. Borrow rules (checked at runtime in debug):
- Multiple `&T` borrows allowed simultaneously
- One `&mut T` borrow, exclusive
- Pin support for intrusive data structures

Used by nexus-collections: a `ListNode<T>` lives in an `RcSlot` and may
be referenced from the list, a hashmap, and user code simultaneously.
