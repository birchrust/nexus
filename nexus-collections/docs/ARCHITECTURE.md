# Architecture

Slab-backed intrusive collections. Every node lives in a slab — no heap
allocation after init. O(1) insert/remove for lists, O(log n) for trees.
Designed for order books, timer wheels, and session management where
allocation jitter is unacceptable.

## Why Slab-Backed?

Standard collections (Vec, BTreeMap, HashMap) allocate on insert and
deallocate on remove. Under load, this creates allocation jitter — the
global allocator lock, page faults, fragmentation. For a market data
handler processing 10M updates/sec, a single `malloc` that takes 5us
(OS reclaiming pages) ruins the entire tick.

Slab-backed collections separate allocation from insertion. You pre-allocate
the slab at startup, then insert/remove nodes by moving them between the
slab's freelist and the collection's structure. Zero syscalls, zero
fragmentation, deterministic performance.

## Collection Types

```
┌──────────────┬────────────┬──────────────┬────────────────┐
│    List      │    Heap    │   RbTree     │    BTree       │
│              │            │              │                │
│ Doubly-linked│  Pairing   │  Red-black   │   B-tree       │
│ O(1) push/   │  O(1) push │  O(log n)    │   O(log n)     │
│ pop/remove   │  O(log n)  │  all ops     │   all ops      │
│              │  pop       │              │                │
│ Free lists,  │  Priority  │  Sorted map, │   Sorted map,  │
│ LRU caches,  │  queues,   │  order book  │   order book   │
│ session mgmt │  timers    │  levels      │   levels       │
└──────────────┴────────────┴──────────────┴────────────────┘
```

**List:** Intrusive doubly-linked list. Nodes have prev/next pointers
via `Cell<NodePtr>`. O(1) push_front/push_back/remove anywhere. Cursor
for positional access.

**Heap:** Pairing heap (min-heap). O(1) amortized insert, O(log n)
amortized pop. Two-pass merge-pairs during pop. Good for timer wheels
where insert is hot and pop is periodic.

**RbTree:** Red-black tree. Color stored in LSB of parent pointer
(requires 2-byte node alignment, verified at compile time). Balanced —
O(log n) worst case for all operations. Entry API for in-place modification.

**BTree:** B-tree with configurable fanout `B`. Internal nodes store up to
`B` keys in `MaybeUninit` arrays. Higher cache utilization than RbTree for
large collections. Entry API, cursor, range iteration.

## Ownership Model

Collections use `RcSlot<Node<T>>` — reference-counted slab slots. A node
can be referenced from multiple places simultaneously:

```
┌──────────┐     ┌─────────────┐     ┌──────────┐
│  List    │────▶│  RcSlot     │◀────│ HashMap  │
│ (free    │     │  ListNode   │     │ (lookup) │
│  list)   │     │  refcount=2 │     │          │
└──────────┘     └─────────────┘     └──────────┘
```

When the node is removed from the list, refcount drops to 1 (HashMap still
holds it). When removed from the HashMap, refcount drops to 0 and the slab
slot is freed.

This is the intrusive collection pattern — the node IS the storage, not a
wrapper around the storage. The slab owns the memory. The collections own
the structure. The user owns the references.

## Node Structure

Each collection type has its own node type:

```rust
// List: prev/next pointers, owner ID, value
pub struct ListNode<T> {
    prev: Cell<NodePtr<T>>,
    next: Cell<NodePtr<T>>,
    owner: Cell<usize>,
    pub value: T,
}

// Heap: parent/child/sibling pointers, owner ID, value
pub struct HeapNode<T> {
    parent: Cell<NodePtr<T>>,
    child: Cell<NodePtr<T>>,
    next: Cell<NodePtr<T>>,
    prev: Cell<NodePtr<T>>,
    owner: Cell<usize>,
    pub value: T,
}

// RbTree: key, left/right, parent+color (LSB), value
pub struct RbNode<K, V> {
    key: K,
    left: Cell<NodePtr<K, V>>,
    right: Cell<NodePtr<K, V>>,
    parent_color: Cell<usize>,  // color in LSB
    value: V,
}

// BTree: keys/values in MaybeUninit arrays, children, len, leaf flag
pub struct BTreeNode<K, V, const B: usize> {
    len: u16,
    leaf: bool,
    keys: [MaybeUninit<K>; B],
    values: [MaybeUninit<V>; B],
    children: [NodePtr<K, V, B>; B],
}
```

All node types use `Cell` for interior mutability (single-threaded) and raw
pointers (`NodePtr`) for structure links.

## Collection IDs

Each collection instance has a unique ID (thread-local counter, starts at 1,
wraps with zero-skip). Nodes store their owner's ID. This prevents
cross-collection use — a node from List A cannot be inserted into List B.
Checked via `debug_assert!` in mutations.

## Clear Before Drop

**All four collection types require `clear(&slab)` before drop.** The
collections store raw pointers to slab-allocated nodes but don't hold a
slab reference — they can't free nodes on drop.

In debug builds, dropping a non-empty collection panics:

```
List dropped with 5 elements without calling clear().
This leaks slab slots. Call list.clear(&slab) before dropping.
```

In release builds, the slab slots leak silently. The debug panic catches
this during development at zero production cost.

## RbTree Color-in-LSB

The red-black tree stores node color (RED=0, BLACK=1) in the LSB of the
parent pointer. This works because slab-allocated nodes are at least
2-byte aligned (verified at compile time):

```rust
const _: () = assert!(align_of::<RbNode<(), ()>>() >= 2);
```

Parent pointer access masks the color bit:
```rust
let parent = ptr::with_exposed_provenance_mut(packed & !1);  // strip LSB
let color = packed & 1;                                        // extract LSB
```

This saves 8 bytes per node (no separate `color: bool` field with padding).

## Prefetch

RbTree traversals (`find`, `lower_bound`, `upper_bound`, `insert`, `entry`)
prefetch both children before the comparator call:

```rust
let node = unsafe { &*node_deref(current) };
prefetch_read_node(node.left.get());   // may be followed
prefetch_read_node(node.right.get());  // may be followed
match C::cmp(key, &node.key) { ... }
```

One prefetch is wasted (wrong child), but the correct one hides ~30-100
cycles of L2/L3 miss latency. For a 10k-element tree (~14 levels), this
is 14 stalls avoided. x86_64 only (`_mm_prefetch`, `_MM_HINT_T0`).

## Comparator Panic Safety

RbTree and BTree document that a comparator panic during mutation (insert,
remove) may leave the tree in an inconsistent state. Mid-rotation or
mid-split, parent/child pointers are temporarily invalid. A panic at
that point corrupts the structure — subsequent operations are UB.

Callers must ensure their `Compare` implementation does not panic. If
panic safety is needed, use `catch_unwind` around the entire tree
operation and discard the tree on panic (don't reuse it).

## Performance

All operations are measured in cycles, pinned to physical cores:

| Operation | List | Heap | RbTree | BTree |
|-----------|------|------|--------|-------|
| insert | ~60 | ~45 | ~280 | ~240 |
| remove | ~60 | — | ~280 | ~230 |
| pop | ~60 | ~45 | ~22 (first/last) | ~45 (first/last) |
| find/get | — | — | ~30 (entry) | ~40 (entry) |
| clear | O(n) | O(n) | O(n) | O(n) |

RbTree insert includes prefetch. BTree benefits from higher cache utilization
(B keys per node vs 1 key per RbTree node).
