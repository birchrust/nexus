# nexus-collections Benchmarks

Cycle-accurate latency on Intel Core Ultra 7 155H, pinned to physical core,
turbo boost disabled. All values in cycles per operation.

## List (doubly-linked list, RcSlot handles)

| Operation | p50 | p90 | p99 | p999 |
|-----------|-----|-----|-----|------|
| link_back (growing) | 20 | 22 | 24 | 140 |
| link_front (growing) | 20 | 22 | 22 | 28 |
| link_back (steady @25k) | 20 | 22 | 22 | 64 |
| pop_front (drain 50k) | 22 | 22 | 24 | 74 |
| pop_back (drain 50k) | 22 | 22 | 24 | 36 |
| unlink (arb order) | 22 | 22 | 24 | 36 |
| move_to_front | 22 | 24 | 24 | 28 |
| move_to_back | 22 | 24 | 26 | 30 |
| try_push_back (alloc+link) | 22 | 22 | 24 | 26 |

## Heap (pairing heap, RcSlot handles)

| Operation | p50 | p90 | p99 | p999 |
|-----------|-----|-----|-----|------|
| push (growing) | 24 | 24 | 26 | 32 |
| push (steady @25k) | 26 | 38 | 56 | 378 |
| pop (drain 50k) | 312 | 440 | 574 | 1148 |
| unlink (arb order) | 30 | 54 | 146 | 1956 |
| try_push (alloc+link) | 22 | 22 | 24 | 38 |
| peek | 20 | 22 | 22 | 22 |

## SkipList (sorted map, @10k population)

| Operation | p50 | p90 | p99 | p999 |
|-----------|-----|-----|-----|------|
| get (hit, @100) | 27 | 28 | 48 | 98 |
| get (hit, @10k) | 171 | 194 | 280 | 380 |
| get (miss, @10k) | 386 | 404 | 476 | 572 |
| get (cold rand, @10k) | 434 | 458 | 525 | 642 |
| contains_key (hit) | 208 | 212 | 267 | 337 |
| insert (growing) | 864 | 2630 | 4058 | 5434 |
| insert (steady) | 510 | 774 | 982 | 1290 |
| insert (duplicate) | 498 | 770 | 956 | 1236 |
| remove | 544 | 810 | 1022 | 1218 |
| pop_first | 26 | 28 | 54 | 60 |
| pop_last | 556 | 582 | 608 | 816 |
| first_key_value | 0 | 0 | 0 | 1 |
| churn (insert+remove) | 1054 | 1428 | 1764 | 2150 |
| entry (occupied) | 484 | 750 | 932 | 1132 |
| entry (vacant+insert) | 486 | 770 | 994 | 1414 |

## RbTree (red-black tree sorted map, @10k population)

| Operation | p50 | p90 | p99 | p999 |
|-----------|-----|-----|-----|------|
| get (hit, @100) | 8 | 8 | 9 | 13 |
| get (hit, @10k) | 14 | 14 | 15 | 49 |
| get (miss, @10k) | 11 | 11 | 17 | 33 |
| get (cold rand, @10k) | 129 | 133 | 176 | 235 |
| contains_key (hit) | 47 | 48 | 52 | 107 |
| insert (growing) | 280 | 372 | 478 | 594 |
| insert (steady) | 228 | 286 | 330 | 382 |
| insert (duplicate) | 196 | 236 | 272 | 300 |
| remove | 242 | 290 | 342 | 416 |
| pop_first | 26 | 28 | 44 | 66 |
| pop_last | 28 | 28 | 30 | 38 |
| first_key_value | 0 | 0 | 1 | 1 |
| churn (insert+remove) | 466 | 560 | 660 | 848 |
| entry (occupied) | 190 | 232 | 274 | 312 |
| entry (vacant+insert) | 228 | 286 | 326 | 446 |

## Running Benchmarks

```bash
# Disable turbo boost (Intel)
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# Build
cargo build --release --examples -p nexus-collections

# Run pinned to a physical core
taskset -c 0 ./target/release/examples/perf_push_hist    # list + heap
taskset -c 0 ./target/release/examples/perf_skiplist     # skip list
taskset -c 0 ./target/release/examples/perf_rbtree       # red-black tree

# Re-enable turbo boost
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
```
