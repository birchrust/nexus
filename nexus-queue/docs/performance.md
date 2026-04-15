# Performance

Numbers are from the `bench_isolated` criterion suite on an Intel
Core i9 at 3.1 GHz with turbo disabled and cores pinned. Your
mileage will vary by CPU; the relative comparison is what matters.

## Cycles per op (push + pop round-trip, p50)

| Variant | p50 cycles | Throughput |
|---|---|---|
| SPSC | ~200 | 113M msg/sec |
| MPSC | ~180 | 90M msg/sec (2 producers) |
| SPMC | ~169 | 80M msg/sec (2 consumers) |

Compared to `crossbeam::ArrayQueue` (which is MPMC for all cases),
nexus-queue is roughly **3x faster at p50** across all variants.
At p999, SPSC via `nexus-channel` is ~15x better than crossbeam's
blocking channel (see the channel benchmarks).

See [`BENCHMARKS.md`](../BENCHMARKS.md) for the full suite output.

## Why it's fast

1. **Topology-specialized.** No CAS on the producer path of SPSC or
   SPMC. No CAS on the consumer path of SPSC or MPSC. You pay only
   for the synchronization your topology requires.

2. **Cached opposite-side indices.** SPSC producer remembers the
   last `head` it saw and skips reading the atomic when its cache
   says there's room. Turns ~80% of pushes into local-only ops.

3. **Cache-line isolation.** `head` and `tail` on separate 128-byte
   cache lines. No false sharing between writer and reader.

4. **Power-of-two capacity.** Slot index is `index & mask` — no
   division, no conditional branch for wrap.

5. **Per-slot lap counters.** No seqlock retries, no epoch tracking,
   no hazard pointers, no generation wrap anxiety.

## When each variant wins

**SPSC** wins whenever you can arrange exactly one writer and one
reader. This is nearly always possible with a little
re-architecting. Prefer SPSC unless you have a concrete reason not
to.

**MPSC** wins when the fan-in is a natural property of the system
(e.g., thread-per-client → single gateway). If you have the choice
between "one MPSC" and "N SPSCs the consumer drains round-robin",
benchmark both — N SPSCs can beat MPSC if the consumer logic is
cheap, because the consumer avoids lap-counter waits at the cost
of extra polling.

**SPMC** wins when work is uniform and distributable — any worker
can handle any item. If you need affinity (symbol X always goes to
worker Y), use SPSCs keyed by affinity and dispatch on the producer.

## Measurement methodology

Benchmarking queues correctly is hard. The `bench_isolated.rs`
suite does the following:

- Runs producer and consumer on separate cores (pinned via
  `taskset`, not tokio workers).
- Warms up for multiple rounds before measuring.
- Uses RDTSC-based cycle counts, not wall clock — avoids vDSO and
  VDSO-related jitter.
- Reports p50, p99, p999, and max separately. Means are not
  meaningful for tail-sensitive workloads.
- Disables turbo boost and SMT siblings.

If you're benchmarking yourself, remember: **a mean latency number
is a correctness bug in a trading context.** What matters is
p999 and max. A queue with a 200-cycle mean but a 50,000-cycle p999
is worthless for hot-path work; one with a 400-cycle mean and a
2000-cycle p999 is excellent.

## Sizing guidance

- Power of two, so 1024, 4096, 16384, etc.
- Small enough to fit comfortably in L2: 16384 × 8 bytes = 128 KiB
  is about the upper end for a hot-path queue on a typical core.
- Big enough to absorb your worst observed burst plus ~2x margin.
  If the queue is ever full, you're dropping data or rejecting
  work — both are operationally noisy.

For archival or journaling paths (not hot-path), size doesn't
matter as much; up to L3 is fine.
