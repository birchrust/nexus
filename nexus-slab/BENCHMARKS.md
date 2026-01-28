# nexus-slab Benchmarks

## Baseline Numbers

Benchmarked on Intel Core Ultra 7 155H, pinned to a physical core, turbo boost disabled.

### BoundedSlab (fixed capacity)

| Operation | nexus-slab | slab crate | Difference |
|-----------|------------|------------|------------|
| INSERT p50 | ~20 cycles | ~22 cycles | 9% faster |
| GET p50 | ~24 cycles | ~26 cycles | 8% faster |
| REMOVE p50 | ~24 cycles | ~30 cycles | 20% faster |

### Slab (growable, steady-state)

| Operation | nexus-slab | slab crate | Notes |
|-----------|------------|------------|-------|
| INSERT p50 | ~22 cycles | ~22 cycles | Comparable |
| GET p50 | ~26 cycles | ~26 cycles | Comparable |
| REMOVE p50 | ~28 cycles | ~30 cycles | ~7% faster |

### Slab (growable, during growth)

| Metric | nexus-slab | slab crate | Difference |
|--------|------------|------------|------------|
| Growth p999 | ~40 cycles | ~2000+ cycles | **50x better** |
| Growth max | ~70K cycles | ~1.5M cycles | **20x better** |

The `slab` crate uses `Vec`, which copies all existing data on reallocation.
`nexus-slab` adds independent chunks - no copying.

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
taskset -c 0 ./target/release/examples/perf_insert_cycles
taskset -c 0 ./target/release/examples/perf_get_cycles
taskset -c 0 ./target/release/examples/perf_churn_cycles
taskset -c 0 ./target/release/examples/perf_indexing_cycles
taskset -c 0 ./target/release/examples/perf_mixed_cycles
taskset -c 0 ./target/release/examples/perf_mixed_cycles_bounded
```

### Criterion Benchmarks

Throughput comparison with statistical analysis:

```bash
# Run all comparison benchmarks
cargo bench --bench slab_comparison

# Run specific benchmark group
cargo bench --bench slab_comparison -- INSERT
cargo bench --bench slab_comparison -- GET_sequential
cargo bench --bench slab_comparison -- REMOVE
```

### Example Benchmarks

The `examples/` directory contains:

| Example | Description |
|---------|-------------|
| `perf_slab_compare.rs` | Criterion-based comparison (throughput) |
| `perf_insert_cycles.rs` | Insert latency in cycles |
| `perf_get_cycles.rs` | Get latency in cycles |
| `perf_churn_cycles.rs` | Insert/remove interleaved |
| `perf_indexing_cycles.rs` | Index operator latency |
| `perf_mixed_cycles.rs` | Mixed operations (growable) |
| `perf_mixed_cycles_bounded.rs` | Mixed operations (bounded) |

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
black_box(slab.insert(value));
let end = rdtscp();
let cycles = end.wrapping_sub(start);
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
3. **Warmup** - Prime caches before measurement
4. **Use `black_box`** - Prevent compiler from optimizing away operations
5. **Large sample sizes** - 100K+ operations for stable percentiles

---

## Interpreting Results

### What to look for

- **p50** - Typical operation latency
- **p99/p999** - Tail latency (important for trading systems)
- **max** - Worst case (often during growth or cache misses)

### Expected variation

- p50 should be stable (within ~5 cycles)
- p99/p999 may vary more due to cache effects
- max varies significantly - run multiple times

### Red flags

- Bimodal distribution (two distinct peaks) - indicates different code paths
- Extremely high max (>100K cycles) - indicates allocation or syscall
- Inconsistent p50 - indicates measurement issues

---

## Re-enable Turbo Boost

```bash
# Intel
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# AMD
echo 1 | sudo tee /sys/devices/system/cpu/cpufreq/boost
```
