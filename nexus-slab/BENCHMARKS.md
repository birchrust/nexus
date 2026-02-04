# nexus-slab Benchmarks

All measurements in CPU cycles (`rdtsc`), pinned to a single core (`taskset -c 0`).
Best of 5 runs per percentile unless noted otherwise.

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

### Interpreting Cycle Counts

- **0-1 cycles**: ILP -- CPU executes the operation alongside other work
- **2-5 cycles**: Single TLS lookup + pointer chase
- **6-12 cycles**: Alloc (freelist pop + value write) for small types
- **25-90 cycles**: Alloc for larger types (memcpy dominates)
- **100+ cycles**: Cache misses, OS interaction, or large copies

---

## Slot vs Box -- Isolated Benchmarks

Slot: `bounded_allocator!` macro, TLS-backed slab, 8-byte RAII handle.
Box: Standard `Box::new()` / `drop()` through glibc malloc.

### CHURN (alloc + deref + drop, LIFO single-slot)

| Size | | p50 | p90 | p99 | p99.9 | p99.99 |
|------|------|-----|-----|-----|-------|--------|
| 32B | **Slot** | **6** | **6** | **9** | **13** | **82** |
| 32B | Box | 17 | 19 | 21 | 41 | 124 |
| 64B | **Slot** | **8** | **8** | **11** | **22** | **82** |
| 64B | Box | 18 | 20 | 29 | 66 | 107 |
| 128B | **Slot** | **11** | **12** | **14** | **27** | **77** |
| 128B | Box | 23 | 25 | 29 | 64 | 111 |
| 256B | **Slot** | **25** | **27** | **29** | **79** | **107** |
| 256B | Box | 27 | 28 | 34 | 87 | 119 |
| 512B | **Slot** | **40** | **42** | 59 | **110** | 170 |
| 512B | Box | 45 | 46 | **52** | 115 | 163 |
| 1024B | Slot | 67 | 68 | 81 | 136 | 194 |
| 1024B | Box | **60** | **61** | **72** | **136** | **177** |
| 4096B | **Slot** | **168** | **173** | **232** | **279** | **402** |
| 4096B | Box | 188 | 194 | 271 | 335 | 442 |

**Slot is 2.8x faster at 32B** (6 vs 17 cycles). Tied or faster at all sizes
except 1024B where Box's placement-new optimization gives it a ~10% edge on
p50. At 4096B, Slot wins again as memcpy dominates and Box's malloc overhead
shows.

### BATCH ALLOC (100 sequential allocations, no interleaved frees)

| Size | | p50 | p90 | p99 | p99.9 | p99.99 |
|------|------|-----|-----|-----|-------|--------|
| 32B | **Slot** | **6** | **7** | **8** | **13** | **68** |
| 32B | Box | 17 | 17 | 22 | 53 | 102 |
| 64B | **Slot** | **8** | **8** | **10** | **16** | **71** |
| 64B | Box | 19 | 20 | 23 | 63 | 99 |
| 128B | **Slot** | **12** | **12** | **14** | **32** | **85** |
| 128B | Box | 38 | 40 | 44 | 96 | 137 |
| 256B | **Slot** | **27** | **28** | **30** | **78** | **101** |
| 256B | Box | 45 | 47 | 53 | 113 | 174 |
| 512B | **Slot** | **43** | **44** | **59** | **109** | **143** |
| 512B | Box | 58 | 60 | 80 | 133 | 188 |
| 1024B | Slot | 87 | 89 | **98** | **152** | **195** |
| 1024B | Box | **81** | **83** | 114 | 163 | 213 |
| 4096B | Slot | 263 | 271 | **313** | **377** | **547** |
| 4096B | Box | **258** | **265** | 322 | 383 | 562 |

**Slot dominates small sizes** (2.8x at 32B, 3.2x at 128B). At 1024B+, p50 is
tied but **Slot has better tails** -- no OS interaction after init.

### BATCH DROP (pre-alloc 100, then free all)

| Size | | p50 | p90 | p99 | p99.9 |
|------|------|-----|-----|-----|-------|
| 32B | **Slot** | **2** | **2** | **2** | **5** |
| 32B | Box | 11 | 13 | 14 | 46 |
| 64B | **Slot** | **2** | **2** | **3** | **6** |
| 64B | Box | 11 | 12 | 17 | 34 |
| 128B | **Slot** | **2** | **2** | **3** | **6** |
| 128B | Box | 24 | 26 | 30 | 80 |
| 256B | **Slot** | **2** | **2** | **3** | **8** |
| 256B | Box | 25 | 27 | 31 | 73 |
| 512B | **Slot** | **4** | **4** | **5** | **9** |
| 512B | Box | 25 | 27 | 34 | 84 |
| 1024B | **Slot** | **5** | **5** | **6** | **9** |
| 1024B | Box | 25 | 27 | 35 | 85 |
| 4096B | **Slot** | **13** | **14** | **15** | **24** |
| 4096B | Box | 27 | 28 | 44 | 85 |

**Slot deallocation is 5.5x faster** at small sizes (2 vs 11 cycles at 32B).
A slab free is a single pointer write to the freelist head. `free()` must
update bin metadata, potentially coalesce, and interact with tcache.

### ACCESS (random deref from pool of 1000)

