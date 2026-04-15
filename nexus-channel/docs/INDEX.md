# nexus-channel documentation

Blocking SPSC channel built on [`nexus-queue`](../../nexus-queue). Three-phase
backoff (spin → yield → park) with conditional parking to minimize wakeup
syscalls.

## Contents

- [overview.md](overview.md) — when to reach for a channel vs a raw queue
- [api.md](api.md) — `Sender`, `Receiver`, `send`/`recv`, try and timeout
  variants
- [backoff.md](backoff.md) — the three-phase backoff strategy and
  `channel_with_config`
- [patterns.md](patterns.md) — cookbook: producer-consumer pairs, pipelines,
  event-loop handoff

## Related crates

- [`nexus-queue`](../../nexus-queue) — the lock-free SPSC ring buffer
  underneath
- [`nexus-slot`](../../nexus-slot) — latest-value conflation instead of
  every-value FIFO
- [`nexus-notify`](../../nexus-notify) — event-driven dispatch with dedup
- `crossbeam-channel` — the MPMC alternative
