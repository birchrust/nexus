# nexus-ascii Performance Benchmarks

CPU cycles measured via `rdtsc` on x86_64. Turbo boost was **not** disabled
for these runs — absolute cycle counts may vary, but relative comparisons
between operations are stable.

**System:** SSE2 baseline (no AVX2 flags). 100k iterations, 10k warmup.

Run any benchmark yourself:
```bash
cargo run --release --example perf_string
cargo run --release --example perf_comparison
cargo run --release --example perf_transform
cargo run --release --example perf_hash
cargo run --release --features serde --example perf_serde
cargo run --release --example perf_simd_crossover
```

---

## Construction

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `empty()` | 14 | 18 | 30 |
| `try_from` (7B "BTC-USD") | 14 | 18 | 32 |
| `try_from` (20B) | 20 | 22 | 46 |
| `try_from` (32B, full cap) | 18 | 24 | 74 |
| `from_bytes_unchecked` (7B) | 16 | 18 | 18 |

Construction includes: ASCII validation (SIMD) + XXH3 hash computation + inline copy.
At 7B (typical symbol), construction is **14 cycles** — same cost as a single L1 cache miss.

---

## Equality

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `eq` (same content) | 14 | 18 | 18 |
| `eq` (different content) | 14 | 18 | 20 |
| `eq` (different length) | 16 | 18 | 18 |
| Baseline: `u64 == u64` | 16 | 18 | 18 |

Equality is a single `u64` comparison (packed hash+length header). Matches
the cost of a raw integer compare — **most non-equal strings are rejected
without touching the byte buffer.**

---

## HashMap

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `HashMap::get` (100 entries) | 16 | 20 | 36 |
| `HashMap::insert` (new key) | 170 | 226 | 332 |

Lookups are fast because `Hash` returns the precomputed hash (zero runtime
hashing cost). The 16-cycle p50 is dominated by the HashMap probe, not hashing.

---

## Ordering & Comparison

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `cmp()` equal (7B) | 24 | 82 | 202 |
| `cmp()` different (7B) | 26 | 32 | 62 |
| `cmp()` equal (37B) | 16 | 28 | 30 |
| `cmp()` differ at end (37B) | 14 | 28 | 30 |
| `eq_ignore_ascii_case()` same case (7B) | 22 | 30 | 256 |
| `eq_ignore_ascii_case()` diff case (7B) | 22 | 26 | 40 |
| `eq_ignore_ascii_case()` same case (38B) | 24 | 58 | 114 |
| `eq_ignore_ascii_case()` diff case (38B) | 18 | 22 | 22 |
| `eq_ignore_ascii_case()` same case (69B) | 26 | 32 | 66 |
| `eq_ignore_ascii_case()` diff case (69B) | 32 | 36 | 58 |
| `starts_with()` 3B prefix (7B string) | 20 | 22 | 24 |
| `ends_with()` 3B suffix (7B string) | 18 | 20 | 22 |
| `contains()` 1B needle (7B string) | 18 | 20 | 32 |

**Baselines (std `&str`):**

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `&str eq_ignore_ascii_case` | 24 | 30 | 52 |
| `&str starts_with` | 18 | 22 | 28 |
| `&str ends_with` | 18 | 20 | 22 |
| `&str contains` | 18 | 20 | 22 |

`eq_ignore_ascii_case` uses SWAR for < 64B (zero domain-crossing overhead)
and SSE2/AVX2 for >= 64B. Performance matches or beats std.

---

## Case Conversion

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `to_ascii_uppercase` (7B) | 28 | 30 | 54 |
| `to_ascii_lowercase` (7B) | 26 | 56 | 80 |
| `to_ascii_uppercase` (20B) | 14 | 18 | 24 |
| `to_ascii_lowercase` (20B) | 16 | 18 | 28 |
| `to_ascii_uppercase` (32B) | 14 | 18 | 60 |
| `to_ascii_lowercase` (32B) | 14 | 16 | 22 |

**Baselines (std in-place):**

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `std make_ascii_uppercase` (20B) | 14 | 16 | 18 |
| `std make_ascii_lowercase` (20B) | 14 | 16 | 18 |

Case conversion uses SSE2 (16B/iter) with no domain crossing — results stay
in SIMD registers and are stored directly. The slightly higher numbers for
AsciiString include the cost of constructing a new string (validation + hash
of the result).

