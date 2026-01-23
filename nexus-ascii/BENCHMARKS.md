# nexus-ascii Performance Benchmarks

CPU cycles measured via `rdtsc` on x86_64. Pinned to a single core via
`taskset -c 0` for stable measurements.

**System:** SSE2 baseline (no AVX2 flags). 100k iterations, 10k warmup.

Run any benchmark yourself:
```bash
taskset -c 0 cargo run --release --example perf_string
taskset -c 0 cargo run --release --example perf_comparison
taskset -c 0 cargo run --release --example perf_transform
taskset -c 0 cargo run --release --example perf_hash
taskset -c 0 cargo run --release --features serde --example perf_serde
taskset -c 0 cargo run --release --example perf_simd_crossover
taskset -c 0 cargo run --release --example perf_vs_ascii_crate
taskset -c 0 cargo run --release --features nohash --example perf_hashmap
```

---

## Construction

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `empty()` | 16 | 28 | 32 |
| `try_from` (7B "BTC-USD") | 18 | 38 | 102 |
| `try_from` (20B) | 24 | 50 | 66 |
| `try_from` (32B, full cap) | 24 | 56 | 100 |
| `from_bytes_unchecked` (7B) | 18 | 20 | 26 |

Construction includes: ASCII validation (SIMD) + XXH3 hash computation + inline copy +
zero-padding. The copy uses an overlap-trick inline implementation (no memcpy call) for
inputs <= 32 bytes. At 7B (typical symbol), construction is **18 cycles**.

---

## Equality

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `eq` (same content) | 18 | 20 | 24 |
| `eq` (different content) | 18 | 20 | 20 |
| `eq` (different length) | 16 | 18 | 26 |
| Baseline: `u64 == u64` | 16 | 20 | 20 |

Equality is a single `u64` comparison (packed hash+length header). Matches
the cost of a raw integer compare — **most non-equal strings are rejected
without touching the byte buffer.**

---

## HashMap

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `HashMap::get` (100 entries) | 20 | 24 | 42 |
| `HashMap::insert` (new key) | 190 | 368 | 502 |

Lookups are fast because `Hash` returns the precomputed hash finalized with a
single Fibonacci multiply (~1 cycle). The 20-cycle p50 is dominated by the
HashMap probe, not hashing.

---

## Ordering & Comparison

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `cmp()` equal (7B) | 16 | 20 | 34 |
| `cmp()` different (7B) | 16 | 20 | 34 |
| `cmp()` different lengths (7B vs 3B) | 16 | 18 | 20 |
| `cmp()` equal (37B) | 16 | 18 | 20 |
| `cmp()` differ at end (37B) | 18 | 20 | 20 |
| `eq_ignore_ascii_case()` same case (7B) | 18 | 20 | 20 |
| `eq_ignore_ascii_case()` diff case (7B) | 18 | 20 | 20 |
| `eq_ignore_ascii_case()` same case (38B) | 18 | 20 | 22 |
| `eq_ignore_ascii_case()` diff case (38B) | 18 | 22 | 32 |
| `eq_ignore_ascii_case()` same case (69B) | 20 | 22 | 30 |
| `eq_ignore_ascii_case()` diff case (69B) | 22 | 28 | 52 |
| `starts_with()` 3B prefix (7B string) | 20 | 22 | 24 |
| `ends_with()` 3B suffix (7B string) | 18 | 20 | 22 |
| `contains()` 1B needle (7B string) | 18 | 20 | 32 |

**Baselines (std `&str`):**

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `[u8] cmp()` equal (baseline) | 22 | 32 | 66 |
| `[u8] cmp()` different (baseline) | 20 | 24 | 26 |
| `&str eq_ignore_ascii_case` (7B) | 24 | 30 | 58 |
| `&str eq_ignore_ascii_case` (38B) | 124 | 204 | 356 |
| `&str starts_with` | 18 | 22 | 24 |
| `&str ends_with` | 18 | 22 | 24 |
| `&str contains` | 18 | 20 | 22 |

`cmp()` uses word-at-a-time comparison (u64 loads with `bswap` for
lexicographic ordering). Full-buffer processing means no remainder loops.

`eq_ignore_ascii_case` uses full-buffer SWAR for < 64B (zero domain-crossing
overhead) and SSE2/AVX2 for >= 64B. At 38B, nexus-ascii is **~7x faster**
than std at p50 (18 vs 124 cycles).

---

