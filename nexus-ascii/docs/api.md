# API reference

## Construction

All constructors are fallible — they return `Result<_, AsciiError>` and
validate both length and byte content:

```rust
use nexus_ascii::{AsciiString, AsciiString16, AsciiError};

// From a &str
let sym: AsciiString16 = AsciiString::try_from("BTC-USD")?;

// From bytes
let sym: AsciiString16 = AsciiString::try_from(&b"BTC-USD"[..])?;
# Ok::<(), AsciiError>(())
```

`AsciiError` distinguishes between three failure modes:

```rust
use nexus_ascii::AsciiError;

match AsciiError::TooLong { len: 20, cap: 16 } {
    AsciiError::TooLong { len, cap } => { /* input too big */ }
    AsciiError::InvalidByte { byte, pos } => { /* non-ASCII byte */ }
    AsciiError::NonPrintable { byte, pos } => { /* AsciiText only */ }
}
```

## Builders

When you need to assemble a string from parts, use `AsciiStringBuilder<CAP>`:

```rust
use nexus_ascii::{AsciiStringBuilder32, AsciiString32};

let mut b = AsciiStringBuilder32::new();
b.push_str("BTC-")?;
b.push_str("USD-")?;
b.push_str("PERP")?;
let s: AsciiString32 = b.finalize();
# Ok::<(), nexus_ascii::AsciiError>(())
```

`finalize()` is what triggers the XXH3 hash computation. During building,
the hash is not yet valid — that's why builders and finished strings are
separate types.

## Access

```rust
use nexus_ascii::AsciiString32;

let s: AsciiString32 = AsciiString32::try_from("BTC-USD")?;

assert_eq!(s.as_str(), "BTC-USD");
assert_eq!(s.as_bytes(), b"BTC-USD");
assert_eq!(s.len(), 7);
assert!(!s.is_empty());
# Ok::<(), nexus_ascii::AsciiError>(())
```

`as_str()` is `&str` — infallible, because the type *is* valid ASCII by
construction.

## Equality

```rust
use nexus_ascii::AsciiString32;

let a: AsciiString32 = AsciiString32::try_from("BTC-USD")?;
let b: AsciiString32 = AsciiString32::try_from("BTC-USD")?;
let c: AsciiString32 = AsciiString32::try_from("ETH-USD")?;

assert_eq!(a, b);
assert_ne!(a, c);
# Ok::<(), nexus_ascii::AsciiError>(())
```

The fast path compares the header word first (tag + length + hash). If the
lengths or hashes differ, `PartialEq::eq` returns `false` without looking at
the content. For the common case of comparing different strings, this is a
single load and compare — branchless, cache-local, ~1 cycle amortized.

When the header matches, the content bytes are compared (`memcmp`), which
is still fast for short strings.

## Ordering

`AsciiString` and `AsciiText` implement `Ord` — lexicographic on the
underlying bytes. Note that ordering does *not* use the hash, only the
content. Don't expect hash-order; the precomputed hash is for equality,
not comparison.

## `Hash`

```rust
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use nexus_ascii::AsciiString32;

let s: AsciiString32 = AsciiString32::try_from("BTC-USD")?;
let mut h = DefaultHasher::new();
s.hash(&mut h);
let h_std = h.finish();
# Ok::<(), nexus_ascii::AsciiError>(())
```

The `Hash` impl feeds the precomputed 48-bit hash into the hasher rather than
re-hashing bytes. If the hasher is `nohash_hasher::NoHashHasher`, it uses
the value directly — see [nohash-feature.md](nohash-feature.md).

If the hasher is a real hasher (`SipHash`, `FxHash`, `AHash`), you still
benefit: instead of hashing N bytes, you hash a single u64. `AsciiString`
is dramatically cheaper than `String` as a map key even without `nohash`.

## `AsciiString` vs `AsciiText`

`AsciiString` accepts any byte in 0x01–0x7F:

```rust
use nexus_ascii::AsciiString32;
let s: AsciiString32 = AsciiString32::try_from("BTC\tUSD")?;  // OK, tab is 0x09
# Ok::<(), nexus_ascii::AsciiError>(())
```

`AsciiText` rejects control characters — only 0x20 (space) through 0x7E
(tilde) allowed:

```rust
use nexus_ascii::AsciiText32;
assert!(AsciiText32::try_from("BTC\tUSD").is_err());  // tab rejected
assert!(AsciiText32::try_from("BTC USD").is_ok());    // space is fine
```

Use `AsciiText` when the field is meant for display (symbol, instrument
name, user-facing message) and you want the type to refuse garbage.
Use `AsciiString` for protocol fields and wire-format content that may
legitimately contain delimiters or control bytes.

## `Flat*` variants

`FlatAsciiString<CAP>` and `FlatAsciiText<CAP>` are the same as their
non-`Flat` counterparts but *without* the precomputed hash header. Smaller
(`Flat` wastes no bytes on hash storage), slower for hash-map operations
(full XXH3 every lookup).

Use `Flat` when you're storing thousands of strings but rarely look them
up — for example, as leaf data in a columnar table.
