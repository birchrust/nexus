# BTree

B-tree sorted map with configurable branching factor. O(log n) operations.
Cache-friendly node layout — keys and children are contiguous arrays.

## Operations

Same API as `RbTree`:

| Operation | Complexity |
|-----------|-----------|
| `try_insert(key, value)` | O(log n) |
| `insert(key, value)` | O(log n) |
| `get(key)` | O(log n) |
| `remove(key)` | O(log n) |
| `range(bounds)` | O(log n + k) |

## Usage

```rust
mod index {
    nexus_collections::btree_allocator!(u64, Record, bounded);
}

index::Allocator::builder().capacity(10_000).build().unwrap();
let mut tree = index::BTree::new(index::Allocator);

tree.try_insert(42, Record::new("hello")).unwrap();
```

## When BTree vs RbTree?

| Property | BTree | RbTree |
|----------|-------|--------|
| Node size | Larger (arrays of keys + children) | Smaller (one key + two pointers) |
| Cache behavior | Better (contiguous keys in node) | Worse (pointer chasing) |
| Memory overhead | Higher per node | Lower per node |
| Insert/remove | Splits and merges | Rotations and recoloring |
| Best for | Large datasets, sequential scan | Frequent insert/remove |

For most use cases, RbTree is simpler and sufficient. BTree is better
when cache performance on large datasets matters or when you do many
range scans.

## Branching Factor

The default branching factor `B` determines how many keys each node
holds. Higher B = fewer levels but larger nodes. The macro uses a
sensible default. Custom B values are available via the const generic
on the `BTree` type directly.