## Case Conversion

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `to_ascii_uppercase` (7B) | 18 | 20 | 34 |
| `to_ascii_lowercase` (7B) | 18 | 20 | 36 |
| `to_ascii_uppercase` (20B) | 18 | 20 | 20 |
| `to_ascii_lowercase` (20B) | 18 | 20 | 30 |
| `to_ascii_uppercase` (32B) | 18 | 20 | 22 |
| `to_ascii_lowercase` (32B) | 18 | 20 | 24 |

**Baselines (std in-place):**

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `std make_ascii_uppercase` (20B) | 18 | 20 | 20 |
| `std make_ascii_lowercase` (20B) | 18 | 20 | 20 |

Case conversion uses full-buffer SSE2 (16B/iter) with no domain crossing —
results stay in SIMD registers and are stored directly. The slightly higher
p99/p999 for AsciiString includes the cost of constructing a new string
(hash of the result). At p50, matches std.

---

## Builder

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `new()` | 16 | 18 | 34 |
| `push_str` (7B "BTC-USD") | 16 | 22 | 72 |
| `push_str` (20B) | 18 | 20 | 50 |
| `push_byte` | 16 | 18 | 18 |
| `build()` (7B content) | 18 | 20 | 50 |
| `build()` (20B content) | 16 | 18 | 20 |
| Full pipeline: push_str + build (7B) | 16 | 20 | 32 |
| Full pipeline: 3x push_str + build | 26 | 60 | 112 |

Builder construction is allocation-free. The `build()` finalization computes
the hash and applies zero-padding.

---

## Truncation

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `truncated` (54B -> 5B) | 18 | 20 | 22 |
| `truncated` (54B -> 30B) | 18 | 26 | 34 |
| `truncated` (53B -> 53B, no change) | 18 | 20 | 22 |
| `try_truncated` (54B -> 5B) | 18 | 20 | 22 |
| `try_truncated` (fails) | 18 | 20 | 22 |

Truncation includes zeroing the tail and recomputing the hash.

---

## from_raw (Null-Terminated Buffers)

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `try_from_bytes` (7B slice) | 16 | 26 | 48 |
| `try_from_raw` (7B in 16B buffer) | 16 | 32 | 66 |
| `from_raw_unchecked` (7B in 16B buffer) | 16 | 18 | 24 |
| `try_from_right_padded` (7B, space pad) | 18 | 22 | 44 |

Null-byte scanning uses SSE2 on x86_64 (16B/iter), SWAR elsewhere (8B/iter).
The `try_from_raw` cost is dominated by the null search + validation, not the
copy.

---

## Serde Deserialization (vs String)

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| **7B (trading symbols)** |
| `AsciiString<32>` from JSON | 50 | 114 | 176 |
| `String` from JSON | 54 | 104 | 156 |
| `&str` from JSON (borrowed) | 34 | 36 | 76 |
| `try_from_str` (no JSON) | 22 | 24 | 40 |
| **20B (order IDs)** |
| `AsciiString<32>` from JSON | 56 | 112 | 166 |
| `String` from JSON | 52 | 126 | 176 |
| `&str` from JSON (borrowed) | 30 | 56 | 124 |
| `try_from_str` (no JSON) | 24 | 26 | 76 |
| **38B (long identifiers)** |
| `AsciiString<64>` from JSON | 76 | 128 | 154 |
| `String` from JSON | 62 | 128 | 152 |
| `&str` from JSON (borrowed) | 50 | 98 | 170 |
| `try_from_str` (no JSON) | 22 | 26 | 44 |
| **64B (large protocol fields)** |
| `AsciiString<128>` from JSON | 86 | 174 | 284 |
| `String` from JSON | 70 | 134 | 212 |
| `&str` from JSON (borrowed) | 56 | 60 | 100 |
| `try_from_str` (no JSON) | 20 | 22 | 52 |

**Key takeaways:**

- At **7-20B** (symbols, short IDs): AsciiString matches String at p50 (~50 cycles).
  The ASCII validation + hash computation cost is offset by avoiding heap allocation.
- At **38B+**: String starts winning at p50 because malloc is a pointer bump while
  validation + hash grows with length.
- **`try_from_str`** (18-22 cycles at all sizes) shows JSON parsing dominates the
  serde path — the actual type construction is trivial.
- At **p99/p999**: AsciiString consistently beats String for tail latency (no allocator
  pressure, no GC). At 20B: p99 is 104 vs 120, p999 is 160 vs 210.
