# `LocalNotify` vs cross-thread

Two implementations of the same semantics:

- **`event_queue` / `event_channel`** — cross-thread, atomic-based.
- **`LocalNotify`** — single-threaded, `&mut self`-based.

Same dispatch model (tokens, dedup, FIFO, budgeted drain), totally
different internals.

## `LocalNotify` at a glance

```rust
use nexus_notify::{local::LocalNotify, Token, Events};

let mut notify = LocalNotify::with_capacity(4);
let mut events = Events::with_capacity(4);

let t0 = notify.register();
let t1 = notify.register();

notify.mark(t0);
notify.mark(t1);
notify.mark(t0);  // deduped

notify.poll(&mut events);
assert_eq!(events.len(), 2);
```

- `register()` returns a fresh `Token`. Allocated from a monotonic
  counter; grows the internal bitset when needed.
- `mark(token)` is the local equivalent of `notify` — sets a bit, pushes
  the index onto the dispatch list if the bit was previously clear.
- `poll(&mut events)` drains the dispatch list and clears all bits.
- `poll_limit(&mut events, limit)` is the budgeted variant.

Internally: one `Vec<u64>` bitset (dedup) and one `Vec<usize>` dispatch
list (FIFO). No atomics.

## Performance

| Operation | Cross-thread (`event_queue`) | `LocalNotify` |
|-----------|------------------------------|---------------|
| `notify`/`mark` (new) | ~16 cy | ~5–7 cy |
| `notify`/`mark` (dedup) | ~16 cy | ~5–7 cy |
| `poll` per token | ~5 cy | ~2 cy |
| Frame clear overhead | — | <1 cy amortized |

`LocalNotify` is roughly 2–3× faster than cross-thread in the hot case
because it doesn't touch atomics. The difference only matters if notify
dispatch is in your flame graph.

## When to use which

Use `LocalNotify` when:

- The producer and consumer live on the same thread.
- You're inside an event loop where everything is single-threaded.
- You want "dirty set" tracking for a batch of things to process at
  the end of the current iteration.

Use cross-thread (`event_queue` / `event_channel`) when:

- Producers are on different threads from the consumer (IO thread
  feeding a strategy thread, for example).
- You can't guarantee single-threaded discipline.

## Single-threaded bounds-check warning

`LocalNotify::register()` grows the internal bitset lazily — the first
`N` registers are cheap, then every power-of-two boundary crossing
reallocates the bitset. If you know your token count up front, pass a
good `capacity` to `with_capacity` so the allocation happens once at
setup time:

```rust
use nexus_notify::local::LocalNotify;

// Pre-sized for 4096 tokens — no growth under load.
let mut notify = LocalNotify::with_capacity(4096);
```

Growth is not catastrophic (the existing bits are preserved) but it's
an allocation on what was meant to be a hot path. Size it once.

## Mixing with cross-thread

You can absolutely have both in the same program: `LocalNotify` for
intra-loop dirty tracking and `event_queue` for wakeups from other
threads. They don't share state, so there's no coordination cost.

```rust
use nexus_notify::{event_queue, local::LocalNotify, Events, Token};

// Wakeups from IO thread
let (io_notifier, io_poller) = event_queue(256);

// Dirty tracking inside the event loop
let mut local = LocalNotify::with_capacity(1024);

// ... in the event loop:
let mut cross_events = Events::with_capacity(256);
let mut local_events = Events::with_capacity(1024);

io_poller.poll(&mut cross_events);
// handle cross-thread events, possibly marking local tokens
local.poll(&mut local_events);
// handle local events
```
