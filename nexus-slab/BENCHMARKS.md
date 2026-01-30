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
| GET (random) | **2** | **2** | 3 | Entry/key tied, faster than slab |
| GET (hot) | **0** | - | 1 | ILP - CPU pipelines loads |
| GET_MUT | **2** | **2** | 3 | Entry/key tied |
| CONTAINS | **2** | 3 | 3 | Entry fastest |
| INSERT | 7 | - | **5** | slab wins - simpler freelist |
| REMOVE | 8 | **3** | 4 | Key-based fastest |
| REPLACE | **3** | - | 4 | Entry has direct pointer |
| TAKE | 18 | - | **8** | slab remove+insert faster |

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
| `entry.get()` | **2** | **2** | **2** | 5 | 21 |
| `get_by_key()` [unsafe] | **2** | **2** | **2** | **2** | 33 |
| slab crate | 3 | 3 | 3 | 8 | 88 |

Entry and key-based both beat slab crate by ~1 cycle.

### Hot Access Pattern

Repeatedly accessing the same entry (measures ILP):

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.get()` | **0** | 1 | 2 | 2 | 1731 |
| slab crate | 1 | 2 | 2 | 2 | 25 |

Entry achieves 0 cycles at p50 due to CPU pipelining repeated loads.

### GET_MUT

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.get_mut()` | **2** | **2** | **2** | **2** | 97 |
| `get_by_key_mut()` [unsafe] | **2** | **2** | 3 | 5 | 53 |
| slab crate | 3 | 3 | 3 | 7 | 116 |

Entry/key-based are 33% faster than slab at p50.

---

## INSERT

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| BoundedSlab | 7 | 11 | 12 | 41 | 105 |
| slab crate | **5** | **5** | **6** | **10** | 81 |

Slab crate wins by ~2 cycles due to simpler freelist management. Our stamp encoding (64-bit with state flags) has slightly more overhead.

---

## REMOVE

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.remove()` | 8 | 12 | 16 | 23 | 481 |
| `remove_by_key()` [unsafe] | **3** | **4** | 7 | 13 | 68 |
| slab crate | 4 | 4 | 4 | 13 | 48 |

Key-based remove is fastest at p50. Entry-based has ~5 cycles overhead for validity check.

---

## REPLACE

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.replace()` | **3** | **3** | **3** | **5** | 62 |
| slab get_mut+replace | 4 | 4 | 6 | 13 | 77 |

Entry's cached pointer saves 1 cycle - no index lookup needed.

---

## CONTAINS

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.is_valid()` | **2** | **2** | **3** | **4** | 24 |
| `contains_key()` | 3 | 3 | 3 | 4 | 547 |
| slab crate | 3 | 3 | 3 | 3 | 45 |

Entry is fastest at p50. All implementations perform a simple stamp/version comparison.

---

## TAKE (extract value, keep slot reserved)

| Method | p50 | p90 | p99 | p99.9 | max |
|--------|-----|-----|-----|-------|-----|
| `entry.take()` | 18 | 26 | 35 | 78 | 1124 |
| slab remove+insert | **8** | **8** | **10** | **19** | 111 |

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
