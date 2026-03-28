# nexus-net Performance Benchmarks

CPU: AMD (uncontrolled — no turbo disable, no core pin)
Measurement: rdtsc cycles, batch=64, 100K samples
Date: 2026-03-28
Commit: baseline (pre-optimization)

## FrameReader — Inbound Hot Path

### Single-frame, unmasked text (client role, server→client)

This is the market data path. The most important benchmark.

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 32B | 56 | 71 | 126 | 242 | 3083 |
| 128B | 72 | 75 | 120 | 272 | 1306 |
| 512B | 119 | 125 | 179 | 303 | 2486 |
| 2048B | 258 | 264 | 435 | 777 | 7518 |

### Single-frame, unmasked binary (client role)

No UTF-8 validation. Shows the validation cost by comparison with text.

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 128B | 57 | 60 | 98 | 225 | 5107 |
| 1024B | 89 | 91 | 156 | 335 | 2356 |

### Single-frame, masked text (server role, client→server)

Includes XOR unmask cost on top of parse + UTF-8 validation.

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 128B | 78 | 83 | 175 | 271 | 10258 |
| 512B | 125 | 131 | 192 | 267 | 776 |
| 2048B | 331 | 347 | 467 | 660 | 7079 |

## Component Costs

### apply_mask (SIMD: SSE2/AVX2)

| Size | p50 | p90 | p99 | p99.9 | max |
|------|-----|-----|-----|-------|-----|
| 64B | 12 | 12 | 12 | 17 | 217 |
| 128B | 12 | 12 | 12 | 18 | 1029 |
| 256B | 13 | 14 | 14 | 35 | 225 |
| 512B | 18 | 20 | 20 | 32 | 266 |
| 1024B | 35 | 36 | 57 | 125 | 6210 |
| 4096B | 85 | 91 | 187 | 429 | 4653 |

### std::str::from_utf8 (baseline, pre-simdutf8)

| Size | p50 | p90 | p99 | p99.9 | max |
|------|-----|-----|-----|-------|-----|
| 64B | 15 | 16 | 16 | 21 | 247 |
| 128B | 17 | 18 | 19 | 73 | 233 |
| 256B | 22 | 22 | 26 | 100 | 415 |
| 512B | 30 | 30 | 59 | 116 | 427 |
| 1024B | 68 | 88 | 123 | 241 | 3152 |
| 4096B | 208 | 224 | 428 | 773 | 15237 |

## FrameWriter — Outbound

### encode_text (raw &mut [u8])

| Payload | Role | p50 | p90 | p99 | p99.9 | max |
|---------|------|-----|-----|-----|-------|-----|
| 128B | Server | 18 | 19 | 19 | 22 | 415 |
| 512B | Server | 20 | 28 | 37 | 87 | 3605 |
| 2048B | Server | 50 | 51 | 75 | 217 | 3201 |
| 128B | Client | 24 | 25 | 38 | 103 | 3480 |
| 512B | Client | 45 | 46 | 77 | 140 | 935 |

### encode_text_into (WriteBuf)

| Payload | p50 | p90 | p99 | p99.9 | max |
|---------|-----|-----|-----|-------|-----|
| 128B | 35 | 35 | 39 | 114 | 9822 |
| 512B | 42 | 45 | 52 | 129 | 744 |
| 2048B | 72 | 76 | 104 | 168 | 318 |

## Throughput — Messages Per Second

128B binary messages, unmasked, varying batch size.
Cycles per message, measured over the full read+parse loop.

| Batch | p50 | p90 | p99 | p99.9 | max |
|-------|-----|-----|-----|-------|-----|
| 10 | 65 | 74 | 80 | 87 | 864 |
| 100 | 50 | 70 | 119 | 237 | 9585 |
| 1000 | 47 | 51 | 69 | 111 | 460 |

At 3GHz: 47 cycles/msg ≈ **~64M msg/sec** (128B binary, batch=1000).

## Cost Breakdown (128B unmasked text)

| Component | Estimated cycles | Notes |
|-----------|-----------------|-------|
| ReadBuf read() | ~8 | memcpy 130B (header+payload) |
| Header parse | ~10 | 2-byte header, branch prediction |
| memcpy to msg_buf | ~15 | **eliminated by zero-copy** |
| UTF-8 validation | ~17 | **target for simdutf8** |
| Message construction | ~5 | |
| msg_buf cleanup | ~5 | clear on next call |
| **Total** | **~72** | measured p50 |

The memcpy to msg_buf (~15 cycles) is the zero-copy optimization
target. UTF-8 validation (~17 cycles) is the simdutf8 target.
Together they account for ~45% of the hot path cost.

## Notes

- All measurements on uncontrolled system (turbo boost enabled,
  no core pinning). Controlled runs will have lower variance.
- Binary is ~15 cycles faster than text at 128B — that's the
  UTF-8 validation cost.
- Masked is ~6 cycles more than unmasked at 128B (SSE2 mask is fast).
- WriteBuf encode_into is ~2x raw encode due to clear+append overhead.
  The WriteBuf path avoids per-send allocation in WsStream, so the
  total cost is lower despite higher encode time.
- Throughput improves with batch size due to amortized ReadBuf overhead.
