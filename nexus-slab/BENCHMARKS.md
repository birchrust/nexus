# nexus-slab Benchmarks

All measurements in CPU cycles using batched unrolled timing (see Methodology below).

**Test conditions:** `taskset -c 0` pinning, best of 5 runs per percentile.

---

## Methodology

### The Problem with Single-Operation Timing

The `rdtsc`/`rdtscp` instructions have ~20-25 cycles of inherent overhead. When measuring operations that complete in 2-8 cycles, single-operation timing gives misleading results—everything appears to take ~22-24 cycles (the measurement floor).

### Solution: Batched Unrolled Measurement

We measure 100 operations per sample using a manually unrolled macro:

```rust
macro_rules! unroll_100 {
    ($op:expr) => {
        $op; $op; $op; $op; $op; $op; $op; $op; $op; $op; // 10
        $op; $op; $op; $op; $op; $op; $op; $op; $op; $op; // 20
        // ... 100 total
    };
}

let start = rdtsc_start();
unroll_100!({ black_box(entry.get()); });
let end = rdtsc_end();
let cycles_per_op = (end - start) / 100;
```

This eliminates:
- **Timing overhead**: Amortized across 100 ops
- **Loop overhead**: No branches, just straight-line code
- **Compiler optimization**: `black_box()` prevents hoisting

### Interpreting Results

- **0-1 cycles**: Instruction-Level Parallelism (ILP) - CPU executes operation "for free" alongside other work
- **2-3 cycles**: Single memory access, no dependent loads
- **4-8 cycles**: Multiple dependent operations
- **8+ cycles**: Multiple indirections, cache misses, or complex operations

---

## Summary (Allocator API p50)

| Operation | Slot API | Key-based | slab crate | Notes |
|-----------|----------|-----------|------------|-------|
| GET (random) | **2** | 2 | 3 | Slot has direct pointer |
| GET (hot) | **0** | - | 1 | ILP - CPU pipelines loads |
| GET_MUT | **2** | 2 | 3 | Slot/key tied |
| CONTAINS | **2** | 3 | 2 | slot.is_valid() |
| INSERT | 8 | - | **5** | slab crate has less indirection |
| REMOVE | **4** | - | 4 | Tied |
| REPLACE | **3** | - | 4 | Slot has direct pointer |

**Key findings:**
- Slot API wins for access patterns (GET, GET_MUT, REPLACE) - direct pointer avoids lookup
- slab crate slightly faster for INSERT due to less indirection
- Hot access shows ILP - CPU pipelines repeated loads (0 cycles at p50)
- 16-byte Slot (with vtable pointer) enables RAII without TLS lookups

---

## GET Operations

### Random Access Pattern

Accessing entries at random indices (realistic workload):

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.get()` | **2** | **2** | **2** | 2 | 20 |
| `get_unchecked()` [unsafe] | 2 | 3 | 3 | 6 | 99 |
| slab crate | 3 | 3 | 5 | 7 | 84 |

Slot's cached pointer saves index lookup.

### Hot Access Pattern

Repeatedly accessing the same entry (measures ILP):

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.get()` | **0** | **0** | **0** | **1** | 10 |
| slab crate | 1 | 2 | 2 | 2 | 24 |

Slot achieves 0 cycles at p50 due to CPU pipelining repeated loads.

### GET_MUT

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.get_mut()` | **2** | **2** | **2** | **2** | 23 |
| `get_unchecked_mut()` [unsafe] | 2 | 3 | 3 | 4 | 78 |
| slab crate | 3 | 4 | 4 | 8 | 96 |

Slot is fastest at all percentiles.

---

## INSERT

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| Allocator API | 8 | 10 | 19 | 20 | 60 |
| slab crate | **5** | **5** | **6** | **10** | 80 |

Slab crate is faster for INSERT because it has less indirection (vtable call). The tradeoff is that nexus-slab provides RAII semantics via the Slot type.

---

## REMOVE

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.into_inner()` | **4** | **5** | 5 | 11 | 39 |
| slab.remove() | **4** | **4** | 6 | 15 | 59 |

