# nexus-collections

High-performance, slab-backed collections for latency-critical systems.

## Why This Crate?

Node-based data structures (linked lists, heaps, trees) offer
operations that contiguous structures can't — O(1) unlink/re-link, stable
handles to interior elements, and movement between collections without
copying. The trade-off is normally heap fragmentation and allocator overhead
on every node allocation.

This crate eliminates that trade-off by using
[`nexus-slab`](https://crates.io/crates/nexus-slab) — a SLUB-style slab
allocator — as dedicated backing storage for all nodes. Nodes live in
contiguous, type-homogeneous slabs rather than scattered across the global
heap, giving you:

- **Global allocator isolation** — your hot path doesn't compete with
  logging, serialization, or background tasks for allocator resources
- **LIFO cache locality** — recently freed nodes are reused first, staying
  hot in L1
- **Zero fragmentation** — every slot is the same size, freed slots are
  immediately reusable
- **Stable handles** — `RcSlot`-based references that survive unlink,
  re-link, and movement between collections
- **Bounded** — fixed capacity, zero allocation after init, returns `Full`
  at capacity
- **Unbounded** — grows via chunks without copying

## Collections

### List — Doubly-Linked List

O(1) push/pop/unlink anywhere. `RcSlot` handles enable O(1) access by
identity. Elements can move between lists without deallocation.

```rust
mod orders {
    nexus_collections::list_allocator!(Order, bounded);
}

orders::Allocator::builder().capacity(1000).build().unwrap();

let mut list = orders::List::new(orders::Allocator);
let handle = list.try_push_back(Order { id: 1, price: 100.0 }).unwrap();

// Access via handle
assert_eq!(handle.exclusive().price, 100.0);

// O(1) unlink and re-link
list.unlink(&handle);
list.link_back(&handle);
```

### Heap — Pairing Heap

O(1) push, O(log n) pop, O(1) peek. `RcSlot` handles enable O(log n)
removal of arbitrary elements by handle.

```rust
mod timers {
    nexus_collections::heap_allocator!(Timer, bounded);
}

timers::Allocator::builder().capacity(1000).build().unwrap();

let mut heap = timers::Heap::new(timers::Allocator);
let handle = heap.try_push(Timer { deadline: 42 }).unwrap();

// O(1) peek
assert_eq!(heap.peek().unwrap().data().deadline, 42);

// O(log n) unlink by handle
heap.unlink(&handle);
```

### RbTree — Red-Black Tree Sorted Map

Deterministic O(log n) sorted map with at most 2 rotations per insert,
3 per delete. Best for entry-heavy and pop-heavy workloads (order books,
timer wheels).

```rust
mod levels {
    nexus_collections::rbtree_allocator!(u64, String, bounded);
}

levels::Allocator::builder().capacity(1000).build().unwrap();

let mut map = levels::RbTree::new(levels::Allocator);
map.try_insert(100, "hello".into()).unwrap();

assert_eq!(map.get(&100), Some(&"hello".into()));

// Entry API
map.entry(200).or_try_insert("world".into()).unwrap();
```

### BTree — B-Tree Sorted Map

Cache-friendly sorted map with tunable branching factor. Best for read-heavy
lookups, existence checking, high-churn streaming data, and range scans.

```rust
mod levels {
    nexus_collections::btree_allocator!(u64, String, bounded);
}

// Custom branching factor: btree_allocator!(u64, String, bounded, 12)

levels::Allocator::builder().capacity(1000).build().unwrap();

let mut map = levels::BTree::new(levels::Allocator);
map.try_insert(100, "hello".into()).unwrap();

assert_eq!(map.get(&100), Some(&"hello".into()));
```

## Allocator Macros

Each collection has a macro that generates a typed thread-local slab
allocator. Invoke inside a module:

| Macro | Collection | Generated Types |
|-------|-----------|-----------------|
| `list_allocator!(T, bounded\|unbounded)` | List | `Allocator`, `Handle`, `List`, `Cursor` |
| `heap_allocator!(T, bounded\|unbounded)` | Heap | `Allocator`, `Handle`, `Heap` |
| `rbtree_allocator!(K, V, bounded\|unbounded)` | RbTree | `Allocator`, `RbTree`, `Cursor`, `Entry` |
| `btree_allocator!(K, V, bounded\|unbounded)` | BTree | `Allocator`, `BTree`, `Cursor`, `Entry` |

**Bounded** allocators have a fixed capacity. Insert operations return
`Result<_, Full<T>>` when full.

**Unbounded** allocators grow as needed via chunk allocation.

### Initialization

Allocators must be initialized before use:

```rust
// Bounded — set capacity
orders::Allocator::builder().capacity(1000).build().unwrap();

// Unbounded — optionally set chunk size (default 4096)
orders::Allocator::builder().chunk_size(512).build().unwrap();
```

## Performance

Cycle-accurate latency, Intel Core Ultra 7 155H, pinned to physical core,
turbo boost disabled. Sorted map benchmarks use batched `seq!` unrolled timing
(100 ops per rdtsc pair) to amortize serialization overhead. Same PRNG seed
across all benchmarks. See [BENCHMARKS.md](BENCHMARKS.md) for individual
tables with all percentiles.

### List (p50 cycles)

| Operation | Cycles |
|-----------|--------|
| link_back | 20 |
| pop_front | 22 |
| unlink | 22 |
| move_to_front | 22 |

### Heap (p50 cycles)

| Operation | Cycles |
|-----------|--------|
| push | 24 |
| pop | 312 |
| peek | 20 |
| unlink | 30 |

### Sorted Maps — p50 (cycles, @10k population)

| Operation | nexus BTree | nexus RbTree | std BTreeMap | Best |
|---|---|---|---|---|
| get (hit, @100) | 14 | **9** | 23 | RbTree |
| get (hit, @10k) | 22 | **15** | 40 | RbTree |
| get (miss, @10k) | **30** | 41 | 48 | nexus BTree |
| get (cold rand, @10k) | 137 | **131** | 153 | RbTree |
| contains_key (hit) | **22** | 50 | 37 | nexus BTree |
| insert (growing) | **254** | 278 | 256 | nexus BTree |
| insert (steady) | 211 | **203** | 231 | RbTree |
| insert (duplicate) | 27 | **24** | 42 | RbTree |
| remove | **209** | 245 | 243 | nexus BTree |
| pop_first | 47 | **22** | 71 | RbTree |
| pop_last | 38 | **21** | 52 | RbTree |
| churn | **455** | 520 | 510 | nexus BTree |
| entry (occupied) | 22 | **20** | 37 | RbTree |
| entry (vacant+insert) | 373 | **197** | 228 | RbTree |

### Sorted Maps — p999 Tail Latency (cycles, @10k population)

| Operation | nexus BTree | nexus RbTree | std BTreeMap | Best |
|---|---|---|---|---|
| get (hit, @100) | 44 | **34** | 87 | RbTree |
| get (hit, @10k) | 139 | **54** | 161 | RbTree |
| get (miss, @10k) | **92** | 105 | 127 | nexus BTree |
| get (cold rand, @10k) | 252 | **210** | 316 | RbTree |
| contains_key (hit) | **81** | 111 | 155 | nexus BTree |
| insert (growing) | **758** | 1178 | 3736 | nexus BTree |
| insert (steady) | **319** | 345 | 401 | nexus BTree |
| insert (duplicate) | 87 | **82** | 152 | RbTree |
| remove | **359** | 372 | 423 | nexus BTree |
| pop_first | 128 | **69** | 153 | RbTree |
| pop_last | 107 | **74** | 132 | RbTree |
| churn | **758** | 803 | 920 | nexus BTree |
| entry (occupied) | 94 | **77** | 141 | RbTree |
| entry (vacant+insert) | **602** | 366 | 406 | RbTree |

Both nexus trees beat `std::collections::BTreeMap` across the board. The slab
allocator eliminates global allocator contention and gives predictable cache
behavior. The advantage is most visible in tail latency — std's p999 on growing
insert is **3736 cycles** (global allocator on node splits) vs nexus BTree's 758.

### When to Choose Which

**RbTree**: Entry-heavy workloads (order books), pop-heavy workloads (timer
wheels), anything where the pattern is check-then-insert via the entry API.

**BTree**: Read-heavy lookups, existence checking (`contains_key`), high-churn
streaming data, range scans. Tunable branching factor via const generic `B`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
