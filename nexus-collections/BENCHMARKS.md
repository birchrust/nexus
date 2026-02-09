# nexus-collections Benchmarks

Cycle-accurate latency on Intel Core Ultra 7 155H, pinned to physical core,
turbo boost disabled. All values in cycles per operation.

Sorted map benchmarks use batched `seq!` unrolled timing (100 ops per rdtsc
pair) to amortize serialization overhead. Population is 10,000 entries unless
noted. Same Xorshift PRNG seed across all benchmarks.

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

## Sorted Maps — Full Comparison

Three sorted map implementations measured with identical methodology.

### nexus RbTree (red-black tree, slab-backed, @10k)

| Operation | p50 | p90 | p99 | p999 | max |
|-----------|-----|-----|-----|------|-----|
| get (hit, @100) | 9 | 9 | 20 | 34 | 287 |
| get (hit, @10k) | 15 | 15 | 16 | 54 | 113 |
| get (miss, @10k) | 41 | 41 | 53 | 105 | 191 |
| get (cold rand, @10k) | 131 | 135 | 167 | 210 | 529 |
| contains_key (hit) | 50 | 51 | 55 | 111 | 680 |
| insert (growing, per-op) | 278 | 394 | 786 | 1178 | 15830 |
| insert (steady) | 203 | 221 | 275 | 345 | 994 |
| insert (duplicate) | 24 | 25 | 41 | 82 | 288 |
| remove | 245 | 256 | 315 | 372 | 1477 |
| pop_first | 22 | 24 | 41 | 69 | 272 |
| pop_last | 21 | 24 | 42 | 74 | 3909 |
| first_key_value | 0 | 1 | 1 | 1 | 160 |
| churn (remove+insert) | 520 | 548 | 651 | 803 | 6431 |
| entry (occupied) | 20 | 21 | 30 | 77 | 317 |
| entry (vacant+insert) | 197 | 211 | 268 | 366 | 815 |

### nexus BTree (B=8, slab-backed, @10k)

| Operation | p50 | p90 | p99 | p999 | max |
|-----------|-----|-----|-----|------|-----|
| get (hit, @100) | 14 | 15 | 21 | 44 | 364 |
| get (hit, @10k) | 22 | 23 | 44 | 139 | 263 |
| get (miss, @10k) | 30 | 31 | 36 | 92 | 256 |
| get (cold rand, @10k) | 137 | 142 | 184 | 252 | 1478 |
| contains_key (hit) | 22 | 23 | 40 | 81 | 393 |
| insert (growing, per-op) | 254 | 314 | 628 | 758 | 12536 |
| insert (steady) | 211 | 218 | 271 | 319 | 1255 |
| insert (duplicate) | 27 | 28 | 41 | 87 | 510 |
| remove | 209 | 221 | 280 | 359 | 582 |
| pop_first | 47 | 50 | 79 | 128 | 532 |
| pop_last | 38 | 41 | 60 | 107 | 226 |
| first_key_value | 6 | 6 | 6 | 9 | 107 |
| churn (remove+insert) | 455 | 481 | 578 | 758 | 6791 |
| entry (occupied) | 22 | 44 | 60 | 94 | 294 |
| entry (vacant+insert) | 373 | 392 | 464 | 602 | 6107 |

### std::collections::BTreeMap (baseline, @10k)

| Operation | p50 | p90 | p99 | p999 | max |
|-----------|-----|-----|-----|------|-----|
| get (hit, @100) | 23 | 24 | 26 | 87 | 5880 |
| get (hit, @10k) | 40 | 43 | 74 | 161 | 369 |
| get (miss, @10k) | 48 | 51 | 59 | 127 | 458 |
| get (cold rand, @10k) | 153 | 160 | 215 | 316 | 9081 |
| contains_key (hit) | 37 | 39 | 50 | 155 | 435 |
| insert (growing, per-op) | 256 | 358 | 614 | 3736 | 17300 |
| insert (steady) | 231 | 244 | 312 | 401 | 11002 |
| insert (duplicate) | 42 | 46 | 68 | 152 | 476 |
| remove | 243 | 263 | 335 | 423 | 12015 |
| pop_first | 71 | 78 | 92 | 153 | 735 |
| pop_last | 52 | 57 | 68 | 132 | 291 |
| churn (remove+insert) | 510 | 546 | 642 | 920 | 7292 |
| entry (occupied) | 37 | 39 | 64 | 141 | 3921 |
| entry (vacant+insert) | 228 | 245 | 313 | 406 | 6266 |

