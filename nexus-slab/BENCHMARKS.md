# nexus-slab Benchmarks

## Baseline Numbers

Benchmarked on Intel Core Ultra 7 155H, pinned to a physical core, turbo boost disabled.

### BoundedSlab (fixed capacity)

| Operation | unchecked | tracked | slab crate | Notes |
|-----------|-----------|---------|------------|-------|
| INSERT p50 | ~24 cycles | ~22 cycles | ~24 cycles | Comparable |
| GET p50 | **~22 cycles** | ~30 cycles | ~28 cycles | Unchecked 21% faster |
| REMOVE p50 | ~30 cycles | ~30 cycles | ~34 cycles | 12% faster |

**unchecked** = `get_unchecked()` or `UntrackedAccessor` (no runtime checks)
**tracked** = `get()` returning `Ref<T>` guard (borrow tracking)

### Slab (growable, steady-state)

| Operation | unchecked | tracked | slab crate | Notes |
|-----------|-----------|---------|------------|-------|
| GET p50 | ~38 cycles | ~44 cycles | ~26 cycles | Chunk decode overhead |

The unbounded `Slab` has inherent overhead from chunk-based storage (decode
chunk index, indirect through chunk pointer). Use `BoundedSlab` when capacity
is known for best performance.

### Slab (growable, during growth)

| Metric | nexus-slab | slab crate | Difference |
|--------|------------|------------|------------|
| Growth p999 | ~64 cycles | ~2700+ cycles | **43x better** |
| Growth max | ~230K cycles | ~2.7M cycles | **12x better** |

The `slab` crate uses `Vec`, which copies all existing data on reallocation.
`nexus-slab` adds independent chunks - no copying. This is the primary value
proposition of the unbounded `Slab`.

### Access Method Comparison (BoundedSlab)

| Method | p50 | p99 | Notes |
|--------|-----|-----|-------|
| `get_unchecked()` | ~22 | ~26 | No checks, fastest |
| `UntrackedAccessor[key]` | ~30 | ~64 | Index syntax, uses get_unchecked |
| `get_untracked()` | ~30 | ~390 | Validity check, no borrow tracking |
| `get()` → `Ref<T>` | ~32 | ~68 | Validity + borrow tracking |
| `contains_key()` | ~30 | ~64 | Validity check only |
| `Entry::get_unchecked()` | ~22 | ~26 | Direct pointer, no checks |
| `Entry::get()` | ~32 | ~68 | Validity + borrow tracking |

### API Safety vs Performance

```
                        ┌─────────────────────────────┐
                        │       SAFETY CHECKS         │
                        ├─────────────┬───────────────┤
                        │  Validity   │    Borrow     │
                        │  (occupied) │   (runtime)   │
┌───────────────────────┼─────────────┼───────────────┤
│ get_unchecked()       │      -      │       -       │ ~22 cycles
│ UntrackedAccessor[key]│      -      │       -       │ ~30 cycles
│ get_untracked()       │      ✓      │       -       │ ~30 cycles
│ get() → Ref<T>        │      ✓      │       ✓       │ ~32 cycles
│ contains_key()        │      ✓      │       -       │ ~30 cycles
│ Entry::get_unchecked()│      -      │       -       │ ~22 cycles
│ Entry::get_untracked()│      ✓      │       -       │ ~30 cycles
│ Entry::get() → Ref<T> │      ✓      │       ✓       │ ~32 cycles
└───────────────────────┴─────────────┴───────────────┘
```

**Recommendations:**
- **Hot paths:** Use `get_unchecked()` (~22 cycles) or `UntrackedAccessor` (~30 cycles)
- **Normal paths:** Use `get()` for safe tracked access (~32 cycles)
- **With Entry handles:** Use `Entry::get_unchecked()` when Entry validity is known

**Entry size:** 16 bytes (slot pointer + vtable pointer)

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

# Run pinned to a physical core (adjust path for workspace)
taskset -c 0 ./target/release/examples/perf_insert_cycles
taskset -c 0 ./target/release/examples/perf_get_cycles
taskset -c 0 ./target/release/examples/perf_churn_cycles
taskset -c 0 ./target/release/examples/perf_indexing_cycles
taskset -c 0 ./target/release/examples/perf_mixed_cycles
taskset -c 0 ./target/release/examples/perf_mixed_cycles_bounded
```

### Available Benchmarks

| Example | Description |
|---------|-------------|
| `perf_insert_cycles.rs` | Insert latency (unbounded Slab) |
| `perf_get_cycles.rs` | Get latency via UntrackedAccessor (unbounded) |
| `perf_churn_cycles.rs` | Insert/remove interleaved |
| `perf_indexing_cycles.rs` | Index operator latency via UntrackedAccessor |
| `perf_mixed_cycles.rs` | Mixed ops with growth (unbounded) |
| `perf_mixed_cycles_bounded.rs` | Mixed ops: tracked vs unchecked |
| `perf_access_methods.rs` | **All access methods side-by-side** |

**Recommended:** Run `perf_access_methods` for a clear comparison of all access methods:
- `get_unchecked()` (~22 cycles) - no checks
- `get_untracked()` (~30 cycles) - validity check only
- `get()` → `Ref<T>` (~32 cycles) - tracked
- `UntrackedAccessor[key]` (~30 cycles) - Index syntax
- `contains_key()` (~30 cycles) - validity check only

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

// Tracked access (safe, returns guard)
let start = rdtscp();
black_box(slab.get(key));  // Returns Option<Ref<T>>
let end = rdtscp();

// Untracked access (unsafe, fastest)
// SAFETY: No Entry operations during benchmark
let accessor = unsafe { slab.untracked() };
let start = rdtscp();
black_box(accessor[key]);  // Direct &T via Index
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
