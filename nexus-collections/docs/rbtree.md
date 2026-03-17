# RbTree

Red-black tree sorted map. O(log n) insert, remove, and lookup.
Self-balancing with guaranteed O(log n) worst case.

## Operations

| Operation | Complexity | Description |
|-----------|-----------|-------------|
| `try_insert(key, value)` | O(log n) | Insert (bounded), returns handle |
| `insert(key, value)` | O(log n) | Insert (unbounded), returns handle |
| `get(key)` | O(log n) | Lookup by key |
| `remove(key)` | O(log n) | Remove by key |
| `first_key_value()` | O(log n) | Smallest key-value pair |
| `last_key_value()` | O(log n) | Largest key-value pair |
| `range(bounds)` | O(log n + k) | Iterator over key range |
| `len()` | O(1) | Current element count |

## Usage

```rust
mod book {
    nexus_collections::rbtree_allocator!(u64, Order, bounded);
}

book::Allocator::builder().capacity(10_000).build().unwrap();
let mut tree = book::RbTree::new(book::Allocator);

// Insert
tree.try_insert(100, Order::new("BTC-USD", 100)).unwrap();
tree.try_insert(50, Order::new("BTC-USD", 50)).unwrap();
tree.try_insert(200, Order::new("BTC-USD", 200)).unwrap();

// Lookup
if let Some(order) = tree.get(&100) {
    println!("{:?}", order);
}

// Range query — all orders between 50 and 150
for (price, order) in tree.range(50..=150) {
    println!("price={price}: {order:?}");
}

// Remove
tree.remove(&100);
```

## Use Cases

- **Order books** — orders sorted by price level
- **Sorted indices** — any key-value store needing ordered traversal
- **Range queries** — efficiently iterate over key ranges

## Entry API

```rust
// Get or insert
tree.entry(100)
    .or_try_insert(Order::default())?;

// Modify existing
tree.entry(100)
    .and_modify(|order| order.qty += 10);
```

## Invariant Verification

In debug builds, call `verify_invariants()` to check the red-black tree
properties (root is black, red nodes have black children, black-height
is uniform):

```rust
#[cfg(debug_assertions)]
tree.verify_invariants();
```
