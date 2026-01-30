# nexus-collections Benchmarks

## Baseline Numbers (v0.5.0 - BoxedStorage)

Benchmarked on Intel Core Ultra 7 155H, pinned to a physical core, turbo boost disabled.

These are baseline numbers with the current `BoxedStorage` implementation before
transitioning to specialized storage types in v0.6.0.

### List (doubly-linked list)

| Operation | Individual | Mixed | Complexity | Notes |
|-----------|------------|-------|------------|-------|
| PUSH_BACK | TBD | TBD | O(1) | Append to tail |
| PUSH_FRONT | TBD | TBD | O(1) | Prepend to head |
| POP_FRONT | TBD | TBD | O(1) | Remove from head |
| POP_BACK | TBD | TBD | O(1) | Remove from tail |
| REMOVE | - | TBD | O(1) | Remove by key |
| GET | - | TBD | O(1) | Access by key |

**Expected:** All operations O(1), ~30-60 cycles typical.

### Heap (binary min-heap)

| Operation | Individual | Mixed | Complexity | Notes |
|-----------|------------|-------|------------|-------|
| PUSH | TBD | TBD | O(log n) | Insert with sift-up |
| POP | TBD | TBD | O(log n) | Extract-min with sift-down |
| PEEK | TBD | TBD | O(1) | View minimum |
| REMOVE | - | TBD | O(log n) | Remove by key |
| DECREASE_KEY | - | TBD | O(log n) | Update priority |

**Expected:** Log-n operations scale with heap size. At 50K elements, ~16 levels.

### SkipList (probabilistic sorted map)

| Operation | Individual | Mixed | Complexity | Notes |
|-----------|------------|-------|------------|-------|
| INSERT | TBD | TBD | O(log n) | Probabilistic insertion |
| GET | TBD | TBD | O(log n) | Key lookup |
| REMOVE | - | TBD | O(log n) | Remove by key |
| FIRST | TBD | TBD | O(1) | Smallest key-value |
| LAST | TBD | TBD | O(1) | Largest key-value |
| POP_FIRST | TBD | TBD | O(1) | Remove smallest |

**Expected:** Log-n for search operations, O(1) for first/last access.

---

## Running Benchmarks

### Prerequisites

```bash
# Disable turbo boost (Intel)
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# Or for AMD
echo 0 | sudo tee /sys/devices/system/cpu/cpufreq/boost
```

### Cycle-Accurate Examples

Individual operation benchmarks with rdtscp timing:

```bash
# Build all examples
cargo build --release --examples

# Run pinned to a physical core
taskset -c 0 ./target/release/examples/perf_list_cycles
taskset -c 0 ./target/release/examples/perf_heap_cycles
taskset -c 0 ./target/release/examples/perf_skiplist_cycles
```

### Available Benchmarks

| Example | Description |
|---------|-------------|
| `perf_list_cycles.rs` | List ops: push/pop front/back, remove, get |
| `perf_heap_cycles.rs` | Heap ops: push, pop, peek, remove, decrease_key |
| `perf_skiplist_cycles.rs` | SkipList ops: insert, get, remove, first, last |

---

## Benchmark Methodology

### Cycle Measurement

Uses `rdtscp` for cycle-accurate timing on x86_64:

```rust
#[inline(always)]
fn rdtscp() -> u64 {
    unsafe {
        let mut aux: u32 = 0;
        std::arch::x86_64::__rdtscp(&mut aux)
    }
}

let start = rdtscp();
black_box(list.pop_front(&mut storage));
let end = rdtscp();
```

### Histogram Collection

Uses `hdrhistogram` for percentile tracking:

```rust
let mut hist = Histogram::<u64>::new(3).unwrap();

for i in 0..ITERATIONS {
    let cycles = measure_operation();
    hist.record(cycles);
}

println!("p50:  {} cycles", hist.value_at_quantile(0.50));
println!("p99:  {} cycles", hist.value_at_quantile(0.99));
println!("p999: {} cycles", hist.value_at_quantile(0.999));
```

### Best Practices

1. **Disable turbo boost** - Prevents frequency scaling artifacts
2. **Pin to physical core** - Avoids context switches and cache pollution
3. **Warmup** - Prime caches before measurement (fill to ~50% capacity)
4. **Use `black_box`** - Prevent compiler from optimizing away operations
5. **Large sample sizes** - 500K operations for stable percentiles

---

## Interpreting Results

### What to look for

- **p50** - Typical operation latency
- **p99/p999** - Tail latency (important for trading systems)
- **max** - Worst case (often during cache misses)
- **Individual vs Mixed** - Pure operation vs realistic workload

### Expected variation

- p50 should be stable (within ~10 cycles)
- p99/p999 may vary more due to cache effects
- max varies significantly - run multiple times

### Red flags

- Bimodal distribution - indicates different code paths
- Extremely high max (>100K cycles) - indicates allocation or syscall
- Inconsistent p50 across runs - measurement issues or system noise

---

## Re-enable Turbo Boost

```bash
# Intel
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# AMD
echo 1 | sudo tee /sys/devices/system/cpu/cpufreq/boost
```

---

## v0.6.0 Migration Notes

After implementing specialized storage types (ListStorage, HeapStorage, SkipStorage),
we'll re-run benchmarks to:

1. Confirm no performance regression from the API changes
2. Identify potential optimizations enabled by the new design
3. Benchmark any new safe APIs that hide node internals

The baseline numbers above serve as the comparison point.
