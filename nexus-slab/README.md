# nexus-slab

A high-performance slab allocator optimized for **predictable tail latency**.

## Why nexus-slab?

Traditional slab allocators using `Vec` exhibit bimodal p999 latency—most operations are fast, but occasional reallocations cause multi-thousand cycle spikes. For latency-critical systems, a single slow operation can mean a missed fill.

`nexus-slab` provides two allocators:

- **`BoundedSlab`**: Fixed capacity, pre-allocated. The production choice for deterministic latency.
- **`Slab`**: Grows by adding independent chunks. No copying on growth—only the new chunk is allocated.

Both use an **Entry-based API** where `insert()` returns a handle (`Entry<T>`) for direct access to the value.

## Performance

Benchmarked on Intel Core Ultra 7 155H, pinned to a physical core.

### BoundedSlab (fixed capacity)

| Operation | BoundedSlab | slab crate | Notes |
|-----------|-------------|------------|-------|
| INSERT p50 | ~22 cycles | ~24 cycles | Comparable |
| GET p50 | ~22 cycles | ~28 cycles | 21% faster (unchecked) |
| REMOVE p50 | ~30 cycles | ~34 cycles | 12% faster |

### Slab (growable)

Steady-state p50 matches `slab` crate (~30-40 cycles). The win is tail latency during growth:

| Metric | Slab | slab crate | Notes |
|--------|------|------------|-------|
| Growth p999 | ~64 cycles | ~2700+ cycles | 43x better |
| Growth max | ~230K cycles | ~2.7M cycles | 12x better |

`Slab` adds chunks independently—no copying. The `slab` crate uses `Vec`, which copies all existing data on reallocation.

## Usage

### BoundedSlab (fixed capacity)

```rust
use nexus_slab::BoundedSlab;

// All memory allocated upfront
let slab = BoundedSlab::with_capacity(100_000);

// Insert returns Result<Entry, Full<T>>
let entry = slab.insert(42).unwrap();
assert_eq!(*entry.get(), 42);

// Modify through the entry
*entry.get_mut() = 100;

// Remove via entry
let value = entry.remove();
assert_eq!(value, 100);

// Returns Err(Full(value)) when full—recover the rejected value
if let Err(full) = slab.insert(123) {
    let rejected_value = full.into_inner();
    // handle backpressure
}
```

### Slab (growable)

```rust
use nexus_slab::Slab;

// Lazy allocation—no memory until first insert
let slab = Slab::new();

// Or pre-allocate for expected capacity
let slab = Slab::with_capacity(10_000);

// Grows automatically, always succeeds
let entry = slab.insert(42);
assert_eq!(*entry.get(), 42);

let value = entry.remove();
assert_eq!(value, 42);
```

### Builder API (Slab only)

```rust
use nexus_slab::Slab;

let slab: Slab<u64> = Slab::builder()
    .chunk_capacity(8192)   // Slots per chunk (default: 4096)
    .reserve(100_000)       // Pre-allocate space for N items
    .build();
```

### Key-Based Access (for collections)

Entry-based access is the primary API, but key-based access is available for integration with data structures like linked lists and heaps:

```rust
use nexus_slab::BoundedSlab;

let slab = BoundedSlab::with_capacity(1024);

let entry = slab.insert(42).unwrap();
let key = entry.key();  // Extract the key

// Key-based access (returns Ref<T> guard with borrow tracking)
assert_eq!(*slab.get(key).unwrap(), 42);

// Unchecked index access via UntrackedAccessor (unsafe, fastest)
// SAFETY: No Entry operations while accessor is in use
let accessor = unsafe { slab.untracked() };
assert_eq!(accessor[key], 42);

// Key-based removal
let value = slab.remove_by_key(key);
assert_eq!(value, 42);
```

### Self-Referential Patterns

`insert_with` provides access to the Entry before the value exists, enabling self-referential structures.
Use `Key` for references (not `Entry`, which is a unique owner):

```rust
use nexus_slab::{BoundedSlab, Key};

struct Node {
    self_key: Key,
    parent: Option<Key>,
    data: u64,
}

let slab = BoundedSlab::leak(1024);

let root = slab.try_insert_with(|e| Node {
    self_key: e.key(),
    parent: None,
    data: 0,
}).unwrap();
let root_key = root.leak();

let child = slab.try_insert_with(|e| Node {
    self_key: e.key(),
    parent: Some(root_key),
    data: 1,
}).unwrap();

assert!(child.get().parent.is_some());
```

### Unchecked Access (hot paths)

For latency-critical code where you can guarantee validity:

```rust
use nexus_slab::BoundedSlab;

let slab = BoundedSlab::with_capacity(1024);
let entry = slab.insert(42).unwrap();

// Safe: ~30 cycles (liveness + borrow check)
let value = entry.get();

// Unchecked: ~20 cycles (direct pointer deref)
// SAFETY: Entry is valid and not borrowed elsewhere
let value = unsafe { entry.get_unchecked() };
```

## Architecture

### Memory Layout

```
BoundedSlab (single contiguous allocation):
┌─────────────────────────────────────────────┐
│ Slot 0: [stamp: u64][value: T]              │
│ Slot 1: [stamp: u64][value: T]              │
│ ...                                         │
│ Slot N: [stamp: u64][value: T]              │
└─────────────────────────────────────────────┘

Slab (multiple independent chunks):
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│ Chunk 0      │  │ Chunk 1      │  │ Chunk 2      │
│ (internal)   │  │ (internal)   │  │ (internal)   │
└──────────────┘  └──────────────┘  └──────────────┘
       ▲                                   ▲
       └─── head_with_space ───────────────┘
             (freelist of non-full chunks)
```