Both fast. Slot drops value + frees via embedded vtable pointer.

---

## REPLACE

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.replace()` | **3** | **3** | **3** | **5** | 29 |
| slab get_mut+replace | 4 | 4 | 5 | 11 | 82 |

Slot's cached pointer saves ~1 cycle - no index lookup needed.

---

## CONTAINS

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.is_valid()` | **2** | **2** | **3** | 6 | 23 |
| `contains_key()` | 3 | 3 | 5 | 8 | 36 |
| slab crate | **2** | 3 | 3 | 6 | 35 |

`slot.is_valid()` is fastest - single stamp read.

---

## When to Use What

### Use Allocator API + Slot when:
- You hold slots for repeated access (GET, REPLACE)
- Mutation patterns dominate (REPLACE is faster)
- You need RAII semantics (auto-deallocation on drop)
- You want stable memory addresses

### Use slab crate when:
- INSERT performance is critical (4-6 cycles vs 9)
- You don't need RAII/Slot handles
- Vec reallocation stalls at p99.99 are acceptable

### Unbounded vs Bounded

The unbounded slab adds ~2-4 cycles per operation due to chunk indirection. The tradeoff:

| | Bounded | Unbounded |
|---|---------|-----------|
| GET overhead | +0 | +2-4 cycles |
| Growth behavior | Panics when full | Adds chunks (no copy) |
| Tail latency | Deterministic | No reallocation stalls |

Use bounded when capacity is known. Use unbounded when growth is needed without `Vec` reallocation spikes.

---

## vs Box (Heap Allocation)

The real comparison for many use cases isn't slab crate vs nexus-slab—it's **pre-allocated slab vs heap allocation**. When should you use `Slot<T>` instead of `Box<T>`?

### Full Distribution Comparison

Stress tests measuring 64-byte struct allocation under various conditions:

#### Burst Allocation (5000 items, repeated 100x)

| | p25 | p50 | p75 | p90 | p99 |
|---|---|---|---|---|---|
| Box alloc | 18 | 18 | 19 | 19 | 25 |
| **Slab alloc** | **13** | **13** | **23** | **27** | **79** |
| Box free | 12 | 12 | 13 | 13 | 16 |
| **Slab free** | **5** | **5** | **5** | **5** | **7** |

**Slab deallocation is 2.4x faster**.

#### Long-Running Churn (10M operations, 50% fill)

| | p25 | p50 | p75 | p90 | p99 | p99.9 |
|---|---|---|---|---|---|---|
| Box | 46 | 62 | 72 | 80 | 136 | 214 |
| **Slab** | **44** | **58** | **66** | **72** | **82** | **110** |

**Slab has 1.7x better p99 and 2x better p99.9** under sustained churn.

#### First Allocation Latency (Cold Start)

This is the killer test. After exhausting the thread cache with large allocations, measure the first 64-byte allocation:

| | p25 | p50 | p75 | p90 | p99 |
|---|---|---|---|---|---|
| Box | 176 | 196 | 250 | 868 | 1656 |
| **Slab** | **28** | **32** | **32** | **34** | **42** |

**Slab is 6x faster at p50, 39x faster at p99** when the heap allocator has to do real work.

#### Tail Latency (1M samples)

| | p25 | p50 | p75 | p90 | p99 | p99.9 | p99.99 | max |
|---|---|---|---|---|---|---|---|---|
| Box | 38 | 38 | 40 | 42 | 46 | 74 | 296 | 234,206 |
| **Slab** | **32** | **32** | **34** | **34** | **40** | **62** | **96** | **10,714** |

**Slab's worst case is 22x better** (no OS interaction after init).

### When to Use Slab vs Box

