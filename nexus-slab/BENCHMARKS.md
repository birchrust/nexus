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
- **4-8 cycles**: Multiple dependent operations or TLS lookup
- **8+ cycles**: Multiple indirections, cache misses, or complex operations

---

## Summary (Macro API p50)

| Operation | Slot API | Key-based | slab crate | Notes |
|-----------|----------|-----------|------------|-------|
| GET (random) | **2** | 3 | 3 | Slot has direct pointer |
| GET (hot) | **1** | - | 2 | ILP - CPU pipelines loads |
| GET_MUT | **2** | 2 | 3 | Slot/key tied |
| CONTAINS | 2 | 3 | **2** | slot.is_valid() fastest |
| INSERT | 8 | - | **4** | slab wins - no TLS |
| REMOVE | 4 | - | **3** | slab slightly faster |
| REPLACE | **2** | - | 4 | Slot has direct pointer |

**Key findings:**
- Slot API wins for access patterns (GET, GET_MUT, REPLACE) - direct pointer avoids lookup
- TLS lookup adds ~3-4 cycles to INSERT vs slab crate
- Hot access shows ILP - CPU pipelines repeated loads to same address
- 8-byte Slot size (vs 16+ for handle-based designs) improves cache efficiency

---

## GET Operations

### Random Access Pattern

Accessing entries at random indices (realistic workload):

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.get()` | **2** | **2** | **2** | 3 | 56 |
| `get_unchecked()` [unsafe] | 3 | 3 | 3 | 4 | 67 |
| slab crate | 3 | 4 | 4 | 5 | 48 |

Slot's cached pointer saves index lookup.

### Hot Access Pattern

Repeatedly accessing the same entry (measures ILP):

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.get()` | **1** | **1** | **1** | **1** | 10 |
| slab crate | 2 | 2 | 2 | 2 | 27 |

Slot achieves 1 cycle at p50 due to CPU pipelining repeated loads.

### GET_MUT

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.get_mut()` | **2** | **2** | **2** | **2** | 68 |
| `get_unchecked_mut()` [unsafe] | **2** | 3 | 4 | 4 | 44 |
| slab crate | 3 | 3 | 4 | 8 | 47 |

Slot is fastest at all percentiles.

---

## INSERT

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| Macro API | 8 | 11 | 16 | 20 | 77 |
| slab crate | **4** | **5** | **6** | **9** | 115 |

Slab crate wins by ~4 cycles. The macro API has TLS lookup overhead (~3-4 cycles) that slab crate avoids with direct struct access. This is the tradeoff for 8-byte slots.

---

## REMOVE

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.into_inner()` | 4 | 5 | 6 | 11 | 53 |
| slab crate | **3** | **3** | 5 | 13 | 46 |

Both fast. Slot drops value + frees via TLS; slab just marks vacant.

---

## REPLACE

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.replace()` | **2** | **3** | **3** | **4** | 33 |
| slab get_mut+replace | 4 | 4 | 5 | 10 | 65 |

Slot's cached pointer saves ~2 cycles - no index lookup needed.

---

## CONTAINS

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `slot.is_valid()` | **2** | **2** | **2** | **2** | 20 |
| `contains_key()` | 3 | 3 | 3 | 4 | 42 |
| slab crate | **2** | **2** | 3 | 6 | 43 |

`slot.is_valid()` is fastest - single stamp read, no TLS lookup.

---

## When to Use What

### Use Macro API + Slot when:
- You hold slots for repeated access (GET, REPLACE)
- Mutation patterns dominate (REPLACE is 2x faster)
- You need 8-byte handles (cache efficiency, smaller data structures)

### Use slab crate when:
- INSERT performance is critical (4 cycles vs 8)
- You don't need Slot handles
- Vec reallocation stalls at p99.99 are acceptable

### Unbounded vs Bounded

The unbounded slab adds ~2-4 cycles per operation due to chunk indirection. The tradeoff:

| | Bounded | Unbounded |
|---|---------|-----------|
| GET overhead | +0 | +2-4 cycles |
| Growth behavior | Fails when full | Adds chunks (no copy) |
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
| Box alloc | 42 | 43 | 44 | 45 | 48 |
| **Slab alloc** | **11** | **11** | **12** | **16** | **24** |
| Box free | 15 | 15 | 16 | 16 | 18 |
| **Slab free** | **4** | **4** | **5** | **7** | **10** |

**Slab is 4x faster** for burst allocation/deallocation.

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
| Setup cost | None | `init()` required |
| Allocation p50 | ~40 cycles | ~11 cycles |
| Deallocation p50 | ~15 cycles | ~4 cycles |
| Worst case | OS syscall (234K cycles) | Freelist pop (10K cycles) |
| Memory overhead | Per-allocation metadata | 8 bytes/slot + stamp |
| Fragmentation | Yes (over time) | No (fixed slots) |
| Cache locality | Scattered | Contiguous |

**Bottom line:** If you're allocating/deallocating the same type frequently and care about latency, slab wins decisively. The 4x faster allocation and 22x better worst-case make it the clear choice for performance-critical paths.

---

## Allocator Isolation (Active Contention)

The most important advantage of a dedicated slab is **isolation from the global allocator**. In production, `Box::new()` shares malloc with every other allocation in your process. The slab is yours alone.

### Test Design

Both Box and Slab run with identical "noise" between measurements:
- 20-80 mixed-size allocations (32B-2KB) to the global allocator
- Swiss-cheese free pattern (keep 50-75%)
- Then measure a batch of 100 allocations

Box goes through the (equally warm) global allocator. Slab bypasses it entirely.

### Results (p50 cycles, clean process state)

| Size | Box | Slab | Box p99.99 | Slab p99.99 |
|------|-----|------|------------|-------------|
| 64B | 15 | **10** | 6003 | **108** |
| 256B | 17 | **17** | 184 | **82** |
| 1024B | 25 | 26 | 107 | 168 |
| 4096B | 133 | **71** | 419 | **367** |

**Key findings:**

1. **Median latency**: Slab wins at small (64B) and large (4096B) sizes, tied at medium sizes
2. **Tail latency is dramatic**: At 64B, Box p99.99 = 6003 cycles vs Slab p99.99 = 108 cycles (**55x better**)
3. **Large allocations**: Slab is ~1.9x faster at 4096B because malloc has to do more bookkeeping

### Why Slab Wins

The global allocator (glibc malloc, jemalloc, etc.) must:
- Handle any size class
- Maintain per-thread caches that other code touches
- Occasionally interact with the OS (mmap, brk)

The slab:
- Is dedicated to one type
- Has its own freelist that nothing else touches
- Never interacts with the OS after init

**This isolation is the killer feature for latency-sensitive code.**

### Benchmark Isolation Warning

**Important**: Run benchmarks in isolation or as the first test in a suite. Memory-intensive tests that run before can pollute cache/TLB state and skew results. We observed 2x slower slab performance at 256B when running after 8 other memory-intensive tests—an artifact of test ordering, not real performance.

For accurate comparisons:
```bash
# Run contention test in isolation
taskset -c 0 ./target/release/examples/minimal_256

# Or ensure it runs first in the stress test suite
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

# Macro API vs slab crate (unrolled methodology)
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
