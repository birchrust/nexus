# Collection Allocator Macros

Each collection type has a macro that generates a slab allocator and
type aliases for the collection and its handles.

## Available Macros

| Macro | Collection | Node Type |
|-------|-----------|-----------|
| `list_allocator!(T, bounded\|unbounded)` | `List<T>` | `ListNode<T>` |
| `heap_allocator!(T, bounded\|unbounded)` | `Heap<T>` | `HeapNode<T>` |
| `rbtree_allocator!(K, V, bounded\|unbounded)` | `RbTree<K, V>` | `RbNode<K, V>` |
| `btree_allocator!(K, V, bounded\|unbounded)` | `BTree<K, V>` | `BTreeNode<K, V>` |

## What Gets Generated

```rust
mod my_list {
    nexus_collections::list_allocator!(Order, bounded);
}

// This generates:
// - my_list::Allocator          — zero-sized allocator (backed by TLS slab)
// - my_list::Builder             — builder for initialization
// - my_list::List                — type alias for List<Order, Allocator>
// - my_list::Handle              — type alias for RcSlot<ListNode<Order>, Allocator>
// - my_list::WeakHandle          — type alias for WeakSlot<...>
```

## Initialization

```rust
// Bounded — fixed capacity
my_list::Allocator::builder().capacity(1024).build().unwrap();

// Unbounded — grows in chunks
mod my_heap {
    nexus_collections::heap_allocator!(Timer, unbounded);
}
my_heap::Allocator::builder()
    .chunk_size(512)
    .initial_chunks(2)
    .build()
    .unwrap();
```

## Bounded vs Unbounded

- **`bounded`** — fixed capacity. `try_insert` / `try_push` return
  `Err(Full)` at capacity. Use when you know the maximum count.
- **`unbounded`** — grows by adding chunks. `insert` / `push` always
  succeed. Use when the count is unknown.

Bounded collections also have `insert` / `push` which panic on full,
but `try_insert` / `try_push` are preferred.

## Thread Safety

Like `nexus-slab` macros, the generated allocators are thread-local.
Collections must be used on the thread that initialized the allocator.
Handles are `!Send`.