**Use Slab when:**
- You have **churning data** (frequent insert/remove cycles)
- **Tail latency matters** (trading, games, real-time systems)
- You need **stable memory addresses** (node-based data structures)
- You want **predictable worst-case** (no mmap/brk syscalls after init)
- Memory is **pre-allocatable** at startup

**Use Box when:**
- Allocation is **infrequent** (one-time setup)
- You need **dynamic sizing** (different types, unknown sizes)
- **Simplicity** is more important than performance
- You're **not latency-sensitive**

### The Tradeoff

| | Box | Slab |
|---|---|---|
| Setup cost | None | `Allocator::builder().build()` required |
| Allocation p50 | ~18 cycles | ~13 cycles |
| Deallocation p50 | ~12 cycles | ~5 cycles |
| Worst case | OS syscall (234K cycles) | Freelist pop (10K cycles) |
| Memory overhead | Per-allocation metadata | 8 bytes/slot + stamp |
| Fragmentation | Yes (over time) | No (fixed slots) |
| Cache locality | Scattered | Contiguous |

**Bottom line:** If you're allocating/deallocating the same type frequently and care about latency, slab wins decisively. The 2.4x faster deallocation and 22x better worst-case make it the clear choice for performance-critical paths.

---

## Allocator Isolation (Active Contention)

The most important advantage of a dedicated slab is **isolation from the global allocator**. In production, `Box::new()` shares malloc with every other allocation in your process. The slab is yours alone.

### Test Design

Both Box and Slab run with identical "noise" between measurements:
- 20-80 mixed-size allocations (32B-2KB) to the global allocator
- Swiss-cheese free pattern (keep 50-75%)
- Then measure a batch of 10-100 allocations

Box goes through the (equally warm) global allocator. Slab bypasses it entirely.

### Hot Cache Results (Best of 5 isolated runs)

Steady-state performance with warm caches:

| Size | Alloc | p25 | p50 | p75 | p90 | p99 | p99.9 |
|------|-------|-----|-----|-----|-----|-----|-------|
| 64B | Box | 12 | 13 | 14 | 15 | 19 | 29 |
| 64B | **Slab** | **8** | **8** | **9** | **10** | **11** | **15** |
| 256B | Box | 15 | 16 | 16 | 17 | 48 | 53 |
| 256B | **Slab** | **16** | **16** | **17** | **17** | **20** | **49** |
| 1024B | Box | 24 | 25 | 27 | 35 | 66 | 102 |
| 1024B | **Slab** | **24** | **24** | **24** | **25** | **32** | **83** |
| 4096B | Box | 101 | 108 | 114 | 130 | 162 | 229 |
| 4096B | **Slab** | **59** | **59** | **60** | **62** | **81** | **152** |

**Hot cache findings:**
- **64B**: Slab 1.6x faster at p50 (8 vs 13), 1.7x better at p99 (11 vs 19)
- **256B**: Tied at p50, but slab 2.4x better at p99 (20 vs 48)
- **1024B**: Tied at p50, 2.1x better at p99 (32 vs 66)
- **4096B**: Slab **1.8x faster** at p50 (59 vs 108), 2.0x better at p99 (81 vs 162)

### Cold Cache Results

Two scenarios measured with 24MB eviction buffer (2x L3), strided access pattern, interleaved measurement:

#### Single-Op Cold (True First-Access Latency)

One alloc+free per cache eviction. Measures the **first operation** after cache pressure—what you pay after a context switch or when allocator state has been evicted.

| Size | Box p50 | Slab p50 | Box p99 | Slab p99 |
|------|---------|----------|---------|----------|
| 64B | 158 | **84** | 300 | **154** |
| 256B | 168 | **108** | 314 | **176** |

Note: Includes ~20-25 cycle rdtsc overhead.

**Single-op findings:**
- **64B**: Slab **1.9x faster** at p50, **1.9x better** at p99
- **256B**: Slab **1.6x faster** at p50, **1.8x better** at p99

