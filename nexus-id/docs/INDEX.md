# nexus-id documentation

High-performance unique ID generators for low-latency systems. No syscalls on
the hot path, stack-allocated output, optional `no_std`.

## Contents

- [overview.md](overview.md) — picking an ID type, performance at a glance
- [snowflake.md](snowflake.md) — `Snowflake64`/`Snowflake32`, const-generic bit layout
- [uuids.md](uuids.md) — `UuidV4`, `UuidV7`, `Ulid` — time-based vs random
- [mixed-id.md](mixed-id.md) — `MixedId64` with Fibonacci mixing for identity hashers
- [string-ids.md](string-ids.md) — `HexId64`, `Base62Id`, `Base36Id`, `TypeId`
- [patterns.md](patterns.md) — cookbook: client order IDs, tracing, content hashes

## Related crates

- [`nexus-ascii`](../../nexus-ascii) — fixed-capacity ASCII strings (good
  companion for `TypeId` prefixes or wire-format IDs)
- [`nexus-collections`](../../nexus-collections) — hash-map-friendly IDs
  pair well with `MixedId64` and `nohash_hasher`
