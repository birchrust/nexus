# nexus-net Performance Benchmarks

CPU: AMD (uncontrolled — no turbo disable, no core pin)
Measurement: rdtsc cycles, batch=64, 100K samples
Date: 2026-03-29
Commit: post-optimization (zero-copy + simdutf8 + inline + cold paths)

## FrameReader — Inbound Hot Path

### Single-frame, unmasked text (client role, server→client)

This is the market data path. The most important benchmark.

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 32B | 38 | 40 | 45 | 70 | 900 |
| 128B | 39 | 40 | 44 | 70 | 900 |
| 512B | 50 | 52 | 65 | 150 | 1000 |
| 2048B | 91 | 94 | 120 | 350 | 900 |

### Single-frame, unmasked binary (client role)

No UTF-8 validation. Shows the validation cost by comparison with text.

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 128B | 35 | 36 | 40 | 60 | 1000 |
| 1024B | 40 | 42 | 50 | 90 | 1000 |

### Single-frame, masked text (server role, client→server)

Includes XOR unmask cost on top of parse + UTF-8 validation.

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 128B | 55 | 57 | 70 | 150 | 2000 |
| 512B | 72 | 75 | 95 | 200 | 2000 |
| 2048B | 150 | 155 | 200 | 500 | 3000 |

## Component Costs

### apply_mask (SIMD: SSE2/AVX2)

| Size | p50 | p90 | p99 | p99.9 | max |
|------|-----|-----|-----|-------|-----|
| 64B | 12 | 12 | 14 | 20 | 600 |
| 128B | 13 | 13 | 16 | 23 | 900 |
| 512B | 32 | 33 | 41 | 110 | 740 |
| 1024B | 36 | 39 | 53 | 150 | 2300 |

### simdutf8::basic::from_utf8

| Size | p50 | p90 | p99 | p99.9 | max |
|------|-----|-----|-----|-------|-----|
| 64B | 5 | 6 | 8 | 10 | 680 |
| 128B | 7 | 8 | 12 | 32 | 320 |
| 512B | 17 | 17 | 25 | 38 | 1600 |
| 1024B | 29 | 30 | 45 | 120 | 760 |

## FrameWriter — Outbound

### encode_text (raw &mut [u8])

| Payload | Role | p50 | p90 | p99 | p99.9 | max |
|---------|------|-----|-----|-----|-------|-----|
| 128B | Server | 11 | 11 | 12 | 23 | 220 |
| 512B | Server | 21 | 22 | 32 | 96 | 1400 |
| 2048B | Server | 54 | 55 | 88 | 180 | 1300 |
| 128B | Client | 26 | 26 | 28 | 110 | 700 |

### encode_text_into (WriteBuf)

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 128B | 39 | 42 | 45 | 130 | 900 |
| 512B | 45 | 46 | 60 | 130 | 400 |
| 2048B | 77 | 80 | 102 | 180 | 630 |

## Throughput — Messages Per Second

128B binary messages, unmasked, varying batch size.
Cycles per message, measured over the full read+parse loop.

| Batch | p50 | p90 | p99 | p99.9 | max |
|-------|-----|-----|-----|-------|-----|
| 10 | 42 | 48 | 58 | 90 | 900 |
| 100 | 33 | 37 | 48 | 80 | 400 |
| 1000 | 31 | 35 | 42 | 70 | 250 |

At 3GHz: 31 cycles/msg ≈ **~97M msg/sec** (128B binary, batch=1000).

## Comparison vs tungstenite

### In-memory parse (no kernel, no TLS)

| Path | nexus-net | tungstenite | Speedup |
|------|----------|-------------|---------|
| text 128B | 39 cycles | 497 cycles | **12.7x** |
| binary 128B | 35 cycles | 467 cycles | **13.3x** |
| text 2048B | 91 cycles | 705 cycles | **7.7x** |
| throughput batch | 31 cycles/msg | 182 cycles/msg | **5.9x** |

### Sustained throughput (in-memory parse + JSON deser)

| Payload | Type | nexus-net | tungstenite | Speedup |
|---------|------|-----------|-------------|---------|
| 40B | binary parse | 22ns, 44.6M/s | 60ns, 16.7M/s | 2.7x |
| 77B | JSON+deser | 145ns, 6.9M/s | 204ns, 4.9M/s | 1.4x |
| 148B | JSON+deser | 326ns, 3.1M/s | 374ns, 2.7M/s | 1.1x |
| 40B | TCP loopback | 25ns, 39.7M/s | 62ns, 16.1M/s | 2.5x |

### TLS decrypt + parse (no kernel)

| Path | nexus-net | tungstenite | Speedup |
|------|----------|-------------|---------|
| 128B text+TLS | 723 cycles | 3,336 cycles | 4.6x |

## Optimization History

| Change | text 128B p50 | binary 128B p50 | batch p50 |
|--------|--------------|-----------------|-----------|
| Original baseline | 59 | 47 | 47 |
| + zero-copy passthrough | 60 | 45 | 43 |
| + simdutf8 | 47 | 43 | 42 |
| + code cleanup (is_control cache, etc.) | 45 | 42 | 42 |
| + inline route_opcode + make_message | 40 | 35 | 33 |
| + cold paths (partial payload, close) | **39** | **35** | **31** |
| **Total improvement** | **34%** | **26%** | **34%** |

## Notes

- All measurements on uncontrolled system (turbo boost enabled, no
  core pinning). Controlled runs would have lower variance.
- Binary is ~4 cycles faster than text at 128B — that's the simdutf8
  validation cost.
- Masked is ~16 cycles more than unmasked at 128B (SSE2 mask is fast).
- Throughput improves with batch size due to amortized ReadBuf overhead.
- p99.9 spikes above ~100 cycles are system noise (interrupts,
  scheduler). Our code's true p99 is 40-44 cycles.
- 517/517 Autobahn conformance tests pass.
