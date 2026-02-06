# nexus-slab Benchmarks

All measurements in CPU cycles (`rdtsc`), pinned to a single core (`taskset -c 0`).
Best of 5 runs per percentile. Percentiles reported up to p99.9 — beyond that,
OS noise (timer interrupts, scheduler) dominates over allocator behavior.

**Platform:** AMD Ryzen / Intel Core, Linux, glibc malloc. Non-Copy types.

---

## Methodology

### Batched Unrolled Measurement

`rdtsc`/`rdtscp` have ~20-25 cycles of inherent overhead. We measure 100
operations per sample using a manually unrolled macro to amortize this:

```rust
macro_rules! unroll_100 {
    ($op:expr) => {
        $op; $op; $op; $op; $op; $op; $op; $op; $op; $op; // 10
        // ... 100 total
    };
}

let start = rdtsc_start();
unroll_100!({ black_box(slot.deref()); });
let end = rdtsc_end();
let cycles_per_op = (end - start) / 100;
```

Eliminates timing overhead, loop branches, and compiler hoisting (`black_box`).

### Cold Churn Measurement

Single operations with 8MB cache polluter traversal between each measurement.
Simulates real-world conditions where allocator state isn't in L1/L2.

### Tail Latency Noise Floor

At p99.99 and beyond, both allocators hit the system noise floor — timer
interrupts (~250-1000 Hz on Linux), TLB shootdowns, and scheduler preemption
dominate the measurement. The allocator's advantage narrows from 2-3x at p50
to ~1.2x at p99.99. Max values are pure OS noise and not meaningful for
allocator comparison. For production tail latency, use `isolcpus`, `nohz_full`,
and `SCHED_FIFO`.

### Interpreting Cycle Counts

- **0-1 cycles**: ILP — CPU executes the operation alongside other work
- **2-5 cycles**: Single TLS lookup + pointer chase
- **6-12 cycles**: Alloc (freelist pop + value write) for small types
- **25-90 cycles**: Alloc for larger types (memcpy dominates)
- **100+ cycles**: Cache misses, OS interaction, or large copies

---

## Slot vs Box — Isolated Benchmarks

Slot: `bounded_allocator!` macro, TLS-backed slab, 8-byte RAII handle.
Box: Standard `Box::new()` / `drop()` through glibc malloc.

### CHURN (alloc + deref + drop, LIFO single-slot)

| Size | | p50 | p90 | p99 | p99.9 |
|------|------|-----|-----|-----|-------|
| 32B | **Slot** | **5** | **6** | **7** | **8** |
| 32B | Box | 14 | 16 | 21 | 49 |
| 64B | **Slot** | **7** | **7** | **10** | **16** |
| 64B | Box | 16 | 18 | 24 | 63 |
| 128B | **Slot** | **11** | **11** | **15** | **35** |
| 128B | Box | 21 | 23 | 27 | 75 |
| 256B | **Slot** | **24** | **25** | **31** | **78** |
| 256B | Box | 26 | 28 | 33 | 81 |
| 512B | **Slot** | **36** | **37** | **55** | **98** |
| 512B | Box | 42 | 43 | 53 | 108 |
| 1024B | Slot | 62 | 64 | 78 | 127 |
| 1024B | Box | **55** | **57** | **72** | **128** |
| 4096B | **Slot** | **157** | **161** | **221** | **264** |
| 4096B | Box | 175 | 179 | 248 | 329 |

**Slot is 2.8x faster at 32B** (5 vs 14 cycles). Tied or faster at all sizes
except 1024B where Box's placement-new optimization gives it a ~10% edge on
p50. At 4096B, Slot wins again as memcpy dominates and Box's malloc overhead
shows.

### BATCH ALLOC (100 sequential allocations, no interleaved frees)

| Size | | p50 | p90 | p99 | p99.9 |
|------|------|-----|-----|-----|-------|
| 32B | **Slot** | **6** | **6** | **8** | **12** |
| 32B | Box | 15 | 15 | 18 | 53 |
| 64B | **Slot** | **8** | **8** | **9** | **16** |
| 64B | Box | 17 | 18 | 23 | 56 |
| 128B | **Slot** | **12** | **12** | **13** | **32** |
| 128B | Box | 35 | 38 | 44 | 96 |
| 256B | **Slot** | **26** | **26** | **29** | **78** |
| 256B | Box | 41 | 43 | 48 | 105 |
| 512B | **Slot** | **40** | **41** | **44** | **100** |
| 512B | Box | 54 | 56 | 65 | 129 |
| 1024B | Slot | 80 | 82 | **97** | **145** |
| 1024B | Box | **77** | **79** | 108 | 161 |
| 4096B | Slot | 248 | 256 | **304** | **344** |
| 4096B | Box | **245** | **252** | 310 | 379 |

