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
| `empty()` | 16 | 18 | 32 |
| `try_from` (7B "BTC-USD") | 16 | 24 | 56 |
| `try_from` (20B) | 22 | 26 | 42 |
| `try_from` (32B, full cap) | 20 | 28 | 62 |
| `from_bytes_unchecked` (7B) | 18 | 20 | 24 |

Construction includes: ASCII validation (SIMD) + XXH3 hash computation + inline copy +
zero-padding. At 7B (typical symbol), construction is **16 cycles** — same cost as a
single L1 cache miss.

---

## Equality

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `eq` (same content) | 16 | 20 | 20 |
| `eq` (different content) | 16 | 18 | 20 |
| `eq` (different length) | 16 | 18 | 18 |
| Baseline: `u64 == u64` | 16 | 18 | 18 |

Equality is a single `u64` comparison (packed hash+length header). Matches
the cost of a raw integer compare — **most non-equal strings are rejected
without touching the byte buffer.**

---

## HashMap

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `HashMap::get` (100 entries) | 20 | 22 | 36 |
| `HashMap::insert` (new key) | 178 | 316 | 480 |

Lookups are fast because `Hash` returns the precomputed hash (zero runtime
hashing cost). The 20-cycle p50 is dominated by the HashMap probe, not hashing.

---

## Ordering & Comparison

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `cmp()` equal (7B) | 18 | 20 | 34 |
| `cmp()` different (7B) | 16 | 20 | 34 |
| `cmp()` different lengths (7B vs 3B) | 18 | 18 | 20 |
| `cmp()` equal (37B) | 16 | 18 | 18 |
| `cmp()` differ at end (37B) | 16 | 18 | 20 |
| `eq_ignore_ascii_case()` same case (7B) | 16 | 18 | 20 |
| `eq_ignore_ascii_case()` diff case (7B) | 16 | 18 | 18 |
| `eq_ignore_ascii_case()` same case (38B) | 16 | 18 | 20 |
| `eq_ignore_ascii_case()` diff case (38B) | 18 | 20 | 26 |
| `eq_ignore_ascii_case()` same case (69B) | 18 | 20 | 48 |
| `eq_ignore_ascii_case()` diff case (69B) | 22 | 26 | 28 |
| `starts_with()` 3B prefix (7B string) | 18 | 20 | 20 |
| `ends_with()` 3B suffix (7B string) | 16 | 18 | 22 |
| `contains()` 1B needle (7B string) | 16 | 20 | 36 |

**Baselines (std `&str`):**

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `[u8] cmp()` equal (baseline) | 20 | 22 | 24 |
| `[u8] cmp()` different (baseline) | 18 | 22 | 22 |
| `&str eq_ignore_ascii_case` (7B) | 22 | 26 | 56 |
| `&str eq_ignore_ascii_case` (38B) | 114 | 122 | 212 |
| `&str starts_with` | 18 | 20 | 22 |
| `&str ends_with` | 18 | 20 | 20 |
| `&str contains` | 16 | 18 | 22 |

`cmp()` uses word-at-a-time comparison (u64 loads with `bswap` for
lexicographic ordering). Full-buffer processing means no remainder loops.

`eq_ignore_ascii_case` uses full-buffer SWAR for < 64B (zero domain-crossing
overhead) and SSE2/AVX2 for >= 64B. At 38B, nexus-ascii is **6-7x faster**
than std at p50 (18 vs 114 cycles).

---

## Case Conversion

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `to_ascii_uppercase` (7B) | 18 | 20 | 34 |
| `to_ascii_lowercase` (7B) | 18 | 18 | 34 |
| `to_ascii_uppercase` (20B) | 16 | 48 | 94 |
| `to_ascii_lowercase` (20B) | 16 | 20 | 44 |
| `to_ascii_uppercase` (32B) | 16 | 18 | 44 |
| `to_ascii_lowercase` (32B) | 18 | 20 | 40 |

**Baselines (std in-place):**

| Operation | p50 | p99 | p999 |
|-----------|-----|-----|------|
| `std make_ascii_uppercase` (20B) | 16 | 18 | 18 |
| `std make_ascii_lowercase` (20B) | 16 | 18 | 20 |

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
| `truncated` (54B -> 5B) | 16 | 18 | 20 |
| `truncated` (54B -> 30B) | 16 | 28 | 32 |
| `truncated` (53B -> 53B, no change) | 20 | 46 | 92 |
| `try_truncated` (54B -> 5B) | 18 | 20 | 20 |
| `try_truncated` (fails) | 18 | 20 | 28 |

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
| `AsciiString<32>` from JSON | 50 | 68 | 126 |
| `String` from JSON | 50 | 122 | 138 |
| `&str` from JSON (borrowed) | 30 | 98 | 100 |
| `try_from_str` (no JSON) | 20 | 50 | 94 |
| **20B (order IDs)** |
| `AsciiString<32>` from JSON | 50 | 104 | 160 |
| `String` from JSON | 50 | 120 | 210 |
| `&str` from JSON (borrowed) | 28 | 32 | 66 |
| `try_from_str` (no JSON) | 22 | 24 | 30 |
| **38B (long identifiers)** |
| `AsciiString<64>` from JSON | 70 | 144 | 148 |
| `String` from JSON | 62 | 130 | 260 |
| `&str` from JSON (borrowed) | 48 | 54 | 98 |
| `try_from_str` (no JSON) | 22 | 24 | 38 |
| **64B (large protocol fields)** |
| `AsciiString<128>` from JSON | 84 | 174 | 306 |
| `String` from JSON | 68 | 144 | 264 |
| `&str` from JSON (borrowed) | 56 | 66 | 104 |
| `try_from_str` (no JSON) | 18 | 22 | 32 |

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
| 32B | 28 | 32 | 42 | 0.88 |
| 64B | 34 | 38 | 56 | 0.53 |
| 128B | 46 | 48 | 82 | 0.36 |
| 256B (SIMD) | 60 | 66 | 94 | 0.23 |
| 1KB (SIMD) | 120 | 134 | 216 | 0.12 |
| 4KB (SIMD) | 364 | 386 | 694 | 0.09 |

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
| 7B | 16 | 34 | 22 | 56 |
| 38B | 18 | 30 | 114 | 212 |

