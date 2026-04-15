# String-encoded IDs

For cases where an ID needs to land in a URL, log line, protocol field, or
human-readable identifier, `nexus-id` provides several string encodings over
a `u64` (or a prefix + u64 pair). All of them parse and format without
allocation, using SIMD where the encoding allows.

## HexId64

16-character lowercase hex. The simplest encoding, one byte per nibble.

```rust
use nexus_id::HexId64;

let id = HexId64::from_u64(0xdead_beef_cafe_babe);
assert_eq!(id.as_str(), "deadbeefcafebabe");

let parsed: HexId64 = "deadbeefcafebabe".parse()?;
assert_eq!(parsed.to_u64(), 0xdead_beef_cafe_babe);
# Ok::<(), nexus_id::DecodeError>(())
```

Encode uses SSSE3 shuffle + add (one u64 → 16 chars in ~4 cycles). Decode
uses SSE2 nibble masking. Faster than `format!("{:016x}", x)` by an order
of magnitude, and never touches the allocator.

**Use when:** you want a stable, short, unambiguous hex form. Logs,
debug output, protocol fields that specify hex.

## Base62Id — 11-char alphanumeric

```rust
use nexus_id::Base62Id;

let id = Base62Id::from_u64(1234567890);
assert_eq!(id.as_str().len(), 11);

let parsed: Base62Id = id.as_str().parse()?;
# Ok::<(), nexus_id::DecodeError>(())
```

Alphabet: `0-9A-Za-z`. 11 characters is enough to represent any `u64`
(log₆₂(2⁶⁴) ≈ 10.75). Good choice for short URLs (YouTube-style) and
anywhere you want dense, URL-safe IDs without hyphens.

## Base36Id — 13-char case-insensitive

```rust
use nexus_id::Base36Id;

let id = Base36Id::from_u64(1234567890);
assert_eq!(id.as_str().len(), 13);
```

Alphabet: `0-9a-z`. Case-insensitive on parse. Slightly longer than Base62
but friendlier when humans might retype the ID (e.g. voice, handwriting,
printed support codes).

## TypeId — prefixed, sortable, Stripe-style

```rust
use nexus_id::TypeId;

let id: TypeId<8> = TypeId::from_parts("ord", 0x0192_ab34_cdef_5678)?;
println!("{}", id);  // "ord_01J2MB6KFF2TM..."

let (prefix, value) = id.into_parts();
assert_eq!(prefix.as_str(), "ord");
# Ok::<(), Box<dyn std::error::Error>>(())
```

The format is `<prefix>_<crockford-base32-u64>` with a separator underscore,
inspired by Stripe's `cus_*` and `pi_*` IDs. The prefix is an
[`AsciiString`](../../nexus-ascii) of const-generic capacity. The value portion
is ULID-alphabet Base32, so IDs are sortable when generated from a
time-based source (e.g. `UuidV7` → `u64` → `TypeId`).

Great for API-facing IDs where you want the type to be obvious from the
string: `ord_...` is an order, `fill_...` is a fill, `pos_...` is a
position. No guessing from a bare number.

## Choosing an encoding

| Encoding | Chars | Alphabet | Best for |
|----------|-------|----------|----------|
| `HexId64` | 16 | `0-9a-f` | Logs, wire protocols, debug |
| `Base62Id` | 11 | `0-9A-Za-z` | Short URLs, dense IDs |
| `Base36Id` | 13 | `0-9a-z` | Human-typed IDs, case-insensitive |
| `TypeId<N>` | `N+22` | prefix + Crockford Base32 | Public API IDs with type tag |

All four are `Copy`, `no_std`-compatible, and zero-allocation.
