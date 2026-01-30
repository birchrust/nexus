# nexus-slab Benchmarks

All measurements in CPU cycles using batched unrolled timing (see Methodology below).

**Test conditions:** Intel Core Ultra 7 155H, `taskset -c 0` pinning, turbo boost disabled.

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

- **0 cycles**: Instruction-Level Parallelism (ILP) - CPU executes operation "for free" alongside other work
- **2-3 cycles**: Single memory access, no dependent loads
- **4-8 cycles**: Multiple dependent operations or cache misses

---

## Summary (BoundedSlab p50)

| Operation | Entry API | Key-based | slab crate | Notes |
|-----------|-----------|-----------|------------|-------|
| GET (random) | 5 | **3** | **3** | Key-based matches slab |
| GET (hot) | **1** | - | **1** | ILP - CPU pipelines loads |
| GET_MUT | **2** | **2** | 3 | Entry/key tied |
| CONTAINS | **2** | 3 | **2** | Entry/slab tied |
| INSERT | 7 | - | **5** | slab wins - simpler freelist |
| REMOVE | 7 | **3** | **3** | Key-based matches slab |
| REPLACE | **2** | - | 4 | Entry has direct pointer |
| TAKE | 19 | - | **8** | slab remove+insert faster |

**Key findings:**
- Entry API wins for mutation (GET_MUT, REPLACE) - direct pointer avoids index lookup
- Key-based removal matches slab crate (both ~3 cycles)
- INSERT/TAKE favor slab crate's simpler freelist
- Hot access shows ILP - CPU pipelines repeated loads to same address

---

## GET Operations

### Random Access Pattern

Accessing entries at random indices (realistic workload):

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.get()` | 5 | 7 | 7 | 25 | 234 |
| `get_by_key()` [unsafe] | **3** | **3** | **4** | **5** | 85 |
| slab crate | **3** | 4 | 4 | 10 | 44 |

Entry's higher p50 reflects validity check overhead. Key-based matches slab.

### Hot Access Pattern

Repeatedly accessing the same entry (measures ILP):

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.get()` | **1** | **1** | **1** | **1** | 18 |
| slab crate | **1** | **1** | 2 | 3 | 57 |

Both achieve ~1 cycle due to CPU pipelining repeated loads.

### GET_MUT

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.get_mut()` | **2** | **2** | **2** | **3** | 24 |
| `get_by_key_mut()` [unsafe] | **2** | **2** | **2** | **3** | 32 |
| slab crate | 3 | 3 | 5 | 10 | 73 |

Entry/key-based are 50% faster than slab at p50, with tighter tail latency.

---

## INSERT

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| BoundedSlab | 7 | 10 | 12 | 15 | 78 |
| slab crate | **5** | **5** | **5** | **8** | 87 |

Slab crate wins by ~2 cycles due to simpler freelist management. Our stamp encoding (64-bit with state flags) has slightly more overhead.

---

## REMOVE

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.remove()` | 7 | 11 | 14 | 18 | 221 |
| `remove_by_key()` [unsafe] | **3** | **3** | 5 | 14 | 134 |
| slab crate | **3** | 4 | 7 | 13 | 91 |

Entry-based remove has ~4 cycles overhead for validity check. Key-based matches slab at p50.

---

## REPLACE

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.replace()` | **2** | **3** | **3** | **5** | 70 |
| slab get_mut+replace | 4 | 4 | 5 | 12 | 78 |

Entry's cached pointer saves 2 cycles - no index lookup needed.

---

## CONTAINS

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.is_valid()` | **2** | **2** | **3** | **5** | 23 |
| `contains_key()` | 3 | 3 | 4 | 6 | 65 |
| slab crate | **2** | **2** | 4 | 6 | 81 |

All implementations perform a simple stamp/version comparison. Entry and slab tied at p50.

---

## TAKE (extract value, keep slot reserved)

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.take()` | 19 | 29 | 33 | 139 | 160 |
| slab remove+insert | **8** | **8** | **12** | **19** | 115 |

Take is expensive due to VacantEntry creation overhead. If you need this pattern frequently, consider remove+insert.

---

## Unbounded vs Bounded

The unbounded `Slab` adds ~2-4 cycles per operation due to chunk indirection (extra pointer chase). The tradeoff is:

| | Bounded | Unbounded |
|---|---------|-----------|
| GET overhead | +0 | +2-4 cycles |
| Growth behavior | Fails when full | Adds chunks (no copy) |
| Tail latency | Deterministic | No reallocation stalls |

Use bounded when capacity is known and latency is critical. Use unbounded when growth may be needed and you want to avoid `Vec` reallocation spikes.

---

## When to Use What

### Use `BoundedSlab` + Entry API when:
- You hold entries for repeated access (GET, REPLACE)
- Capacity is known upfront
- You need the absolute lowest GET latency (2 cycles vs 3)

### Use `BoundedSlab` + key-based API when:
- Single-shot access patterns
- Integrating with data structures that store keys
- Remove performance matters (4 cycles vs 8)

### Use `slab` crate when:
- INSERT performance is critical (5 cycles vs 8)
- You don't need Entry handles
- Vec reallocation stalls at p99.99 are acceptable

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

# Full distribution with unrolled methodology
taskset -c 0 ./target/release/examples/perf_full_distribution
```

### Re-enable Turbo Boost

```bash
# Intel
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# AMD
echo 1 | sudo tee /sys/devices/system/cpu/cpufreq/boost
```