| Size | | p50 | p90 | p99 | p99.9 |
|------|------|-----|-----|-----|-------|
| 32B | Slot | 0 | 1 | 1 | 1 |
| 32B | Box | 1 | 1 | 1 | 1 |
| 64B | Slot | 1 | 1 | 1 | 1 |
| 64B | Box | 1 | 1 | 1 | 1 |
| 256B | Slot | 2 | 2 | 2 | 2 |
| 256B | Box | 1 | 1 | 1 | 1 |
| 4096B | Slot | 3 | 5 | 10 | 18 |
| 4096B | Box | 1 | 1 | 1 | 1 |

Both are single-pointer deref. Box is faster at larger sizes because `Box<T>`
points directly to the value, while `Slot` dereferences through the
`SlotCell<T>` union. At 32-64B both achieve ILP (0-1 cycles). In practice the
difference is negligible -- both are sub-cache-line access.

### COLD CHURN (cache-evicted, single alloc+deref+drop)

8MB polluter traversal between each operation. Simulates allocator state
evicted from cache (context switches, competing workloads).

| Size | | p50 | p90 | p99 | p99.9 |
|------|------|-----|-----|-----|-------|
| 32B | **Slot** | **36** | **48** | **68** | **90** |
| 32B | Box | 68 | 82 | 138 | 192 |
| 64B | **Slot** | **44** | **60** | **70** | **114** |
| 64B | Box | 100 | 124 | 176 | 286 |
| 128B | **Slot** | **140** | **378** | **682** | **1290** |
| 128B | Box | 306 | 578 | 1020 | 1522 |
| 256B | **Slot** | **298** | 668 | **1080** | **1646** |
| 256B | Box | 330 | **660** | 1184 | 2004 |
| 512B | Slot | 424 | 792 | 1400 | 2024 |
| 512B | Box | **398** | **754** | **1258** | **1982** |
| 1024B | Slot | 542 | 902 | **1366** | **1856** |
| 1024B | Box | **524** | **816** | 1412 | 2188 |
| 4096B | **Slot** | **626** | **890** | **1584** | **2296** |
| 4096B | Box | 776 | 1128 | 1856 | 2952 |

**Slot is 1.9x faster at 32B cold** (36 vs 68). At 64B, **2.3x faster** (44
vs 100). At mid sizes (512-1024B), memcpy dominates and both converge. At
4096B, Slot pulls ahead again -- fewer cache lines to fetch for allocator
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

| | p50 | p99 | p99.9 | p99.99 |
|---|-----|-----|-------|--------|
| Box (fragmented) | 38 | 42 | 52 | 646 |
| **Slab (no frag)** | **20** | **22** | **24** | **172** |

**Slab is 1.9x faster** with 3.8x better p99.99.

#### Brutal Fragmentation (Swiss-cheese heap)

50K mixed-size allocations, free every 3rd, measure in fragmented heap:

| | p50 | p99 | p99.9 | p99.99 |
|---|-----|-----|-------|--------|
| Box (fragmented) | 32 | 36 | 38 | 166 |
| Box (clean) | 32 | 36 | 60 | 210 |
| **Slab** | **22** | **24** | **26** | **52** |

Box doesn't degrade much under fragmentation (glibc's tcache handles same-size
well), but Slab is still **1.5x faster** and has **3.2x better p99.99**.

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

## Why Use a Slab?

The slab is purpose-built for **constant churn of same-type objects** -- the
pattern where you repeatedly allocate, use, and free the same struct. Orders
in a matching engine. Connections in a server. Nodes in a graph. Timers in a
wheel.

### Predictable latency under churn

Every `Box::new()` goes through the global allocator. That allocator is shared
with every other allocation in your process -- string formatting, logging,
collection resizing, serialization buffers. Under load, these compete for the
same tcache bins, the same arenas, the same locks. A slab is yours alone.
Your hot path doesn't spike because a logger allocated.

### Reduce pressure on the global allocator

Taking high-frequency same-type allocations off the global allocator isn't
just faster for your hot path -- it's faster for everything else. Fewer
tcache evictions, less arena contention, fewer size-class collisions. If your
process allocates millions of 64-byte structs per second through `Box`, that
traffic competes with every other 64-byte allocation. Moving it to a slab
means the global allocator handles less volume and serves the rest of your
process better.

### Stable pointers, zero fragmentation

Slab-allocated objects don't move. Pointers remain valid until explicitly
freed. There's no reallocation, no compaction, no address instability. And
because every slot is the same size, there's no fragmentation -- freed slots
are immediately reusable without coalescing.

### The tradeoff

| | Box | Slot |
|---|---|---|
| Setup cost | None | `Allocator::builder().build()` |
| Handle size | 8 bytes | 8 bytes |
| Alloc p50 (32B) | 17 cycles | **6 cycles** |
| Alloc p50 (1024B) | **60 cycles** | 67 cycles |
| Dealloc p50 (32B) | 11 cycles | **2 cycles** |
| Cold alloc p50 (32B) | 68 cycles | **36 cycles** |
| First alloc p50 | 588 cycles | **24 cycles** |
| Worst case | OS syscall | Freelist pop |
| Fragmentation | Yes | No (fixed slots) |
| Cache locality | Scattered | Contiguous |

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