---

## Serde Deserialization (vs String)

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| **7B (trading symbols)** |
| `AsciiString<32>` from JSON | 42 | 92 | 156 |
| `String` from JSON | 42 | 102 | 134 |
| `&str` from JSON (borrowed) | 26 | 30 | 36 |
| `try_from_str` (no JSON) | 16 | 20 | 58 |
| **20B (order IDs)** |
| `AsciiString<32>` from JSON | 42 | 48 | 136 |
| `String` from JSON | 42 | 104 | 138 |
| `&str` from JSON (borrowed) | 24 | 40 | 70 |
| `try_from_str` (no JSON) | 18 | 22 | 34 |
| **38B (long identifiers)** |
| `AsciiString<64>` from JSON | 58 | 116 | 150 |
| `String` from JSON | 52 | 102 | 148 |
| `&str` from JSON (borrowed) | 40 | 44 | 82 |
| `try_from_str` (no JSON) | 18 | 24 | 40 |
| **64B (large protocol fields)** |
| `AsciiString<128>` from JSON | 70 | 154 | 212 |
| `String` from JSON | 56 | 106 | 128 |
| `&str` from JSON (borrowed) | 44 | 48 | 82 |
| `try_from_str` (no JSON) | 16 | 18 | 24 |

**Key takeaways:**

- At **7-20B** (symbols, short IDs): AsciiString matches String at p50 (~42 cycles).
  The ASCII validation + hash computation cost is offset by avoiding heap allocation.
- At **38B+**: String starts winning at p50 because malloc is a pointer bump while
  validation + hash grows with length.
- **`try_from_str`** (16-18 cycles at all sizes) shows JSON parsing dominates the
  serde path — the actual type construction is trivial.
- At **p99**: AsciiString has better tail at 20B (48 vs 104), worse at 64B (154 vs 106).
  String's tail comes from malloc contention; AsciiString's from cache effects on
  larger inline buffers.
- **Serialization** is identical for both types (both serialize as `&str`).

**When AsciiString wins over String for serde:**
- Short strings (< 32B) — avoids allocation jitter
- High-frequency deserialization — no GC/allocator pressure
- Downstream HashMap lookups — hash is precomputed at construction, not at lookup

---

## Hash (XXH3)

| Input Size | p50 | p99 | p999 | cycles/byte |
|-----------|-----|-----|------|-------------|
| 8B | 20 | 22 | 24 | 2.50 |
| 16B | 18 | 20 | 22 | 1.12 |
| 32B | 28 | 30 | 32 | 0.88 |
| 64B | 54 | 58 | 58 | 0.84 |
| 128B | 72 | 78 | 78 | 0.56 |
| 256B (SIMD) | 54 | 98 | 100 | 0.21 |
| 1KB (SIMD) | 118 | 122 | 206 | 0.12 |
| 4KB (SIMD) | 394 | 606 | 1000 | 0.10 |

For AsciiString (CAP <= 128), hashing is always scalar XXH3. The SIMD paths
(SSE2/AVX2/AVX-512) only activate for inputs > 240B, which matters for
`AsciiStr` (DST) but never for fixed-capacity `AsciiString`.

---

## SIMD Validation Crossover

Shows cycles/byte at various input lengths for `validate_ascii`:

| Length | p50 | cycles/byte |
|--------|-----|-------------|
| 7B | 14 | 2.00 |
| 8B | 16 | 2.00 |
| 15B | 16 | 1.07 |
| 16B | 16 | 1.00 |
| 32B | 14 | 0.44 |
| 64B | 16 | 0.25 |
| 128B | 16 | 0.12 |

Validation cost is **flat at 16 cycles** from 8B to 128B — SIMD setup is
amortized even for short strings. The cycles/byte improves with length
as the per-iteration cost is distributed over more data.

---

## Notes

- All benchmarks use `rdtsc` (CPU cycles), not wall-clock time
- Results vary by CPU microarchitecture (measured on x86_64 desktop)
- For AVX2 results, run with `RUSTFLAGS="-C target-feature=+avx2"`
- For production measurements, disable turbo boost and pin to a physical core:
  ```bash
  echo 1 | sudo tee /sys/devices/system/cpu/intel_pstate/no_turbo
  sudo taskset -c 2 ./target/release/examples/perf_serde
  ```