### Slot Stamp Encoding

Each slot has a `stamp: u64` encoding state and key:

- **Bits 63-32 (state)**:
  - Bit 63: Vacant flag (1 = vacant, 0 = occupied)
  - Bit 62: Borrowed flag (runtime borrow tracking)
  - Bits 61-32: Next free slot index (when vacant)
- **Bits 31-0 (key)**: Slot key, stored when claimed

Single comparison (`stamp & VACANT_BIT == 0`) checks occupancy.

### Entry Design

`Entry<T>` is 16 bytes:
- `*mut SlotCell<T>` — direct pointer for O(1) access
- `*const FreeSlotVTable` — vtable with slab pointer and free function

The vtable is leaked once per slab (16 bytes overhead). On drop, Entry calls the vtable's free function to return the slot to the freelist.

The safe API (`get()`, `get_mut()`) sets a borrow bit. The unchecked API bypasses all checks for minimal latency.

### Key Encoding

`BoundedSlab` keys are direct slot indices.

`Slab` keys encode chunk and local index via power-of-2 arithmetic:
```
┌─────────────────────┬──────────────────────┐
│  chunk_idx (high)   │  local_idx (low)     │
└─────────────────────┴──────────────────────┘
```

Decoding is two instructions: shift and mask.

## API Summary

### BoundedSlab

| Method | Returns | Description |
|--------|---------|-------------|
| `insert(value)` | `Result<Entry, Full<T>>` | Insert, returns `Err(Full(value))` if full |
| `insert_with(f)` | `Result<Entry, CapacityError>` | Insert with access to Entry (self-ref) |
| `vacant_entry()` | `Result<VacantEntry, CapacityError>` | Reserve slot, fill later |
| `get(key)` | `Option<Ref<T>>` | Get by key (tracked borrow) |
| `get_mut(key)` | `Option<RefMut<T>>` | Get mutable by key (tracked borrow) |
| `remove(entry)` | `T` | Remove via Entry (fast path) |
| `remove_by_key(key)` | `T` | Remove via key |
| `contains_key(key)` | `bool` | Check if key is valid |
| `untracked()` | `UntrackedAccessor` | Index access (unsafe) |
| `len()` / `capacity()` | `usize` | Slot counts |
| `clear()` | `()` | Remove all elements |

### Slab

| Method | Returns | Description |
|--------|---------|-------------|
| `insert(value)` | `Entry<T>` | Insert, grows if needed |
| `insert_with(f)` | `Entry<T>` | Insert with access to Entry |
| `vacant_entry()` | `SlabVacantEntry<T>` | Reserve slot, fill later |
| `get(key)` | `Option<Ref<T>>` | Get by key (tracked borrow) |
| `get_mut(key)` | `Option<RefMut<T>>` | Get mutable by key (tracked borrow) |
| `remove(entry)` | `T` | Remove via Entry (fast path) |
| `remove_by_key(key)` | `T` | Remove via key |
| `contains_key(key)` | `bool` | Check if key is valid |
| `untracked()` | `SlabUntrackedAccessor` | Index access (unsafe) |
| `len()` / `capacity()` | `usize` | Slot counts |
| `clear()` | `()` | Remove all (keeps allocated chunks) |

### Entry

| Method | Returns | Description |
|--------|---------|-------------|
| `get()` | `Ref<T>` | Borrow with safety checks (panics if invalid) |
| `try_get()` | `Option<Ref<T>>` | Borrow, returns None if invalid |
| `get_mut()` | `RefMut<T>` | Mutable borrow with safety checks |
| `try_get_mut()` | `Option<RefMut<T>>` | Mutable borrow, returns None if invalid |
| `get_unchecked()` | `&T` | Direct access (unsafe) |
| `get_unchecked_mut()` | `&mut T` | Direct mutable access (unsafe) |
| `replace(value)` | `T` | Swap value, return old |
| `and_modify(f)` | `&Self` | Modify in place (chainable) |
| `take()` | `(T, VacantEntry)` | Extract value, keep slot reserved |
| `remove()` | `T` | Remove and return value |
| `key()` | `Key` | Get the key for this entry |
| `is_valid()` | `bool` | Check if entry is valid (slab alive, slot occupied) |

## When to Use This

**Use `BoundedSlab` when:**
- Capacity is known upfront
- You need deterministic latency (no allocations after init)
- Production trading systems, matching engines
- Using with nexus-collections (intrusive data structures)

**Use `Slab` when:**
- Capacity is unknown or needs overflow headroom
- Growth is infrequent and acceptable
- You want the tail latency benefits over `Vec`-based slabs

**Use the `slab` crate when:**
- You don't need the tail latency guarantees
- Simpler dependency is preferred

## No Generational Indices

Keys are simple indices with occupancy checks. After removal, `get()` returns `None`. If a new value occupies the same slot, an old key will access the new value.

**This is intentional.** In real systems, your data has authoritative external identifiers (exchange order IDs, database keys). You validate against those anyway. Generational indices add ~8 cycles per operation to catch bugs that domain validation already catches.

See the [`Key`](https://docs.rs/nexus-slab/latest/nexus_slab/struct.Key.html) documentation for full rationale.

## License

MIT OR Apache-2.0
