# nexus-slab

A high-performance slab allocator optimized for **predictable tail latency**.

## Why nexus-slab?

Traditional slab allocators using `Vec` exhibit bimodal p999 latency—most operations are fast, but occasional reallocations cause multi-thousand cycle spikes. For latency-critical systems like trading engines, a single slow operation can mean a missed fill.

`nexus-slab` provides two allocators:

- **`BoundedSlab`**: Fixed capacity, pre-allocated. The production choice for deterministic latency.
- **`Slab`**: Grows by adding independent chunks. No copying on growth—only the new chunk is allocated.

## Performance

Benchmarked on Intel Core Ultra 7 155H, pinned to a physical core.

### BoundedSlab (fixed capacity)

| Operation | BoundedSlab | slab crate | Notes |
|-----------|-------------|------------|-------|
| INSERT p50 | ~20 cycles | ~22 cycles | 2 cycles faster |
| GET p50 | ~24 cycles | ~26 cycles | 2 cycles faster |
| REMOVE p50 | ~24 cycles | ~30 cycles | 6 cycles faster |

### Slab (growable)

Steady-state p50 matches `slab` crate (~22-32 cycles). The win is tail latency during growth:

| Metric | Slab | slab crate | Notes |
|--------|------|------------|-------|
| Growth p999 | ~40 cycles | ~2000+ cycles | 50x better |
| Growth max | ~70K cycles | ~1.5M cycles | 20x better |

`Slab` adds chunks independently—no copying. The `slab` crate uses `Vec`, which copies all existing data on reallocation.

## Usage

### BoundedSlab (fixed capacity)
```rust
use nexus_slab::{BoundedSlab, Full};

// All memory allocated upfront, pre-touched
let mut slab: BoundedSlab<u64> = BoundedSlab::with_capacity(100_000);

// O(1) operations
let key = slab.try_insert(42).unwrap();
assert_eq!(slab[key], 42);

let value = slab.remove(key);
assert_eq!(value, 42);

// Returns Err(Full) when exhausted—no surprise allocations
match slab.try_insert(123) {
    Ok(key) => { /* ... */ }
    Err(Full(value)) => { /* handle backpressure */ }
}
```

### Slab (growable)
```rust
use nexus_slab::Slab;

// Lazy allocation—no memory until first insert
let mut slab: Slab<u64> = Slab::new();

// Or pre-allocate for expected capacity
let mut slab: Slab<u64> = Slab::with_capacity(10_000);

// Grows automatically when needed
let key = slab.insert(42);
assert_eq!(slab[key], 42);

let value = slab.remove(key);
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

### Vacant Entry Pattern

For self-referential structures where the value needs to know its own key:
```rust
use nexus_slab::{BoundedSlab, Key};

struct Node {
    self_key: Key,
    data: u64,
}

let mut slab = BoundedSlab::with_capacity(1024);

let entry = slab.try_vacant_entry().unwrap();
let key = entry.key();
entry.insert(Node { self_key: key, data: 42 });

assert_eq!(slab[key].self_key, key);
```

## Architecture

### Memory Layout
```
BoundedSlab (single contiguous allocation):
┌─────────────────────────────────────────────┐
│ Slot 0: [tag: u32][value: T]                │
│ Slot 1: [tag: u32][value: T]                │
│ ...                                         │
│ Slot N: [tag: u32][value: T]                │
└─────────────────────────────────────────────┘

Slab (multiple independent chunks):
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│ Chunk 0      │  │ Chunk 1      │  │ Chunk 2      │
│ (BoundedSlab)│  │ (BoundedSlab)│  │ (BoundedSlab)│
└──────────────┘  └──────────────┘  └──────────────┘
       ▲                                   ▲
       └─── head_with_space ───────────────┘
             (freelist of non-full chunks)
```

### Slot Tag Encoding

Each slot has a `tag: u32`:

- **Occupied**: `tag == 0`
- **Vacant**: bit 31 set, bits 0-30 encode next free slot index

Single comparison (`tag == 0`) checks occupancy.

### Allocation Strategy

1. **Check freelist head**: O(1) access to a slot (or chunk) with space
2. **LIFO reuse**: Recently-freed slots reused first for cache locality
3. **Pop when full**: Exhausted chunks removed from freelist
4. **Growth** (Slab only): Allocate new chunk when all are full

### Key Encoding

`BoundedSlab` keys are direct slot indices.

`Slab` keys encode chunk and local index via power-of-2 arithmetic:
```
┌─────────────────────┬──────────────────────┐
│  chunk_idx (high)   │  local_idx (low)     │
└─────────────────────┴──────────────────────┘
```

Decoding is two instructions: shift and mask.

## Key Validity

Keys are simple indices with occupancy checks. After removal, `get()` returns `None`:
```rust
let key = slab.insert(42);
assert!(slab.contains(key));

slab.remove(key);
assert!(!slab.contains(key));  // Slot is vacant
assert!(slab.get(key).is_none());
```

**No generational indices.** If a new value occupies the same slot, an old key will access the new value. For systems requiring stale-key protection, validate against authoritative external identifiers (exchange order IDs, database keys, etc.).

See the [`Key`](https://docs.rs/nexus-slab/latest/nexus_slab/struct.Key.html) documentation for design rationale.

## API Summary

| Method | BoundedSlab | Slab | Description |
|--------|-------------|------|-------------|
| `try_insert(value)` | `Result<Key, Full>` | — | Insert, returns `Err` if full |
| `insert(value)` | — | `Key` | Insert, grows if needed |
| `get(key)` | `Option<&T>` | `Option<&T>` | Get reference |
| `get_mut(key)` | `Option<&mut T>` | `Option<&mut T>` | Get mutable reference |
| `remove(key)` | `T` | `T` | Remove and return (panics if invalid) |
| `slab[key]` | `&T` / `&mut T` | `&T` / `&mut T` | Index access (panics if invalid) |
| `contains(key)` | `bool` | `bool` | Check if slot is occupied |
| `len()` | `usize` | `usize` | Number of occupied slots |
| `capacity()` | `usize` | `usize` | Total slots available |
| `is_full()` | `bool` | — | Check if at capacity |
| `clear()` | `()` | `()` | Remove all elements |

## When to Use This

**Use `BoundedSlab` when:**
- Capacity is known upfront
- You need deterministic latency (no allocations after init)
- Production trading systems, matching engines

**Use `Slab` when:**
- Capacity is unknown or needs overflow headroom
- Growth is infrequent and latency spikes during growth are acceptable
- You want the tail latency benefits over `Vec`-based slabs

**Use the `slab` crate when:**
- You don't need the tail latency guarantees
- Simpler dependency is preferred

## License

MIT OR Apache-2.0