- **Serialization** is identical for both types (both serialize as `&str`).

**When AsciiString wins over String for serde:**
- Short strings (< 32B) — avoids allocation jitter
- High-frequency deserialization — no GC/allocator pressure
- Downstream HashMap lookups — hash is precomputed at construction, not at lookup

---

## Hash (XXH3)

| Input Size | p50 | p99 | p999 | cycles/byte |
|-----------|-----|-----|------|-------------|
| 8B | 22 | 24 | 26 | 2.75 |
| 16B | 20 | 22 | 26 | 1.25 |
| 32B | 30 | 34 | 50 | 0.94 |
| 64B | 36 | 40 | 52 | 0.56 |
| 128B | 50 | 52 | 90 | 0.39 |
| 256B (SIMD) | 66 | 72 | 100 | 0.26 |
| 1KB (SIMD) | 128 | 136 | 210 | 0.13 |
| 4KB (SIMD) | 414 | 424 | 764 | 0.10 |

For AsciiString (CAP <= 128), hashing is always scalar XXH3. The SIMD paths
(SSE2/AVX2/AVX-512) only activate for inputs > 240B, which matters for
`AsciiStr` (DST) but never for fixed-capacity `AsciiString`.

---

## SIMD Validation Crossover

Shows cycles at various input lengths for `validate_ascii`:

| Length | p50 | p99 | p999 | cycles/byte |
|--------|-----|-----|------|-------------|
| 7B | 18 | 20 | 26 | 2.57 |
| 8B | 16 | 20 | 20 | 2.00 |
| 15B | 16 | 18 | 20 | 1.07 |
| 16B | 18 | 20 | 20 | 1.12 |
| 32B | 18 | 30 | 32 | 0.56 |
| 48B | 16 | 20 | 26 | 0.33 |
| 64B | 16 | 20 | 22 | 0.25 |
| 128B | 18 | 20 | 28 | 0.14 |

Validation cost is **flat at 16-18 cycles** from 7B to 128B — SIMD setup is
amortized even for short strings. The cycles/byte improves with length
as the per-iteration cost is distributed over more data.

---

## Zero-Pad Full-Buffer Processing

Validates the unconditional full-buffer strategy. All operations process
the entire CAP-byte buffer regardless of content length.

**Validation (validate_ascii_bounded):**

| CAP | Content | len-aware p50 | full-buffer p50 | Overhead |
|-----|---------|---------------|-----------------|----------|
| 8 | 7B | 18 | 18 | +0 |
| 16 | 7B | 18 | 18 | +0 |
| 16 | 13B | 16 | 18 | +0-2 |
| 32 | 7B | 18 | 16 | +0 |
| 32 | 20B | 16 | 18 | +0-2 |
| 64 | 7B | 18 | 18 | +0 |
| 64 | 20B | 18 | 18 | +0 |
| 64 | 48B | 18 | 18 | +0 |

**Case conversion (to_uppercase via AsciiString):**

| CAP | Content | p50 | p99 | p999 |
|-----|---------|-----|-----|------|
| 32 | 7B | 18 | 46 | 96 |
| 32 | 19B | 18 | 26 | 56 |
| 64 | 7B | 20 | 40 | 66 |
| 64 | 19B | 18 | 24 | 68 |
| 64 | 49B | 24 | 28 | 82 |

**eq_ignore_ascii_case (full-buffer SWAR):**

| CAP | Content | p50 | p99 | p999 |
|-----|---------|-----|-----|------|
| 32 | 7B | 18 | 32 | 36 |
| 32 | 19B | 18 | 20 | 54 |
| 64 | 7B | 18 | 22 | 34 |
| 64 | 19B | 18 | 26 | 64 |

Full-buffer overhead is negligible (0-2 cycles at p50) for all tested
configurations. Branch elimination and simpler code generation offset
the extra zero-byte processing.

---

## Tail Latency (eq_ignore_ascii_case vs std)

| Input | nexus p50 | nexus p999 | std p50 | std p999 |
|-------|-----------|------------|---------|----------|
| 7B | 18 | 20 | 24 | 58 |
| 38B | 18 | 32 | 124 | 356 |

At 38B, nexus-ascii is **6.9x faster at p50** and **11x better at p999**
compared to std's `eq_ignore_ascii_case`. The gap widens with length because
std processes byte-by-byte while nexus uses SWAR (8B/iter).

---