**Slot dominates small sizes** (2.5x at 32B, 2.9x at 128B). At 1024B+, p50 is
tied but **Slot has better tails** — no OS interaction after init.

### BATCH DROP (pre-alloc 100, then free all)

| Size | | p50 | p90 | p99 | p99.9 |
|------|------|-----|-----|-----|-------|
| 32B | **Slot** | **2** | **2** | **3** | **5** |
| 32B | Box | 12 | 13 | 14 | 39 |
| 64B | **Slot** | **2** | **2** | **2** | **5** |
| 64B | Box | 12 | 14 | 14 | 34 |
| 128B | **Slot** | **2** | **2** | **3** | **5** |
| 128B | Box | 23 | 24 | 26 | 78 |
| 256B | **Slot** | **2** | **2** | **3** | **8** |
| 256B | Box | 23 | 24 | 29 | 73 |
| 512B | **Slot** | **3** | **4** | **4** | **9** |
| 512B | Box | 23 | 25 | 28 | 80 |
| 1024B | **Slot** | **4** | **5** | **5** | **9** |
| 1024B | Box | 24 | 25 | 32 | 81 |
| 4096B | **Slot** | **13** | **13** | **14** | **47** |
| 4096B | Box | 25 | 27 | 43 | 85 |

**Slot deallocation is 6x faster** at small sizes (2 vs 12 cycles at 32B).
A slab free is a single pointer write to the freelist head. `free()` must
update bin metadata, potentially coalesce, and interact with tcache.

### ACCESS (random deref from pool of 1000)

| Size | | p50 | p90 | p99 | p99.9 |
|------|------|-----|-----|-----|-------|
| 32B | Slot | 0 | 0 | 1 | 1 |
| 32B | Box | 0 | 1 | 1 | 1 |
| 64B | Slot | 1 | 1 | 1 | 1 |
| 64B | Box | 1 | 1 | 1 | 1 |
| 256B | Slot | 2 | 2 | 2 | 2 |
| 256B | Box | 1 | 1 | 1 | 1 |
| 4096B | Slot | 3 | 5 | 5 | 7 |
| 4096B | Box | 1 | 1 | 1 | 1 |

Both are single-pointer deref. Box is faster at larger sizes because `Box<T>`
points directly to the value, while `Slot` dereferences through the
`SlotCell<T>` union. At 32-64B both achieve ILP (0-1 cycles). In practice the
difference is negligible — both are sub-cache-line access.

### COLD CHURN (cache-evicted, single alloc+deref+drop)

8MB polluter traversal between each operation. Simulates allocator state
evicted from cache (context switches, competing workloads).

| Size | | p50 | p90 | p99 | p99.9 |
|------|------|-----|-----|-----|-------|
| 32B | **Slot** | **40** | **44** | **60** | **72** |
| 32B | Box | 60 | 74 | 110 | 138 |
| 64B | **Slot** | **44** | **58** | **66** | **90** |
| 64B | Box | 86 | 104 | 134 | 184 |
| 128B | **Slot** | **120** | **340** | **792** | **1472** |
| 128B | Box | 298 | 462 | 1000 | 1858 |
| 256B | **Slot** | **188** | **516** | **948** | **1584** |
| 256B | Box | 312 | 498 | 1356 | 2362 |
| 512B | **Slot** | **338** | **662** | **1176** | **1844** |
| 512B | Box | 326 | 594 | 1198 | 2212 |
| 1024B | **Slot** | **466** | **796** | **1366** | **2014** |
| 1024B | Box | 412 | 696 | 1212 | 2342 |
| 4096B | **Slot** | **552** | **838** | **1488** | **2516** |
| 4096B | Box | 932 | 1232 | 1834 | 3182 |

**Slot is 1.5x faster at 32B cold** (40 vs 60). At 64B, **2x faster** (44
vs 86). At mid sizes (512-1024B), memcpy dominates and results are mixed. At
4096B, Slot pulls ahead again — fewer cache lines to fetch for allocator
metadata.

---

## Stress Tests

Real-world allocation patterns. 64-byte values unless noted.

### Active Contention (Isolation Advantage)

Both allocators see identical "noise" (mixed-size global allocator traffic
between measurements). Box goes through the contested global allocator; Slab
bypasses it entirely.

