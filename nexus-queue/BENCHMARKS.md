# Benchmarks

Performance measurements for nexus-queue SPSC ring buffer.

## Running Benchmarks

```bash
# Build release examples
cargo build -p nexus-queue --examples --release

# Run with CPU pinning (two cores for ping-pong)
taskset -c 0,1 ./target/release/examples/bench_spsc

# For more stable results, disable turbo boost:
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
# Re-enable after:
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# Hardware counters (requires perf):
sudo perf stat -r 5 taskset -c 0,1 ./target/release/examples/bench_spsc
```

## Methodology

- **Latency**: Ping-pong round-trip time divided by 2 for one-way estimate
- **Timing**: rdtscp instruction for cycle-accurate measurement
- **Histogram**: hdrhistogram for percentile distribution
- **Warmup**: 10,000 iterations before measurement
- **Samples**: 100,000 latency samples
- **Throughput**: 1M messages, measures total time

## Baseline Results

Single-socket AMD Ryzen, 2.69 GHz base clock, Linux 6.18.

### Latency (one-way, cycles)

| Queue | min | p50 | p99 | p999 | max |
|-------|-----|-----|-----|------|-----|
| **nexus-queue** | 60 | 68 | 130 | 135 | 61k |
| rtrb | 62 | 67 | 123 | 124 | 50k |
| crossbeam (MPMC) | 78 | 83 | 160 | 207 | 8k |

### Latency (one-way, nanoseconds)

| Queue | min | p50 | p99 | p999 |
|-------|-----|-----|-----|------|
| **nexus-queue** | 22.3 | 25.3 | 48.3 | 50.2 |
| rtrb | 23.0 | 24.9 | 45.7 | 46.1 |
| crossbeam (MPMC) | 29.0 | 30.8 | 59.5 | 76.9 |

### Throughput

| Queue | M msgs/sec | ns/msg |
|-------|------------|--------|
| **nexus-queue** | 640 | 1.6 |
| rtrb | 485 | 2.1 |
| crossbeam (MPMC) | 92 | 10.9 |

## Analysis

**nexus-queue vs rtrb**: Similar latency characteristics (both ~25ns p50). nexus-queue
achieves 32% higher throughput (640 vs 485 M msg/s) due to optimized hot path with
cached buffer/mask pointers avoiding Arc indirection.

**vs crossbeam**: crossbeam's ArrayQueue is MPMC (multi-producer multi-consumer),
requiring CAS operations on every push/pop. SPSC queues avoid this overhead entirely,
resulting in 7x higher throughput and ~20% lower latency.

## Notes

- Results vary significantly between single-socket and multi-socket NUMA systems
- The index-based design (head/tail on separate cache lines) performs well on NUMA
- For accurate comparisons, benchmark on your target production hardware
- Tail latency (p9999+) dominated by OS scheduling and interrupts
