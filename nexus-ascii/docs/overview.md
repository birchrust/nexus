# Overview

`nexus-ascii` exists for one specific use case: you have short, immutable
ASCII strings (ticker symbols, order IDs, protocol fields, enum discriminants)
that you want to store, compare, and hash *without touching the allocator*.

## The fundamental bet

ASCII strings that fit in a cache line or two have very different
characteristics than general-purpose `String`:

- They never grow or shrink — once you have the bytes, they're final.
- They're compared against each other millions of times per second.
- They're used as hash-map keys where hash cost dominates lookup cost.
- They're almost always small (≤ 32 bytes).

`AsciiString<CAP>` is designed around those assumptions:

- **Stack-allocated** — `[u8; CAP]` inline, no heap, no drop glue.
- **`Copy`** — cheap to pass by value, no move semantics to reason about.
- **Precomputed hash** — 48 bits of XXH3 computed once at construction,
  stored in the header alongside length and tag. Hashing a key at lookup
  time is a single load.
- **Fast-path equality** — the first 8 bytes of the header (tag + len + hash)
  are compared before any byte-by-byte content comparison. Most inequal
  strings fail out in a single u64 compare.
- **ASCII-only** — no UTF-8 validation, no multi-byte codepoints. If your
  data isn't ASCII, you need a different crate.

## Comparison with the alternatives

| Crate | Heap? | `Copy`? | Precomputed hash? | Grows? |
|-------|------|---------|------------------|-------|
| `String` | yes | no | no | yes |
| `SmolStr` | sometimes (>23 bytes) | no | no | no |
| `heapless::String<N>` | no | no | no | yes (bounded) |
| `arrayvec::ArrayString<N>` | no | yes | no | yes (bounded) |
| **`nexus_ascii::AsciiString<N>`** | **no** | **yes** | **yes** | **no** |

The precomputed hash is the thing that makes `nexus-ascii` different. It's
the reason `AsciiString` beats every other option for hot hash-map keys —
see [nohash-feature.md](nohash-feature.md).

## Families of types

Four families, each answering a slightly different question:

- **`AsciiString<CAP>`** — general-purpose. Accepts any ASCII byte 0x01–0x7F
  (null is structural). Has the precomputed hash header.
- **`AsciiText<CAP>`** — like `AsciiString`, but rejects non-printable bytes
  (anything below 0x20 or above 0x7E). Use when you want a stronger type
  for UI-facing fields.
- **`FlatAsciiString<CAP>`** — same content rules as `AsciiString`, but
  without the precomputed hash. Smaller header, better when you're storing
  *lots* of them and don't need fast hashing.
- **`FlatAsciiText<CAP>`** — `AsciiText` without the precomputed hash.

Pick based on: *do I need fast hashing (AsciiString/Text) or dense storage
(FlatAsciiString/Text)?*

## Capacity choice

Pre-built type aliases cover the common sizes: `AsciiString8`, `16`, `32`,
`64`, `128`, `256`. Pick the smallest one that fits your longest legal
input. Exchange symbols fit in 16. FIX tags fit in 8. Order IDs typically
fit in 32.

Capacity is a const generic — using `AsciiString<24>` works fine if a
standard size is wasteful.