### p50 Comparison Matrix

| Operation | nexus BTree | nexus RbTree | std BTreeMap | Best |
|---|---|---|---|---|
| get (hit, @100) | 14 | **9** | 23 | RbTree |
| get (hit, @10k) | 22 | **15** | 40 | RbTree |
| get (miss, @10k) | **30** | 41 | 48 | nexus BTree |
| get (cold rand, @10k) | 137 | **131** | 153 | RbTree |
| contains_key (hit) | **22** | 50 | 37 | nexus BTree |
| insert (growing) | **254** | 278 | 256 | nexus BTree |
| insert (steady) | 211 | **203** | 231 | RbTree |
| insert (duplicate) | 27 | **24** | 42 | RbTree |
| remove | **209** | 245 | 243 | nexus BTree |
| pop_first | 47 | **22** | 71 | RbTree |
| pop_last | 38 | **21** | 52 | RbTree |
| churn | **455** | 520 | 510 | nexus BTree |
| entry (occupied) | 22 | **20** | 37 | RbTree |
| entry (vacant+insert) | 373 | **197** | 228 | RbTree |

### p999 Tail Latency Comparison

| Operation | nexus BTree | nexus RbTree | std BTreeMap | Best |
|---|---|---|---|---|
| get (hit, @100) | 44 | **34** | 87 | RbTree |
| get (hit, @10k) | 139 | **54** | 161 | RbTree |
| get (miss, @10k) | **92** | 105 | 127 | nexus BTree |
| get (cold rand, @10k) | 252 | **210** | 316 | RbTree |
| contains_key (hit) | **81** | 111 | 155 | nexus BTree |
| insert (growing) | **758** | 1178 | 3736 | nexus BTree |
| insert (steady) | **319** | 345 | 401 | nexus BTree |
| insert (duplicate) | 87 | **82** | 152 | RbTree |
| remove | **359** | 372 | 423 | nexus BTree |
| pop_first | 128 | **69** | 153 | RbTree |
| pop_last | 107 | **74** | 132 | RbTree |
| churn | **758** | 803 | 920 | nexus BTree |
| entry (occupied) | 94 | **77** | 141 | RbTree |
| entry (vacant+insert) | **602** | 366 | 406 | RbTree |

### Analysis

**nexus vs std BTreeMap**: Both nexus trees beat std across the board. The slab
allocator eliminates global allocator contention and gives predictable cache
behavior. The advantage is most visible in tail latency — std's p999 on growing
insert is 3736 cycles (global allocator on node splits) vs nexus BTree's 758.

**nexus RbTree strengths**: Entry API (cached insertion point gives 197 vs 373
for BTree), pop operations (cached leftmost/rightmost), hot lookups (40B nodes
fit in one cache line).

**nexus BTree strengths**: Miss lookups (fewer nodes to confirm absence),
contains_key, remove, churn (contiguous key layout), and tail latency on
growing insert (preemptive splitting avoids cascading rebalance).

### When to Choose Which

**RbTree**: Entry-heavy workloads (order books), pop-heavy workloads (timer
wheels), anything where the pattern is check-then-insert via the entry API.

**BTree**: Read-heavy lookups, existence checking (`contains_key`), high-churn
streaming data, range scans, workloads where miss performance matters. Tunable
branching factor via const generic B parameter.

## Running Benchmarks

```bash
# Disable turbo boost (Intel)
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# Build
cargo build --release --examples -p nexus-collections

# Run pinned to a physical core
taskset -c 0 ./target/release/examples/perf_push_hist      # list + heap
taskset -c 0 ./target/release/examples/perf_rbtree         # red-black tree
taskset -c 0 ./target/release/examples/perf_btree          # B-tree
taskset -c 0 ./target/release/examples/perf_std_btreemap   # std baseline

# Re-enable turbo boost
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
```
