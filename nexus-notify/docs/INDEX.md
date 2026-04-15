# nexus-notify documentation

Event-driven dispatch primitives with per-token deduplication. A producer
signals "token N is ready"; the consumer discovers which tokens fired and
handles them. Duplicate notifications to the same token before the
consumer polls are conflated away.

## Contents

- [overview.md](overview.md) — what this is for and the conflation guarantee
- [event-queue.md](event-queue.md) — non-blocking `event_queue` / `Notifier`
  / `Poller`
- [event-channel.md](event-channel.md) — blocking `event_channel` /
  `Sender` / `Receiver`
- [local-vs-cross-thread.md](local-vs-cross-thread.md) — `LocalNotify` vs
  cross-thread, when each is appropriate
- [patterns.md](patterns.md) — cookbook: dirty-set tracking, change
  notifications, signal coalescing

## Related crates

- [`nexus-slot`](../../nexus-slot) — latest-value conflation; pairs well
  with notify to deliver "latest data + wakeup"
- [`nexus-queue`](../../nexus-queue) — the underlying MPSC ring buffer
- mio — same `Token` pattern, notify is the cross-thread complement