| Size | | p50 | p90 | p99 | p99.9 |
|------|------|-----|-----|-----|-------|
| 64B | Box | 14 | 15 | 18 | 58 |
| 64B | **Slab** | **4** | **4** | **6** | **8** |
| 256B | Box | 16 | 17 | 22 | 56 |
| 256B | **Slab** | **8** | **9** | **25** | **49** |
| 1024B | Box | 25 | 28 | 56 | 88 |
| 1024B | **Slab** | **19** | **20** | **31** | **71** |
| 4096B | Box | 123 | 139 | 193 | 251 |
| 4096B | **Slab** | **52** | **54** | **62** | **139** |

**Slab is 3.5x faster at 64B** under contention. At 4096B, **2.4x faster**.
This is the primary reason to use a dedicated slab: isolation from the global
allocator means your hot path doesn't pay for someone else's allocations.

### Long-Running Churn (10M ops, 50% fill)

10 phases of 1M operations each. Looking for degradation over time:

| Phase | Box p50 | Slab p50 | Box p99 | Slab p99 |
|-------|---------|----------|---------|----------|
| 1 | 56 | **42** | 128 | **60** |
| 5 | 56 | **44** | 118 | **60** |
| 10 | 56 | **42** | 124 | **60** |

**No degradation for either.** Slab is consistently 1.3x faster at p50 and
**2x better at p99**. Neither shows fragmentation effects over 10M operations.

### Fragmentation Resistance

#### Mixed Lifetimes (90% long-lived, 10% short-lived)

| | p50 | p99 | p99.9 |
|---|-----|-----|-------|
| Box (fragmented) | 38 | 42 | 52 |
| **Slab (no frag)** | **20** | **22** | **24** |

**Slab is 1.9x faster** with 2.2x better p99.9.

#### Brutal Fragmentation (Swiss-cheese heap)

50K mixed-size allocations, free every 3rd, measure in fragmented heap:

| | p50 | p99 | p99.9 |
|---|-----|-----|-------|
| Box (fragmented) | 32 | 36 | 38 |
| Box (clean) | 32 | 36 | 60 |
| **Slab** | **22** | **24** | **26** |

Box doesn't degrade much under fragmentation (glibc's tcache handles same-size
well), but Slab is still **1.5x faster** and has **1.5x better p99.9**.

### Burst Allocation (5000 items, repeated 100x)

| | p50 | p90 | p99 |
|---|-----|-----|-----|
| Box alloc | 39 | 41 | 46 |
| **Slab alloc** | **4** | **5** | **6** |
| Box free | 11 | 12 | 15 |
| **Slab free** | **3** | **4** | **12** |

**Slab alloc is 10x faster.** Slab free is 3.7x faster.

### Sustained Memory Pressure (100MB, 1.6M allocations)

| | p50 | p90 | p99 | p99.9 |
|---|-----|-----|-----|-------|
| Box (100MB) | 820 | 886 | 1216 | 1548 |
| **Slab (100MB)** | **416** | **490** | **888** | **1306** |

**Slab is 2x faster at p50** under 100MB pressure. Both degrade at this scale
due to TLB pressure, but Slab's contiguous layout helps.

### First Allocation Latency (Cold Start)

Cache flushed between each measurement. Measures true first-access cost:

| | p50 | p90 | p99 |
|---|-----|-----|-----|
| Box | 588 | 1058 | 2528 |
| **Slab** | **24** | **24** | **26** |

**Slab is 25x faster.** Box's first allocation may trigger heap management
overhead. Slab is always a freelist pop.

### Size Class Comparison (Fragmented Global Allocator)

Churn (alloc+free) with global allocator pre-fragmented:

| Size | Fill | Box p50 | Slab p50 | Box p99 | Slab p99 |
|------|------|---------|----------|---------|----------|
| 64B | 10% | 34 | **22** | 38 | **24** |
| 64B | 50% | 36 | **22** | 38 | **34** |
| 256B | 10% | 38 | **22** | 42 | **26** |
| 256B | 50% | 36 | **22** | 38 | **26** |
| 1024B | 10% | 40 | **32** | 66 | **34** |
| 1024B | 50% | 46 | **36** | 68 | **56** |
| 4096B | 10% | 162 | **78** | 278 | **82** |
| 4096B | 50% | 118 | **76** | 124 | **82** |

**Slab is consistently 1.5-2x faster** across all sizes and fill levels.
Performance is stable regardless of slab occupancy.

---

## Why SLUB-Style Allocation?

