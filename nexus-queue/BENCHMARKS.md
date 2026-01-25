# Benchmarks

Performance measurements for nexus-queue SPSC and MPSC ring buffers.

## Running Benchmarks

```bash
# Build release examples
cargo build -p nexus-queue --examples --release

# Run with CPU pinning (separate physical cores, not hyperthreads)
# Check your topology with: lscpu -e
taskset -c 0,2 ./target/release/examples/bench_spsc
taskset -c 0,2 ./target/release/examples/bench_mpsc_pingpong

# For more stable results, disable turbo boost:
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
# Re-enable after:
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# Hardware counters (requires perf):
sudo perf stat -r 5 taskset -c 0,2 ./target/release/examples/bench_spsc
```

**Important**: Use separate physical cores (e.g., 0,2) not hyperthreads (e.g., 0,1).
Hyperthreads share L1/L2 cache and give artificially low latency numbers.

## Methodology

- **Latency**: Ping-pong round-trip time divided by 2 for one-way estimate
- **Timing**: rdtscp instruction for cycle-accurate measurement
- **Histogram**: hdrhistogram for percentile distribution
- **Warmup**: 10,000 iterations before measurement
- **Samples**: 100,000 latency samples
- **Throughput**: 1M messages, measures total time

## Baseline Results

Intel Core Ultra 7 165U (hybrid P+E cores), 2.69 GHz base clock, Linux 6.18.
**Single-socket, single NUMA node.** Pinned to separate physical P-cores (0,2).

### SPSC Latency (one-way, cycles)

| Queue | min | p50 | p99 | p999 | max |
|-------|-----|-----|-----|------|-----|
| **nexus-queue SPSC** | ~180 | 208 | 237 | 405 | ~4k |
| rtrb | ~180 | ~210 | ~240 | ~400 | ~4k |
| crossbeam (MPMC) | ~450 | 520 | 580 | 820 | ~8k |

### SPSC Throughput

| Queue | M msgs/sec | ns/msg |
|-------|------------|--------|
| **nexus-queue SPSC** | ~50 | ~20 |

### MPSC Latency (one-way, cycles)

| Queue | p50 | p99 | p999 | Notes |
|-------|-----|-----|------|-------|
| **nexus-queue MPSC** | 202 | 276-411 | 350-540 | CAS + turn counter |
| crossbeam ArrayQueue | 522-532 | 574-584 | 817-876 | MPMC overhead |

### MPSC Latency (one-way, nanoseconds)

| Queue | p50 | p99 | p999 |
|-------|-----|-----|------|
| **nexus-queue MPSC** | 75 ns | 103-153 ns | 130-201 ns |
| crossbeam ArrayQueue | 195 ns | 213-217 ns | 304-326 ns |

## Analysis

### SPSC

**nexus-queue vs rtrb**: Nearly identical performance. Both use the same
cached index design with separate cache lines for head/tail.

**vs crossbeam**: crossbeam's ArrayQueue is MPMC (multi-producer multi-consumer),
requiring CAS operations on every push/pop. SPSC queues avoid this overhead entirely.

### MPSC

**nexus-queue MPSC is 2.6x faster than crossbeam ArrayQueue at p50.**

Key optimizations:
1. **Cached head in Producer** - avoids atomic load when queue not full
2. **Cached slots/mask/capacity** - avoids Arc indirection on hot path
3. **Single consumer** - no CAS contention on consumer side (vs crossbeam's MPMC)
4. **`#[repr(C)]` layout** - hot fields at struct base

The MPSC is ~3% slower than SPSC (202 vs 208 cycles) for the producer side due to
CAS on tail, but this is acceptable for the "producers NOT on hot path" use case.

## Notes

- Results vary significantly with core topology - always use separate physical cores
- Hyperthreaded cores (same physical core) share cache and give misleading results
- For accurate comparisons, benchmark on your target production hardware
- Tail latency (p9999+) dominated by OS scheduling and interrupts

## Multi-Socket NUMA Considerations

These benchmarks were run on a **single-socket** system. On multi-socket NUMA
architectures (common in production servers), the benefits of nexus-queue's
design should be **even more pronounced**:

1. **Cached head/slots/mask** - Avoids cross-socket memory accesses on the hot path.
   Reading from a remote NUMA node can cost 100-300ns additional latency.

2. **Separate cache lines for head/tail** - Prevents false sharing across sockets,
   which is particularly expensive when cache coherency traffic must traverse
   the interconnect (QPI/UPI on Intel, Infinity Fabric on AMD).

3. **Local producer state** - Each producer's cached_head stays socket-local,
   only refreshing from shared memory when the cache indicates the queue is full.

For latency-critical production deployments on multi-socket servers, pin producers
and consumers to cores on the same socket when possible, or ensure the queue's
shared memory is allocated on the consumer's local NUMA node.
