# nexus-ascii documentation

Fixed-capacity, stack-allocated ASCII strings with a precomputed 48-bit XXH3
hash in the header. `Copy`, immutable, `no_std`-compatible.

## Contents

- [overview.md](overview.md) — when to reach for `AsciiString` instead of
  `String`, `heapless::String`, or `SmolStr`
- [api.md](api.md) — construction, comparison, hashing, `AsciiString` vs
  `AsciiText` vs `FlatAsciiString`
- [nohash-feature.md](nohash-feature.md) — the `nohash` feature and identity
  hashing
- [patterns.md](patterns.md) — cookbook: symbols, protocol fields, hot map
  keys

## Related crates

- [`nexus-id`](../../nexus-id) — `TypeId` stores an `AsciiString` prefix
- [`nexus-collections`](../../nexus-collections) — `AsciiString` keys with
  identity hashing are a common pairing
