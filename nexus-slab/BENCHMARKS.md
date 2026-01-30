# nexus-slab Benchmarks

All measurements in CPU cycles using `rdtscp`. Best of 5 runs with `taskset -c 0` pinning.

**Test conditions:** 100k capacity, 500k operations, 50% fill steady-state, pre-allocated.

---

## INSERT

| Variant | p50 | p99 | p99.9 | p99.99 |
|---------|-----|-----|-------|--------|
| bounded | **22** | 28 | 42 | 132 |
| unbounded | 32 | 44 | 54 | 130 |
| slab crate | **22** | 24 | 30 | **2397** |

**Key finding:** bounded and slab are tied at p50 (22 cycles). The slab crate's p99.99 spikes to ~2400 cycles due to `Vec` reallocation. unbounded is ~10 cycles slower due to chunk indirection but has no reallocation stalls.

---

## GET by Key (valid slot)

### Checked + borrow-tracked: `slab.get(key)`

| Variant | p50 | p99 | p99.9 | p99.99 |
|---------|-----|-----|-------|--------|
| bounded | 24 | 52 | 60 | 392 |
| unbounded | 34 | 62 | 76 | 448 |
| slab crate | **22** | **24** | **28** | 138 |

### Untracked (no borrow tracking): `slab.get_untracked(key)` *(unsafe)*

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | **22** | **24** | 38 |
| unbounded | 24 | 28 | 48 |

### Unchecked (no validity check): `slab.get_unchecked(key)` *(unsafe)*

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | **22** | **24** | 40 |
| unbounded | **22** | 26 | 42 |

### UntrackedAccessor indexing: `slab.untracked()[key]` *(unsafe)*

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | **22** | **24** | **36** |
| unbounded | **22** | 26 | 42 |

**Key finding:** The safe checked path (`slab.get(key)`) is 2-12 cycles slower than slab crate at p50. The unsafe paths (`get_unchecked`, `untracked()`) match slab crate performance exactly (22 cycles).

---

## GET by Key (stale/invalid slot)

Tests validity checking when key points to a removed slot.

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | **22** | 26 | 38 |
| unbounded | 28 | 32 | 44 |
| slab crate | **22** | **24** | **28** |

**Key finding:** Validity checking for stale keys is essentially free (same as valid access). The check is a simple version comparison, not a hash lookup.

---

## Entry API - GET (valid slot)

### Checked + borrow-tracked: `entry.get()`

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | 30 | 66 | 418 |
| unbounded | 30 | 66 | 432 |

### Untracked: `entry.get_untracked()` *(unsafe)*

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | **22** | **24** | **36** |
| unbounded | **22** | **24** | **36** |

### Unchecked: `entry.get_unchecked()` *(unsafe)*

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | **22** | **24** | **36** |
| unbounded | **22** | **24** | 38 |

**Key finding:** `entry.get()` is ~8 cycles slower than key-based access at p50 (30 vs 22). This is the cost of creating a `Ref<T>` guard. The unsafe entry methods match the fastest key-based paths (22 cycles).

---

## Entry API - GET (stale entry)

Tests `entry.try_get()` when the slot was removed via another handle.

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | **22** | 26 | 46 |
| unbounded | **22** | 28 | 40 |

**Key finding:** Detecting a stale entry is 22 cycles - same as a valid access. No penalty for safety.

---

## REMOVE by Key

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | **22** | **24** | 40 |
| unbounded | 26 | 32 | 50 |
| slab crate | **22** | **24** | **28** |

---

## Entry API - REMOVE

### Checked: `entry.remove()`

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | **22** | 26 | 42 |
| unbounded | 26 | 34 | 44 |

### Unchecked: `entry.remove_unchecked()` *(unsafe)*

| Variant | p50 | p99 | p99.9 |
|---------|-----|-----|-------|
| bounded | **22** | **24** | **38** |
| unbounded | 24 | 30 | 46 |

**Key finding:** `entry.remove_unchecked()` saves 2-4 cycles at p99 by skipping the `is_available()` check. At p50 they're equivalent for bounded (22 cycles).

---

## Summary

### bounded vs slab crate (p50)

| Operation | bounded | slab | Δ |
|-----------|---------|------|---|
| insert | 22 | 22 | tie |
| get (checked) | 24 | 22 | +2 |
| get (unchecked) | 22 | 22 | tie |
| get (stale) | 22 | 22 | tie |
| remove | 22 | 22 | tie |

### bounded vs unbounded (p50)

| Operation | bounded | unbounded | Δ |
|-----------|---------|-----------|---|
| insert | 22 | 32 | +10 |
| get (checked) | 24 | 34 | +10 |
| get (unchecked) | 22 | 22 | tie |
| remove | 22 | 26 | +4 |

The ~10 cycle overhead for unbounded comes from chunk indirection (extra pointer chase).

---

## When to Use What

### Use `bounded::Slab` when:
- Capacity is known at compile time or startup
- You need the absolute lowest latency
- Memory footprint must be fixed

### Use `unbounded::Slab` when:
- Capacity may grow over time
- You need no-copy growth (important for embedded pointers)
- The 10-cycle overhead is acceptable

### Use `slab` crate when:
- You don't need the Entry API
- You don't need validity checking for stale keys
- Vec reallocation stalls (p99.99) are acceptable

---

## Access Method Quick Reference

### GET

| Method | Validity | Borrow | p50 | Use case |
|--------|----------|--------|-----|----------|
| `slab.get(key)` | ✓ | ✓ | 24 | General safe access |
| `slab.get_untracked(key)` | ✓ | - | 22 | Single accessor, still validates |
| `slab.get_unchecked(key)` | - | - | 22 | Hot path, caller guarantees validity |
| `slab.untracked()[key]` | - | - | 22 | Batch access pattern |
| `entry.get()` | ✓ | ✓ | 24 | Repeated access with guard |
| `entry.get_untracked()` | ✓ | - | 22 | Known single accessor via entry |
| `entry.get_unchecked()` | - | - | 22 | Fastest via entry |
| `entry.try_get()` | ✓ | ✓ | 22 | Check if entry still valid |

### REMOVE

| Method | Validity | p50 | Use case |
|--------|----------|-----|----------|
| `slab.remove_by_key(key)` | ✓ | 24 | Remove by key, returns Option |
| `slab.remove_unchecked_by_key(key)` | - | 22 | Unsafe, caller guarantees validity |
| `entry.remove()` | ✓ | 22 | Remove via entry handle |
| `entry.remove_unchecked()` | - | 22 | Unsafe, skips availability check |

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

# Full API comparison (all methods)
taskset -c 0 ./target/release/examples/perf_full_distribution

# Isolated GET comparison
taskset -c 0 ./target/release/examples/perf_get_comparison

# Insert latency
taskset -c 0 ./target/release/examples/perf_insert_cycles

# Mixed operations
taskset -c 0 ./target/release/examples/perf_mixed_cycles
```

### Re-enable Turbo Boost

```bash
# Intel
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# AMD
echo 1 | sudo tee /sys/devices/system/cpu/cpufreq/boost
```
