# Overview

`nexus-notify` answers a very specific question: "which of my N wakeup
sources have new work since the last time I checked, and how do I find
out *without* re-polling every one of them?"

## The problem it solves

Imagine you have 100 WebSocket sessions, each feeding market data into a
conflation slot. A strategy thread wants to:

1. Be woken up when *any* session has new data.
2. Learn *which* sessions have new data.
3. Do it in FIFO order so no session starves.
4. Not pay for sessions that haven't changed.

A naive `Arc<Mutex<VecDeque<Token>>>` solves this but is expensive and
doesn't deduplicate: if the same session fires three times before the
strategy reads, it ends up in the queue three times and you do the same
work thrice.

`nexus-notify` is the efficient answer. Each token has a one-bit "dirty"
flag. Producers swap the flag atomically — the first producer to flip it
from `false` to `true` pushes the token into an MPSC ring buffer. Later
producers see the flag already set and do nothing (conflated). The
consumer pops tokens FIFO from the ring buffer and clears the flag on
pop, re-arming the token for future notifications.

The result:

- **Cheap notify.** Fast path is a single atomic swap.
- **Conflated.** Each token fires at most once per poll cycle.
- **FIFO.** Oldest-dirty-first, no starvation.
- **Cross-thread.** Producers and consumer can live on different threads.
- **Bounded memory.** The queue holds at most `max_tokens` entries
  because the flag gates admission.

## Two flavors

- **[`event_queue`](event-queue.md)** — `(Notifier, Poller)`. Non-blocking.
  The consumer polls on its own schedule.
- **[`event_channel`](event-channel.md)** — `(Sender, Receiver)`. Blocking.
  The consumer's `recv` blocks until something is ready.

Both use the same underlying dedup + FIFO queue. `event_channel` is just
`event_queue` wrapped in a parker/unparker for idle-when-empty wakeups.

## Tokens, not handles

A `Token` is a bare `usize` index. It doesn't own anything. The consumer
chooses what a token *means* — usually it's the index of a session, a
slab key for some resource, or an enum-discriminant for "which source
fired". Users assign tokens at setup time.

Every `Notifier` has access to the same flag array, so any thread can
notify any token. The `Poller` (or `Receiver`) is single-consumer — only
one owner drains the queue.

## Spurious wakeups

Because tokens are `usize` indices assigned by the caller, there's a
window during resource teardown where an in-flight `notify()` for an
old-but-freed token can fire after the token is reassigned to a new
resource. The consumer must tolerate spurious wakeups during
deregister: re-check the underlying resource rather than trusting the
token alone. Same contract as mio.

## Single-threaded variant

For cases where producer and consumer live on the same thread,
[`LocalNotify`](local-vs-cross-thread.md) gives you the same semantics
without atomics — same dispatch-order guarantees, same conflation, just
`&mut self` operations and a `Vec<u64>` bitset instead of the cross-
thread machinery.