At 38B, nexus-ascii is **6.3x faster at p50** and **7x better at p999**
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
| 7B | 18 | 30 | 20 | 1.7x faster |
| 20B | 18 | 34 | 20 | 1.9x faster |
| 38B | 20 | 32 | 18 | 1.6x faster |

nexus-ascii avoids heap allocation entirely. The `ascii` crate allocates on
every construction, adding malloc overhead (~12-14 cycles) on top of validation.

**Case conversion (to_uppercase):**

| Size | nexus p50 | ascii p50 | std p50 | nexus vs ascii |
|------|-----------|-----------|---------|----------------|
| 7B | 18 | 32 | 18 | 1.8x faster |
| 20B | 18 | 40 | 18 | 2.2x faster |
| 38B | 28 | 62 | 18 | 2.2x faster |

nexus uses full-buffer SIMD (SSE2 16B/iter, no domain crossing). The `ascii`
crate clones + modifies in-place (malloc + byte-by-byte conversion). std wins
at p50 because it modifies in-place with no allocation, but nexus returns a
new value (includes hash recomputation).

**Case-insensitive comparison:**

| Size | nexus p50 | ascii p50 | std p50 | nexus vs ascii |
|------|-----------|-----------|---------|----------------|
| 7B | 18 | 22 | 20 | 1.2x faster |
| 38B | 18 | 84 | 112 | 4.7x faster |

At 38B, nexus-ascii is **4.7x faster** than `ascii` crate and **6.2x faster**
than std. nexus uses full-buffer SWAR (8B/iter, zero domain crossing). Both
`ascii` and std process byte-by-byte.

**HashMap lookup (100 entries):**

| Impl | p50 | p99 | p999 |
|------|-----|-----|------|
| nexus (default hasher) | 32 | 36 | 70 |
| ascii (default hasher) | 46 | 50 | 108 |
| std String (default hasher) | 34 | 82 | 166 |

nexus wins due to precomputed hash (Hash trait returns stored value, zero
runtime hashing). The `ascii` crate and String must rehash on every lookup.

---

## HashMap: Identity Hashing (AsciiHashMap)

`AsciiHashMap` uses `nohash-hasher` (identity hashing) with AsciiString's
precomputed 48-bit XXH3 hash. Zero hashing cost at lookup time — the stored
hash IS the bucket index.

```bash
taskset -c 0 cargo run --release --features nohash --example perf_hashmap
```

**GET (hit) — varying map size:**

| Map size | nohash p50 | default p50 | String p50 | nohash vs default |
|----------|-----------|-------------|------------|-------------------|
| 10 | 18 | 28 | 34 | 1.6x faster |
| 100 | 18 | 30 | 36 | 1.7x faster |
| 1,000 | 18 | 30 | 36 | 1.7x faster |
| 10,000 | 18 | 30 | 36 | 1.7x faster |

nohash lookup is **constant 18 cycles** regardless of map size. The lookup
cost is dominated by the bucket probe, not hashing. With identity hashing,
there's zero hash computation — just read the precomputed header.

**GET (miss) — varying map size:**

| Map size | nohash p50 | default p50 | String p50 |
|----------|-----------|-------------|------------|
| 10 | 32 | 28 | 36 |
| 100 | 38 | 28 | 36 |
| 1,000 | 38 | 28 | 34 |
| 10,000 | 34 | 28 | 34 |

Miss performance is comparable across hashers. The miss path is dominated by
bucket probing and equality checks, not hash computation.

**INSERT — batch of N keys:**

| Map size | nohash p50 | default p50 | String p50 |
|----------|-----------|-------------|------------|
| 10 | 406 | 406 | 840 |
| 100 | 4,116 | 4,176 | 14,146 |
| 1,000 | 33,772 | 41,762 | 147,024 |
| 10,000 | 611,150 | 405,652 | 1,437,268 |

At n <= 1000, nohash matches or beats default hasher. String is 2-4x slower
due to allocation overhead per key. At n=10000, default hasher wins on insert
due to better hash distribution reducing probe chains during table growth.

**GET (hit) — varying CAP (map=1000):**

| CAP | nohash p50 | default p50 |
|-----|-----------|-------------|
| 16 | 20 | 32 |
| 32 | 20 | 32 |
| 64 | 20 | 32 |
| 128 | 20 | 34 |

nohash cost is **independent of CAP** — the hash is in the header, never
touches the data buffer. Default hasher (SipHash) must process more bytes
as CAP grows, showing slight degradation.

**When to use AsciiHashMap (nohash):**
- Read-heavy workloads (the 1.7x get-hit advantage dominates)
- Fixed key sets populated at startup (insert distribution doesn't matter)
- Latency-sensitive lookups where 18 vs 30 cycles per lookup compounds

**When to use default hasher:**
- Write-heavy workloads with large maps (> 10k entries)
- When hash distribution quality matters more than lookup speed

---

## Notes

- All benchmarks use `rdtsc` (CPU cycles), not wall-clock time
- Pinned to core 0 via `taskset -c 0` for measurement stability
- Results vary by CPU microarchitecture (measured on x86_64 desktop)
- For AVX2 results, run with `RUSTFLAGS="-C target-feature=+avx2"`
- SSE2 is the baseline configuration (always available on x86_64)
