# Heap

Min-heap using a pairing heap structure. O(1) insert, O(1) peek,
O(log n) amortized pop. Slab-backed with `RcSlot` nodes.

## Operations

| Operation | Complexity | Description |
|-----------|-----------|-------------|
| `push(value)` | O(1) | Insert, returns handle |
| `try_push(value)` | O(1) | Fallible insert (bounded slab) |
| `peek()` | O(1) | View the minimum element |
| `pop()` | O(log n) amort. | Remove and return minimum |
| `remove(handle)` | O(log n) amort. | Remove by handle |
| `len()` | O(1) | Current element count |
| `clear()` | O(n) | Remove all elements |

## Usage

```rust
mod timers {
    nexus_collections::heap_allocator!(TimerEntry, bounded);
}

timers::Allocator::builder().capacity(4096).build().unwrap();
let mut heap = timers::Heap::new(timers::Allocator);

// Insert
let h1 = heap.push(TimerEntry { deadline: 100 });
let h2 = heap.push(TimerEntry { deadline: 50 });
let h3 = heap.push(TimerEntry { deadline: 200 });

// Peek — O(1), returns the minimum
let min = heap.peek().unwrap();
assert_eq!(min.value().deadline, 50);

// Pop — O(log n), removes the minimum
let min = heap.pop().unwrap();
assert_eq!(min.value().deadline, 50);

// Cancel by handle — O(log n)
heap.remove(&h3);
```

## Ordering

The heap orders elements using `Ord`. The smallest element is at the
top (min-heap). For max-heap behavior, implement `Ord` with reversed
comparison or use `std::cmp::Reverse`.

## Use Cases

- **Timer wheels** — next-to-fire timer at the top
- **Priority queues** — process highest-priority items first
- **Scheduling** — earliest deadline first

## Drain

```rust
// Pop all elements in order
for node in heap.drain() {
    process(node.value());
}

// Pop while a condition holds
for node in heap.drain_while(|n| n.value().deadline < now) {
    fire(node.value());
}
```

`drain()` consumes all elements. `drain_while()` stops when the
predicate returns false — remaining elements stay in the heap.
