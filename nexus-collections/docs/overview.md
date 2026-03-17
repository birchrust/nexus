# Overview

nexus-collections provides intrusive data structures backed by
`nexus-slab`. Nodes live in slab slots. Handles are reference-counted
(`RcSlot`). Operations are O(1) for lists, O(log n) for trees/heaps.
Zero allocation after the slab is initialized.

## Why Slab-Backed?

Standard collections (`Vec`, `BTreeMap`, `LinkedList`) allocate from
the global allocator on every insert. In a hot loop processing millions
of events, this creates allocation jitter and fragmentation.

Slab-backed collections pre-allocate all node storage. Insert pops from
the freelist (O(1)). Remove pushes back (O(1)). The slab never fragments
because all slots are the same size.

## Collection Types

| Collection | Structure | Insert | Remove | Lookup | Ordered |
|------------|-----------|--------|--------|--------|---------|
| `List<T>` | Doubly-linked list | O(1) | O(1) | O(n) | Insertion order |
| `Heap<T>` | Pairing heap | O(1) | O(log n) pop | O(1) peek | Min-first |
| `RbTree<K,V>` | Red-black tree | O(log n) | O(log n) | O(log n) | Key order |
| `BTree<K,V>` | B-tree | O(log n) | O(log n) | O(log n) | Key order |

## Ownership Model

```
Collection (List, Heap, RbTree, BTree)
    │
    ├── owns structural links (prev/next pointers, parent/child)
    │
    └── nodes live in slab slots (RcSlot handles)
            │
            └── slab owns the memory
                collection owns the structure
                user owns the handles
```

**Key invariant:** The slab owns the memory. The collection owns the
structural relationships (links between nodes). The user holds `RcSlot`
handles that prevent deallocation while in use.

When a node is removed from the collection, its structural links are
cleared, but the slab slot persists as long as any `RcSlot` handle
exists. When the last handle drops, the slot returns to the freelist.

## Node Types

Each collection wraps its values in a node type:

| Collection | Node Type | Contains |
|------------|-----------|----------|
| `List<T>` | `ListNode<T>` | value + prev/next pointers + owner ID |
| `Heap<T>` | `HeapNode<T>` | value + child/sibling pointers + owner ID |
| `RbTree<K,V>` | `RbNode<K,V>` | key + value + parent (with color bit) + left/right |
| `BTree<K,V>` | `BTreeNode<K,V,B>` | keys + values + children arrays |

Users don't construct node types directly. The collection's `push`,
`insert`, etc. methods allocate from the slab and return handles.

## Allocator Setup

Collections are generic over `A: Alloc`. The allocator is typically
generated via macro:

```rust
mod order_queue {
    use super::Order;
    nexus_collections::list_allocator!(Order, bounded);
}

// Initialize
order_queue::Allocator::builder().capacity(1024).build().unwrap();

// Create the collection
let mut list = order_queue::List::new(order_queue::Allocator);
```

See [Macros](macros.md) for the full set of allocator macros.

## Moving Nodes Between Collections

A key feature: nodes can be moved between collections without
deallocation. The node's slab slot persists — only the structural
links change.

```rust
// Remove from list A (O(1))
let handle = list_a.remove(handle);

// Insert into list B (O(1))
list_b.push_back_handle(&handle);
```

The slab slot is reused. No allocation, no deallocation. The node
moves from one list to another in two pointer operations.

This is why the storage is separate from the structure — it enables
zero-copy movement between collections.

## Collection IDs

Each collection instance has a unique ID (thread-local counter). Nodes
track which collection they belong to. Attempting to remove a node from
the wrong collection is caught by `debug_assert!`.

This prevents a class of bugs where a handle from collection A is
accidentally passed to collection B.
