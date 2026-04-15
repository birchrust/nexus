# nexus-slot documentation

Single-writer conflation slots: the writer overwrites, each reader gets
every *new* value exactly once. Seqlock-based, `Pod`-only.

## Contents

- [overview.md](overview.md) — what conflation is, why this is not a
  queue of size 1, and when to reach for it
- [api.md](api.md) — `spsc::slot` and `spmc::shared_slot`, the `Pod`
  trait, read/write semantics
- [patterns.md](patterns.md) — cookbook: latest-price snapshots, config
  updates, sensor readings

## Related crates

- [`nexus-queue`](../../nexus-queue) — when you actually need every value
- [`nexus-channel`](../../nexus-channel) — blocking SPSC
- [`nexus-notify`](../../nexus-notify) — pair a slot with a notify token
  to get "latest data + wakeup when fresh"
