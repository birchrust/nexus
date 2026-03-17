# nexus-collections Documentation

Slab-backed intrusive collections with O(1) insert/remove and zero
allocation after init.

## User Guide

- [Overview](overview.md) — Architecture, collection types, ownership model
- [List](list.md) — Doubly-linked list with O(1) operations
- [Heap](heap.md) — Min-heap (pairing heap) with O(1) insert, O(log n) pop
- [RbTree](rbtree.md) — Red-black tree sorted map
- [BTree](btree.md) — B-tree sorted map
- [Macros](macros.md) — Allocator macro generation for collections
