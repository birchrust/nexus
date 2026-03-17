# List

Doubly-linked list with O(1) push, pop, and remove. Backed by slab-allocated
`RcSlot` nodes.

## Operations

| Operation | Complexity | Description |
|-----------|-----------|-------------|
| `push_back(value)` | O(1) | Append to tail, returns handle |
| `push_front(value)` | O(1) | Prepend to head, returns handle |
| `pop_front()` | O(1) | Remove and return head |
| `pop_back()` | O(1) | Remove and return tail |
| `remove(handle)` | O(1) | Remove by handle (any position) |
| `len()` | O(1) | Current element count |
| `is_empty()` | O(1) | Whether the list is empty |
| `clear()` | O(n) | Remove all elements |

## Usage

```rust
mod queue {
    nexus_collections::list_allocator!(Order, bounded);
}

queue::Allocator::builder().capacity(100).build().unwrap();
let mut list = queue::List::new(queue::Allocator);

// Push
let h1 = list.push_back(Order::new(1));
let h2 = list.push_back(Order::new(2));
let h3 = list.push_front(Order::new(0));

// Iterate
for node in list.iter() {
    println!("{:?}", node.value());
}

// Remove by handle — O(1) from any position
list.remove(&h2);

// Pop
let front = list.pop_front();  // returns RcSlot<ListNode<Order>>
```

## Use Cases

- **Order queues** — price level queues in an order book
- **LRU caches** — move accessed items to front, evict from back
- **Timer slots** — pending timers at the same deadline
- **Work queues** — push at back, pop from front

## Moving Between Lists

```rust
let mut active = queue::List::new(queue::Allocator);
let mut archive = queue::List::new(queue::Allocator);

let handle = active.push_back(Order::new(42));

// Move from active to archive — no allocation
active.unlink(&handle);
archive.link_back(&handle);
```

## Cursor

Navigate and modify the list from a specific position:

```rust
let mut cursor = list.cursor_front_mut();
while let Some(node) = cursor.current() {
    if node.value().is_expired() {
        cursor.remove();  // removes current, advances cursor
    } else {
        cursor.move_next();
    }
}
```
