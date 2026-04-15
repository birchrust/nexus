# Backoff strategy

The blocking paths in `send`/`recv`/`recv_timeout` progress through three
phases before touching the kernel. Each phase is designed to handle a
different scenario without paying the cost of the next one.

## Phase 1: fast path

```text
try queue op → success → return
```

The first thing every blocking operation does is try the underlying
queue. If the queue isn't full (`send`) or isn't empty (`recv`), we
return immediately. This is the path taken by ~99% of operations in
steady state.

No atomics beyond the queue itself. No syscalls. No spinning.

## Phase 2: spin with backoff

```text
try queue op → fail → crossbeam_utils::Backoff::snooze() × N
```

If the fast path fails, the operation enters a bounded spin loop using
`crossbeam_utils::Backoff::snooze()`. This yields the CPU to hyperthreads
on the same core via the `PAUSE` instruction (on x86), then gradually
escalates to `yield_now()`.

The number of iterations is controlled by `snooze_iters`, which defaults
to **8**. After each iteration, the operation retries the queue. Typical
contention windows (one side is a few cycles behind the other) resolve
inside this phase without ever calling into the kernel.

Configured per-channel:

```rust
use nexus_channel::channel_with_config;

// Aggressive: spin longer before parking. Burns more CPU but lower tail.
let (tx, rx) = channel_with_config::<u64>(1024, 64);

// Conservative: park quickly. Better for idle-heavy workloads.
let (tx, rx) = channel_with_config::<u64>(1024, 2);
```

Higher `snooze_iters` favor latency (never park when the counterparty is
about to wake up). Lower values favor CPU efficiency on channels that
spend most of their time idle.

## Phase 3: park

```text
register as parked → final retry → thread::park() → wake → retry
```

If spinning doesn't resolve the operation, the thread registers itself as
parked (sets a `CachePadded<AtomicBool>` in the shared state) and calls
`std::thread::park()`. The other side's `send` / `recv` sees the parked
flag and issues `Unparker::unpark()` — the *only* syscall in the whole
flow.

The final retry between "register as parked" and "park()" is critical:
without it, there's a race where the other side's value lands between
the previous retry and the park, and the parked thread sleeps forever.

## Conditional parking — the whole point

Most channel implementations unpark on every send/recv transition
regardless of whether the other side is actually sleeping. That's
defensive — it always works — but it pays for a syscall on operations
that didn't need one.

`nexus-channel` checks the parked flag before unparking:

```rust,ignore
// Conceptual — after a successful send()
if shared.receiver_parked.load(Acquire) {
    receiver_unparker.unpark();  // only this path touches the kernel
}
```

When both sides are busy (the common hot path), neither is parked, and
no unparks fire. The p50 stays in the ~600 cycle range and only the rare
burst-end cases hit the kernel.

This is why the p999 is 14.8× better than crossbeam: the tail is
dominated by avoided syscalls, not by avoided contention.

## Picking `snooze_iters`

| Workload | Suggestion |
|----------|-----------|
| Hot pipeline, continuous throughput | 16–64 |
| Bursty producer with idle gaps | 4–8 (default) |
| Mostly idle, occasional messages | 1–2 |
| Latency-critical, CPU abundant | 128+ |

Measure before tuning. The default of 8 is already good for most cases.

## What the backoff is not

- **Not adaptive.** The snooze iterations are fixed per channel. The
  implementation doesn't learn from past contention.
- **Not priority-aware.** There's no concept of "urgent" messages that
  skip the queue.
- **Not cross-channel.** Each channel has its own pair of park flags;
  you can't share a wakeup source across channels. If you want that,
  look at [`nexus-notify`](../../nexus-notify).