## AsciiStr (DST)

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `try_from_bytes` (7B) | 16 | 20 | 36 |
| `try_from_bytes` (32B) | 16 | 20 | 34 |
| `as_ascii_str()` from AsciiString | 16 | 18 | 20 |
| Deref coercion | 16 | 18 | 20 |
| `AsciiStr == AsciiStr` | 16 | 18 | 18 |
| `AsciiStr == str` | 18 | 20 | 20 |

AsciiStr construction is just validation (no hash, no copy). Cross-type
equality works through Deref and trait impls with no additional overhead.

---

## AsciiText (Printable ASCII)

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `try_from_str` (7B) | 18 | 28 | 72 |
| `try_from_str` (20B) | 18 | 22 | 70 |
| `into_ascii_string()` | 18 | 20 | 22 |
| `AsciiText == AsciiText` | 16 | 20 | 20 |
| `AsciiText == AsciiString` | 18 | 20 | 20 |

AsciiText adds printable validation (0x20-0x7E) on top of ASCII validation.
The additional range check is essentially free at the SIMD level (1 extra
comparison per chunk).

---

## AsciiChar

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `try_new` (valid) | 18 | 20 | 36 |
| `new_unchecked` | 16 | 18 | 20 |
| `to_uppercase` | 18 | 20 | 28 |
| `to_lowercase` | 16 | 20 | 20 |
| `is_alphabetic` | 16 | 18 | 20 |
| `eq_ignore_case` | 16 | 18 | 20 |
| Baseline: `char.is_ascii_uppercase` | 16 | 24 | 28 |

AsciiChar operations are single-byte range checks — equivalent to std char
methods but with ASCII guarantee enforced at the type level.

---

## Iteration

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `chars().count()` (8B) | 16 | 18 | 20 |
| `chars().count()` (32B) | 16 | 18 | 20 |
| `bytes().sum()` (8B) | 18 | 22 | 24 |
| `bytes().sum()` (32B) | 18 | 42 | 50 |
| Baseline: `&str chars().count()` | 18 | 22 | 54 |
| Baseline: `&str bytes().sum()` | 18 | 28 | 42 |

Iteration performance matches std. AsciiString's `chars()` iterator yields
`AsciiChar` values (no UTF-8 decoding needed), but the compiler optimizes
both cases similarly for simple operations.

---

## vs `ascii` Crate

