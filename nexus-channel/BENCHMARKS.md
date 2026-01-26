# nexus-channel Benchmarks

Benchmarks run on AMD Ryzen / Intel Core system. Results in CPU cycles for consistency across clock speeds.

## SPSC Channel

Ping-pong latency benchmark measuring round-trip time divided by 2 (one-way estimate).

| Metric | nexus-channel | crossbeam-channel | Improvement |
|--------|---------------|-------------------|-------------|
| p50    | ~400 cycles   | ~485 cycles       | **1.2x**    |
| p99    | ~800 cycles   | ~660 cycles       | 0.8x        |
| p999   | ~2000 cycles  | ~1900 cycles      | ~1x         |

nexus-channel optimizes for median latency. Crossbeam has slightly better p99 in some runs due to different backoff strategies.

## MPSC

For MPSC channels, use [crossbeam-channel](https://docs.rs/crossbeam-channel) which is well-optimized for multi-producer workloads.

For raw MPSC queue performance without blocking semantics, [nexus-queue::mpsc](https://docs.rs/nexus-queue) provides a 2.4x faster queue primitive than crossbeam's ArrayQueue.

## Running Benchmarks

```bash
# Build all benchmarks
cargo build --release -p nexus-channel --benches

# SPSC latency
./target/release/deps/perf_channel_latency-*

# Crossbeam comparison
./target/release/deps/perf_crossbeam_channel_latency-*
```

For best results, disable turbo boost and pin to physical cores:

```bash
# Disable turbo (Intel)
echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo

# Pin to cores
sudo taskset -c 0,2 ./target/release/deps/perf_channel_latency-*

# Re-enable turbo
echo 0 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
```

## Design Trade-offs

- Three-phase backoff (try → spin → park) minimizes syscalls
- Conditional unpark: only wake if receiver is actually parked
- Result: consistent low latency, especially at p50
