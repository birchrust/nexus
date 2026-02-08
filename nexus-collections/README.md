# nexus-collections

High-performance, slab-backed collections for latency-critical systems.

## Why This Crate?

Standard collections allocate on every insert and scatter nodes across the
heap. This crate uses thread-local slab allocators from
[`nexus-slab`](https://crates.io/crates/nexus-slab) to pre-allocate storage
at startup, eliminating allocation jitter on the hot path.

- **Zero allocation after initialization** — all storage is pre-allocated
- **Stable handles** — `RcSlot`-based references that survive insert/remove
- **Bounded capacity** — fixed capacity with `Full` errors, no surprise growth
- **Cache-friendly** — contiguous slab-backed storage

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

### SkipList — Sorted Map

Probabilistic sorted map with O(log n) insert/lookup/remove. Internal
allocation — user sees only keys and values.

```rust
mod levels {
    nexus_collections::skip_allocator!(u64, String, bounded);
}

levels::Allocator::builder().capacity(1000).build().unwrap();

let mut map = levels::SkipList::new(levels::Allocator);
map.try_insert(100, "hello".into()).unwrap();

assert_eq!(map.get(&100), Some(&"hello".into()));

// Sorted iteration
for (k, v) in map.iter() {
    println!("{k}: {v}");
}

// Entry API
map.entry(200).or_try_insert("world".into()).unwrap();
```

## Allocator Macros

Each collection has a macro that generates a typed thread-local slab
allocator. Invoke inside a module:

| Macro | Collection | Generated Types |
|-------|-----------|-----------------|
| `list_allocator!(T, bounded\|unbounded)` | List | `Allocator`, `Handle`, `List`, `Cursor` |
| `heap_allocator!(T, bounded\|unbounded)` | Heap | `Allocator`, `Handle`, `Heap` |
| `skip_allocator!(K, V, bounded\|unbounded)` | SkipList | `Allocator`, `SkipList`, `Cursor`, `Entry` |

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
turbo boost disabled. See [BENCHMARKS.md](BENCHMARKS.md) for full results.

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

### SkipList (p50 cycles, @10k population)

| Operation | Cycles |
|-----------|--------|
| get (hit) | 171 |
| insert | 510 |
| remove | 544 |
| pop_first | 26 |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