#### Batched Cold (Burst After Cache Eviction)

10 operations per cache eviction. Measures burst allocation patterns where only the first op is truly cold.

| Size | Alloc | p25 | p50 | p75 | p90 | p99 | p99.9 |
|------|-------|-----|-----|-----|-----|-----|-------|
| 64B | Box | 48 | 51 | 54 | 57 | 71 | 140 |
| 64B | Slab | 48 | 52 | 54 | 57 | **65** | **117** |
| 256B | Box | 55 | 58 | 61 | 65 | 94 | 247 |
| 256B | Slab | 59 | 62 | 65 | 68 | **80** | **186** |
| 4096B | Box | 177 | 181 | 186 | 193 | 270 | 568 |
| 4096B | **Slab** | **116** | **120** | **124** | **128** | **166** | **227** |

**Batched findings:**
- **64B/256B**: Roughly tied at p50 (first op cold, rest warm). Slab wins at p99.
- **4096B**: Slab **1.5x faster** at p50, **1.6x better** at p99

#### Why the Difference?

In batched measurement, ops 2-10 run with warm metadata from op 1. Box's tcache is highly optimized for warm small-object free, which masks Slab's cold-start advantage.

Single-op measurement shows true cold performance where Slab's simpler data structures (fewer cache lines to fetch) win consistently.

### Why Slab Wins

The global allocator (glibc malloc, jemalloc, etc.) must:
- Handle any size class
- Maintain per-thread caches that other code touches
- Occasionally interact with the OS (mmap, brk)

The slab:
- Is dedicated to one type
- Has its own freelist that nothing else touches
- Never interacts with the OS after init

**The cold cache advantage is particularly important**: In real systems, your allocator state won't always be in L1/L2 cache. The slab's simpler data structures mean fewer cache lines to fetch on the critical path.

### Benchmark Isolation Warning

**Important**: Run benchmarks in isolation or as the first test in a suite. Memory-intensive tests that run before can pollute cache/TLB state and skew results.

For accurate comparisons:
```bash
# Hot cache (steady-state performance)
taskset -c 0 ./target/release/examples/minimal_contention

# Cold cache - batched (burst allocation pattern)
taskset -c 0 ./target/release/examples/minimal_contention_cold

# Cold cache - single-op (true first-access latency)
taskset -c 0 ./target/release/examples/cold_single_op
```

---

## Warming the Global Allocator

If you're comparing against Box fairly, warm the global allocator first to avoid cold-start penalties:

```rust
/// Warm the global allocator by pre-faulting pages across size classes
fn warm_allocator() {
    for size in [64, 128, 256, 512, 1024, 2048, 4096] {
        let chunks: Vec<Vec<u8>> = (0..1000)
            .map(|_| vec![0u8; size])
            .collect();
        // Touch each allocation to fault pages
        for chunk in &chunks {
            std::hint::black_box(&chunk[0]);
        }
        drop(chunks);
    }
}
```

This pre-faults pages and warms tcache for common size classes. Without warming, Box's first allocations may trigger mmap syscalls.

---

## Running Benchmarks

### Prerequisites

```bash
# Disable turbo boost (Intel)
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# Or for AMD
echo 0 | sudo tee /sys/devices/system/cpu/cpufreq/boost
```

### Run Benchmarks

```bash
cargo build --release --examples

# Allocator API vs slab crate (unrolled methodology)
taskset -c 0 ./target/release/examples/perf_full_distribution

# Slab vs Box stress tests (churn, fragmentation, tail latency)
taskset -c 0 ./target/release/examples/perf_stress_test

# Quick Box comparison
taskset -c 0 ./target/release/examples/perf_vs_box
```

### Re-enable Turbo Boost

```bash
# Intel
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# AMD
echo 1 | sudo tee /sys/devices/system/cpu/cpufreq/boost
```
