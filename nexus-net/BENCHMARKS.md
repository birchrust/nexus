# nexus-net Performance Benchmarks

CPU: AMD (uncontrolled — no turbo disable, no core pin)
Measurement: rdtsc cycles, batch=64, 100K samples
Date: 2026-03-29

## FrameReader — Inbound Hot Path

### Single-frame, unmasked text (client role, server→client)

This is the market data path. The most important benchmark.

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 32B | 43 | 46 | 64 | 87 | 4200 |
| 128B | 38 | 39 | 66 | 157 | 812 |
| 512B | 42 | 43 | 51 | 190 | 734 |
| 2048B | 81 | 83 | 128 | 381 | 665 |

### Single-frame, unmasked binary (client role)

No UTF-8 validation. Shows the validation cost by comparison with text.

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 128B | 34 | 35 | 92 | 130 | 676 |
| 1024B | 33 | 33 | 34 | 52 | 577 |

### Single-frame, masked text (server role, client→server)

Includes XOR unmask cost on top of parse + UTF-8 validation.

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 128B | 51 | 51 | 53 | 83 | 682 |
| 512B | 65 | 69 | 101 | 247 | 2900 |
| 2048B | 145 | 147 | 203 | 488 | 2200 |

## Component Costs

### apply_mask (SIMD: SSE2/AVX2)

| Size | p50 | p90 | p99 |
|------|-----|-----|-----|
| 64B | 11 | 12 | 12 |
| 128B | 12 | 12 | 13 |
| 512B | 30 | 30 | 34 |
| 1024B | 31 | 32 | 37 |

### simdutf8::basic::from_utf8

| Size | p50 | p90 | p99 |
|------|-----|-----|-----|
| 64B | 5 | 6 | 6 |
| 128B | 7 | 7 | 9 |
| 512B | 15 | 16 | 17 |
| 1024B | 26 | 27 | 28 |

## FrameWriter — Outbound

### encode_text (raw &mut [u8])

| Payload | Role | p50 | p90 | p99 |
|---------|------|-----|-----|-----|
| 128B | Server | 10 | 10 | 11 |
| 512B | Server | 19 | 20 | 20 |
| 2048B | Server | 50 | 52 | 72 |
| 128B | Client | 56 | 59 | 86 |

### encode_text_into (WriteBuf)

| Payload | p50 | p90 | p99 |
|---------|-----|-----|-----|
| 128B | 45 | 47 | 68 |
| 512B | 50 | 52 | 84 |
| 2048B | 82 | 84 | 114 |

## Throughput — Messages Per Second

128B binary messages, unmasked, varying batch size.

| Batch | p50 | p90 | p99 |
|-------|-----|-----|-----|
| 10 | 33 | 35 | 39 |
| 100 | 26 | 27 | 53 |
| 1000 | 30 | 31 | 46 |

At 3GHz: 26 cycles/msg ≈ **~115M msg/sec** (128B binary, batch=100).

## Comparison vs tungstenite

### In-memory parse (no kernel, no TLS)

| Path | nexus-net | tungstenite | Speedup |
|------|----------|-------------|---------|
| text 128B | 38 cycles | 497 cycles | **13x** |
| binary 128B | 34 cycles | 467 cycles | **14x** |
| text 2048B | 81 cycles | 705 cycles | **8.7x** |
| throughput batch | 26 cycles/msg | 182 cycles/msg | **7x** |

### Sustained throughput (in-memory parse + JSON deser)

| Payload | Type | nexus-net | tungstenite | Speedup |
|---------|------|-----------|-------------|---------|
| 40B | binary parse | 18ns, 55M/s | 62ns, 16M/s | 3.4x |
| 77B | JSON+deser | 145ns, 6.9M/s | 204ns, 4.9M/s | 1.4x |
| 40B | TCP loopback | 24ns, 42M/s | 68ns, 15M/s | 2.8x |

### TLS decrypt + parse (no kernel)

| Path | nexus-net | tungstenite | Speedup |
|------|----------|-------------|---------|
| TLS + 77B JSON | 185ns, 5.4M/s | 281ns, 3.6M/s | 1.5x |

## Optimization History

| Change | text 128B p50 | binary 128B p50 | batch p50 |
|--------|--------------|-----------------|-----------|
| Original baseline | 59 | 47 | 47 |
| + zero-copy passthrough | 60 | 45 | 43 |
| + simdutf8 | 47 | 43 | 42 |
| + code cleanup | 45 | 42 | 42 |
| + inline route_opcode/make_message | 40 | 35 | 33 |
| + cold paths | 39 | 35 | 31 |
| + Box<[u8]> + compact + audit fixes | **38** | **34** | **26** |
| **Total improvement** | **36%** | **28%** | **45%** |

## Notes

- All measurements on uncontrolled system (turbo boost enabled, no
  core pinning). Controlled runs would have lower variance.
- Binary is ~4 cycles faster than text at 128B — the simdutf8 cost.
- Masked is ~13 cycles more than unmasked at 128B (SSE2 mask).
- Throughput improves with batch size due to amortized ReadBuf overhead.
- p99.9 spikes above ~100 cycles are system noise (interrupts, scheduler).
- 517/517 Autobahn conformance tests pass.
- ReadBuf::compact() auto-fires when spare() is exhausted with consumed
  data. Cost: memmove of partial frame (~10-20ns for 200B). Jitter
  bounded by partial frame size.