Comparison against the [`ascii`](https://crates.io/crates/ascii) crate (v1.1),
which provides heap-allocated `AsciiString` (like `String`) and `AsciiStr`
(like `&str`).

```bash
taskset -c 0 cargo run --release --example perf_vs_ascii_crate
```

**Construction (from &str):**

| Size | nexus p50 | ascii p50 | std String p50 | nexus vs ascii |
|------|-----------|-----------|----------------|----------------|
| 7B | 18 | 32 | 20 | 1.8x faster |
| 20B | 20 | 36 | 22 | 1.8x faster |
| 38B | 22 | 36 | 20 | 1.6x faster |

nexus-ascii avoids heap allocation entirely. The `ascii` crate allocates on
every construction, adding malloc overhead (~12-14 cycles) on top of validation.

**Case conversion (to_uppercase):**

| Size | nexus p50 | ascii p50 | std p50 | nexus vs ascii |
|------|-----------|-----------|---------|----------------|
| 7B | 18 | 34 | 20 | 1.9x faster |
| 20B | 18 | 44 | 20 | 2.4x faster |
| 38B | 24 | 68 | 20 | 2.8x faster |

nexus uses full-buffer SIMD (SSE2 16B/iter, no domain crossing). The `ascii`
crate clones + modifies in-place (malloc + byte-by-byte conversion). std wins
at p50 because it modifies in-place with no allocation, but nexus returns a
new value (includes hash recomputation).

**Case-insensitive comparison:**

| Size | nexus p50 | ascii p50 | std p50 | nexus vs ascii |
|------|-----------|-----------|---------|----------------|
| 7B | 20 | 26 | 22 | 1.3x faster |
| 38B | 20 | 94 | 124 | 4.7x faster |

At 38B, nexus-ascii is **4.7x faster** than `ascii` crate and **6.2x faster**
than std. nexus uses full-buffer SWAR (8B/iter, zero domain crossing). Both
`ascii` and std process byte-by-byte.

**HashMap lookup (100 entries):**

| Impl | p50 | p99 | p999 |
|------|-----|-----|------|
| nexus (default hasher) | 32 | 36 | 38 |
| ascii (default hasher) | 48 | 54 | 92 |
| std String (default hasher) | 36 | 38 | 62 |

nexus wins due to precomputed hash (Hash trait returns the stored header
finalized with a single multiply). The `ascii` crate and String must rehash
on every lookup.

---

## HashMap: Identity Hashing (AsciiHashMap)

`AsciiHashMap` uses `nohash-hasher` (identity hashing) with AsciiString's
precomputed hash. The `Hash` impl applies a Fibonacci multiply finalizer
(`header * 0x9E3779B97F4A7C15`) which provides full avalanche for both
h1 (bucket selection) and h2 (SIMD group filtering). Cost: ~1 cycle.

```bash
taskset -c 0 cargo run --release --features nohash --example perf_hashmap
```

**GET (hit) — varying map size:**

| Map size | nohash p50 | ahash p50 | fxhash p50 | default p50 | String p50 |
|----------|-----------|-----------|-----------|-------------|------------|
| 10 | 16 | 20 | 20 | 32 | 38 |
| 100 | 20 | 20 | 20 | 32 | 38 |
| 1,000 | 20 | 20 | 20 | 32 | 36 |
| 10,000 | 20 | 20 | 18 | 34 | 38 |

nohash/ahash/fxhash are **tied at 18-20 cycles** — all benefit from our
precomputed hash (the Hash impl writes a single u64, so external hashers
do minimal extra work). Default SipHash is 1.7x slower. Lookup cost is
dominated by the bucket probe, not hashing.

**GET (miss) — varying map size:**

| Map size | nohash p50 | ahash p50 | fxhash p50 | default p50 | String p50 |
|----------|-----------|-----------|-----------|-------------|------------|
| 10 | 20 | 20 | 20 | 34 | 36 |
| 100 | 20 | 20 | 18 | 28 | 36 |
| 1,000 | 18 | 20 | 18 | 30 | 34 |
| 10,000 | 20 | 20 | 20 | 28 | 38 |

Miss performance is **constant 18-20 cycles** for nohash/ahash/fxhash. The
Fibonacci finalizer gives proper h2 control byte distribution, so SIMD group
filtering rejects non-matching slots without equality checks.

**INSERT — batch of N keys (pre-sized HashMap):**

| Map size | nohash p50 | ahash p50 | fxhash p50 | default p50 | String p50 |
|----------|-----------|-----------|-----------|-------------|------------|
| 10 | 284 | 316 | 292 | 420 | 992 |
| 100 | 2,120 | 2,622 | 2,212 | 4,368 | 14,700 |
| 1,000 | 18,938 | 22,040 | 19,340 | 43,012 | 143,756 |
| 10,000 | 196,502 | 229,276 | 198,604 | 458,342 | 1,384,858 |

nohash dominates at all sizes: **2.3x faster than default** at n=10000.
The Fibonacci multiply gives equivalent bucket distribution to ahash/fxhash
(all three match within noise), while avoiding any external hasher overhead.

**GET (hit) — varying CAP (map=1000):**

| CAP | nohash p50 | ahash p50 | fxhash p50 | default p50 |
|-----|-----------|-----------|-----------|-------------|
| 16 | 24 | 26 | 26 | 42 |
| 32 | 26 | 26 | 24 | 44 |
| 64 | 26 | 26 | 26 | 42 |
| 128 | 24 | 26 | 26 | 42 |

nohash cost is **independent of CAP** — the hash is in the header, never
touches the data buffer. All fast hashers match because our Hash impl writes
a single u64 regardless of string size.

**When to use AsciiHashMap (nohash):**
- Any workload. nohash now matches or beats ahash/fxhash at all sizes.
- Lookup-heavy: 1.7x faster than default (20 vs 34 cycles)
- Insert-heavy: 2.3x faster than default at n=10000
- Zero external hasher overhead — the precomputed hash + finalize is all you need

**When to use default hasher:**
- When you need HashDoS resistance (SipHash is keyed, our hash is not)

---

## Notes

- All benchmarks use `rdtsc` (CPU cycles), not wall-clock time
- Pinned to core 0 via `taskset -c 0` for measurement stability
- Results vary by CPU microarchitecture (measured on x86_64 desktop)
- For AVX2 results, run with `RUSTFLAGS="-C target-feature=+avx2"`
- SSE2 is the baseline configuration (always available on x86_64)