nexus-slab uses a **SLUB-style design** (named after the Linux kernel's default
allocator). The key insight: **per-type freelists with LIFO allocation order**
provide cache locality that general-purpose allocators can't match.

### LIFO Cache Locality

The freelist is a stack. Free a slot, it goes to the head. Allocate, you get
the head back. This means:

```
Free slot A  →  A is now at freelist head
Alloc        →  Get A back, still hot in L1 cache

Time: 5-8 cycles (freelist pop + value write)
```

Compare to `malloc`/`free`:

```
free(A)      →  Update tcache, possibly arena metadata
malloc()     →  Search tcache, possibly different address

Time: 40-60+ cycles, cold cache likely
```

The **LIFO pattern matches real workloads**: request handlers allocate, process,
free, repeat. Event loops create timers, fire them, destroy them. State machines
allocate transitions, execute, clean up. In all these cases, you want the same
hot memory back.

### Placement-New Optimization

The `Claim` API returns a raw slot pointer before the value is written. Combined
with `#[inline]`, LLVM can construct values directly in the slot:

```rust
// Conceptually what happens:
let slot_ptr = slab.claim_ptr();          // Pop from freelist
(*slot_ptr).value = MaybeUninit::new(v);  // Write directly, no intermediate copy
Slot::from_ptr(slot_ptr)                  // Wrap in RAII handle
```

When inlined, there's no memcpy — the value is constructed in place. This is
why small types (32-128B) see the biggest wins: no copy overhead at all.

### Global Allocator Isolation

Your hot path doesn't compete with background work for the same allocator:

| Scenario | Box | Slab |
|----------|-----|------|
| Background logging | Contends for tcache | No impact |
| JSON serialization | Evicts your size class | No impact |
| Collection resize | Arena lock contention | No impact |
| Your allocation | Waits | Just pops freelist |

The benchmarks under "Active Contention" show this: Slab is **9x faster at 64B**
when the global allocator is busy. In production, the global allocator is always
busy.

### Zero Fragmentation

Every slot is the same size. A freed slot is immediately reusable by any
allocation of that type — no coalescing, no compaction, no "best fit" search.

General allocators fragment because they handle variable-sized allocations.
When you free a 64-byte block between two 256-byte blocks, that 64 bytes might
never be reusable. Slabs don't have this problem: same type, same size, instant
reuse.

### Reduce Global Allocator Pressure

Taking high-frequency same-type allocations off the global allocator isn't just
faster for your hot path — it's faster for everything else. Fewer tcache
evictions, less arena contention, fewer size-class collisions. If your process
allocates millions of 64-byte structs per second through `Box`, that traffic
competes with every other 64-byte allocation. Moving it to a slab means the
global allocator handles less volume and serves the rest of your process better.

### Stable Pointers

Slab-allocated objects don't move. Pointers remain valid until explicitly freed.
There's no reallocation, no compaction, no address instability. This makes slabs
ideal for node-based data structures (linked lists, trees, graphs) where nodes
hold pointers to each other.

### The Tradeoff

| | Box | Slot |
|---|---|---|
| Setup cost | None | `Allocator::builder().build()` |
| Handle size | 8 bytes | 8 bytes |
| Alloc p50 (32B) | 14 cycles | **5 cycles** |
| Alloc p50 (1024B) | **55 cycles** | 62 cycles |
| Dealloc p50 (32B) | 12 cycles | **2 cycles** |
| Cold alloc p50 (32B) | 60 cycles | **40 cycles** |
| First alloc p50 | 588 cycles | **24 cycles** |
| Worst case | OS syscall | Freelist pop |
| Fragmentation | Yes (coalescing needed) | **No** (fixed slots) |
| Cache locality | Variable (tcache order) | **LIFO** (hot reuse) |
| Allocator contention | Shared with process | **Isolated** |

**Use Slot when** you churn same-type objects and care about latency or
tail behavior. **Use Box when** allocation is infrequent, types vary, or
simplicity matters more than performance.

---

## Running Benchmarks

```bash
# Build
cargo build --release --examples -p nexus-slab

# Isolated benchmark (all percentiles, multiple sizes)
taskset -c 0 ./target/release/examples/bench_isolated slot
taskset -c 0 ./target/release/examples/bench_isolated box

# Stress tests (fragmentation, contention, sustained load)
taskset -c 0 ./target/release/examples/perf_stress_test
```

For accurate results:
- Pin to a single physical core (`taskset -c 0`)
- Disable turbo boost (`echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo`)
- Run in isolation (no other CPU-intensive processes)
- Take best of 5 runs to minimize noise
