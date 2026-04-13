# Unsafe Code & Soundness Guarantees

This document describes the unsafe code in nexus-collections (466 unsafe
blocks across 4 data structures), the invariants that make each pattern
sound, and miri verification. These collections are the backbone of order
book management, timer wheels, and LRU caches in the trading infrastructure.

---

## Guiding Principles

1. **External storage, internal structure.** Collections don't own their
   nodes — nodes live in a slab. The collection manages pointers between
   nodes (prev/next, parent/child, color). The slab manages allocation.
2. **`clear()` before drop is mandatory.** Collections have no `Drop` impl
   in release (they don't store a slab reference). In debug builds, a
   `#[cfg(debug_assertions)]` Drop panics if non-empty — catching leaks
   during development with zero release cost.
3. **37 miri tests** cover every structural operation: link/unlink,
   rotations, splits, merges, entry API, iteration.

---

## Architecture: Raw Pointer Intrusive Structures

All four collections use raw pointers (`*mut SlotCell<Node<T>>`) as node
handles. The slab allocates nodes, the collection wires them together
via pointer fields inside each node (Cell-wrapped for interior mutability
through shared references).

```
┌──────────┐     ┌──────────┐     ┌──────────┐
│ SlotCell │◄───►│ SlotCell │◄───►│ SlotCell │
│  Node A  │     │  Node B  │     │  Node C  │
│ prev/next│     │ prev/next│     │ prev/next│
└──────────┘     └──────────┘     └──────────┘
      ▲                                 ▲
      │          Collection             │
      └──── head              tail ─────┘
```

Nodes are accessed through `node_deref(ptr)` which returns `&Node<T>`
(List/Heap) or `*const Node<K,V>` (RbTree/BTree). The returned reference
has an unbounded lifetime — it's valid as long as the slab slot is live.

---

## Unsafe Code Categories

### 1. List (list.rs) — 67 unsafe blocks

Doubly-linked list using RC slab nodes. Each `ListNode<T>` has
`prev: Cell<NodePtr>` and `next: Cell<NodePtr>`.

**Operations and their pointer manipulation:**
- `link_back/link_front` — updates 2-4 pointers (head/tail + neighbor links)
- `unlink` — patches prev.next and next.prev around the removed node
- `clear` — walks the list, drops the collection's cloned RC reference
  for each node
- `cursor` — traversal via raw pointer following

**Key invariant:** All pointer updates use `Cell::set` — interior
mutability through `&Node` (shared reference). This is sound because
the collection is `!Sync` (single-threaded). The `Cell` eliminates
the need for `&mut Node`.

**Ownership model:** The list holds CLONED RC references to slab nodes.
The user also holds a reference (the `RcSlot` from `slab.alloc`). The
node is freed when ALL references are dropped — the user must call
`list.clear(&slab)` to release the list's references, then `slab.free(handle)`
to release their own.

### 2. Heap (heap.rs) — 32 unsafe blocks

Pairing heap (min-heap). Each `HeapNode<T>` has `child`, `next`, `prev`
pointer fields for the multi-way tree structure.

**Operations:**
- `push` — links new node as child of root via `link()`
- `pop` — detaches root, runs `merge_pairs` on children
- `merge_pairs` — the core algorithm: left-to-right pairing pass, then
  right-to-left accumulation. Heavy pointer manipulation (6+ pointer
  writes per pair).
- `unlink` — removes a node from its parent's child list

**Key invariant:** Same as List — all through `Cell`, `!Sync`.

**Prefetch:** Not currently applied to heap traversal. The merge_pairs
pass is latency-sensitive but typically operates on <10 nodes.

### 3. RbTree (rbtree.rs) — 126 unsafe blocks

Red-black tree with color packed into the LSB of the parent pointer.

**Color-in-LSB encoding:**
```rust
fn set_color(parent_ptr: *mut SlotCell<RbNode<K,V>>, color: Color) {
    let addr = parent_ptr as usize;
    let new_addr = (addr & !1) | (color as usize);
    // store new_addr as the parent pointer
}
```

**Compile-time assertion:** `align_of::<RbNode<(), ()>>() >= 2` — verified
at compile time. The LSB is always 0 for aligned pointers, so packing
the color bit is safe. `SlotCell` is `repr(C)` with a pointer field first,
giving it at least pointer alignment (8 on 64-bit).

**Rotations (the hardest unsafe code):**
- `left_rotate` — 6 pointer writes (parent, left, right × 2 nodes)
- `right_rotate` — mirror of left_rotate
- `insert_fixup` — up to 2 rotations + color flips, walking up the tree
- `delete_fixup` — up to 3 rotations + color flips

Each rotation maintains all 5 red-black invariants. `verify_invariants()`
checks them in debug builds.

**Panic safety:** If `Compare::cmp()` panics during insert/remove, the
tree may have partially-updated pointers. This is documented on the struct:
"Subsequent operations on a corrupted tree are undefined behavior." The
caller is responsible for ensuring their comparator does not panic.

**Prefetch:** Both children are prefetched before the comparison in
`find()`, `lower_bound`, `upper_bound`, and all insertion traversals.
This hides the L1/L2 miss latency of the dependent pointer chase.

### 4. BTree (btree.rs) — 241 unsafe blocks

B-tree with `MaybeUninit<K>` and `MaybeUninit<V>` arrays in each node.
The most unsafe-dense code in the workspace.

**MaybeUninit arrays:**
Each `BTreeNode<K, V, B>` has:
- `keys: [MaybeUninit<K>; B]` — up to B-1 keys are live
- `values: [MaybeUninit<V>; B]` — matching values
- `children: [NodePtr; B]` — up to B child pointers
- `len: u16` — number of live keys

**Critical operations:**
- `split_child_core` — splits a full node: copies half the keys/values
  to a new node via `ptr::copy_nonoverlapping` on MaybeUninit arrays.
  The source slots become logically uninitialized (MaybeUninit doesn't
  drop, so no double-free).
- `merge_children` — reverse of split: copies keys/values from sibling
  into target via `ptr::copy_nonoverlapping`.
- `shift_left/shift_right` — moves elements within a node's arrays.
  Uses `ptr::copy` (overlapping regions).
- `take_kv` — `assume_init_read()` on a key/value pair. The MaybeUninit
  slot becomes logically uninitialized.

**OccupiedEntry::remove** — uses `assume_init_read()` to extract the
key, wraps in `ManuallyDrop`, then re-searches the tree to call
`remove_found`. The re-search reads the same MaybeUninit slot again.
This is sound because MaybeUninit doesn't propagate drops — the
ManuallyDrop wrapper prevents the first copy from being dropped, and
`remove_found` moves the value out properly. Documented with a
TODO(perf) noting that storing the path would avoid the O(log n)
re-traverse.

### 5. Collection ID (lib.rs) — minimal unsafe

`next_collection_id()` uses `wrapping_add(1)` with skip-zero. IDs are
per-thread, wrap after `usize::MAX` allocations (~18 quintillion on
64-bit). No unsafe — just a `Cell<usize>` counter.

---

## Debug-Mode Leak Detection

All four collections panic in debug builds if dropped non-empty:

```rust
#[cfg(debug_assertions)]
impl<T> Drop for List<T> {
    fn drop(&mut self) {
        if self.len > 0 && !std::thread::panicking() {
            panic!("List dropped with {} elements without calling clear()", self.len);
        }
    }
}
```

The `!std::thread::panicking()` guard prevents double-panic during
unwinding. Zero cost in release — the entire impl is compiled out.

This catches the most common bug: forgetting to call `clear(&slab)`
before dropping a collection, which permanently leaks slab slots.

---

## Drain Inconsistency (Documented)

`RbTree::drain()` and `BTree::drain()` clear remaining elements on Drop
(the Drain struct holds a slab reference). `Heap::Drain` does NOT —
partially consumed heap drains leave elements in the heap. This is
documented on `Heap::Drain`.

---

## Miri Testing

```bash
MIRIFLAGS="-Zmiri-ignore-leaks" cargo +nightly miri test -p nexus-collections --test miri_tests
```

37 tests, ~8 seconds:

| Category | Tests | What they cover |
|----------|-------|----------------|
| List | 6 | link/unlink, clear, cursor traverse+remove, front/back interleaved, single element, DropTracker |
| Heap | 6 | push/pop sorted + reverse, clear, decrease-key, single element, DropTracker |
| RbTree | 13 | ascending (right-heavy rotations), descending (left-heavy), mixed (all cases), remove leaf/one-child/two-children, stress (30+15+10), clear, iteration, range, entry API, pop first/last, DropTracker |
| BTree | 12 | insert-until-split, cascade split, remove-from-leaf, remove-causing-merge, remove-causing-redistribution, stress (50+25+15), clear, iteration, entry insert+remove, range, pop first/last, DropTracker |

**DropTracker pattern:** Each category includes a test that inserts
`DropTracker(u64)` values, clears the collection, and verifies the
drop count matches exactly. This catches double-free (count too high)
and missed-free (count too low).

---

## Adding New Unsafe Code

1. **Use `Cell` for pointer fields in nodes.** Interior mutability through
   shared references (`&Node`) is the established pattern. Never take
   `&mut Node` — the slab doesn't support exclusive node borrows.

2. **Maintain `verify_invariants()`.** RbTree and BTree have invariant
   checkers. Call them in tests after structural operations. They catch
   rotation/split/merge bugs that miri can't (logic errors vs UB).

3. **Add miri tests for new structural operations.** Each test should:
   - Insert enough elements to trigger the operation
   - Verify the collection's structural invariants
   - Clear and verify DropTracker count

4. **Document panic safety.** If a new operation calls user-provided
   comparators or closures during structural mutation, document what
   happens if they panic.

5. **Run miri:**
   ```bash
   MIRIFLAGS="-Zmiri-ignore-leaks" cargo +nightly miri test -p nexus-collections --test miri_tests
   ```
