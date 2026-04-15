# `event_queue` — non-blocking

```rust
use nexus_notify::{event_queue, Notifier, Poller, Token, Events};

let (notifier, poller): (Notifier, Poller) = event_queue(256);
```

`event_queue(max_tokens)` returns a `(Notifier, Poller)` pair. `max_tokens`
is the largest token index you'll ever use; it determines the size of
the dedup flag array and the backing MPSC queue.

`Notifier` is `Clone + Send + Sync` — share it across as many producer
threads as you want. `Poller` is single-consumer; don't clone it.

## Producing

```rust
# use nexus_notify::{event_queue, Token};
# let (notifier, _poller) = event_queue(256);
let tok = Token::new(42);
notifier.notify(tok).unwrap();
```

`notify(token)` is the only producer operation. Fast path:

1. Swap the dedup flag for this token (single `AtomicBool`).
2. If the flag was already `true` → return `Ok(())`. Conflated.
3. If the flag was `false` → push the token index into the MPSC queue.

Total cost: one atomic swap, and one CAS on the queue tail in the
non-conflated case.

The return type is `Result<(), NotifyError>`. `NotifyError` fires only if
the underlying queue overflows, which should be *impossible* given the
flag gate — one notification per token and the queue is sized to hold
every token. An error here indicates a logic bug (the caller is using a
`Token` with an index beyond `max_tokens`).

## Polling

```rust
# use nexus_notify::{event_queue, Token, Events};
# let (notifier, poller) = event_queue(256);
# notifier.notify(Token::new(1)).unwrap();
// Pre-allocate an Events buffer. Reuse it across polls.
let mut events = Events::with_capacity(256);

// Drain everything that's ready right now.
poller.poll(&mut events);

for evt in events.as_slice() {
    let idx = evt.index();
    // dispatch based on idx
    let _ = idx;
}
```

`poll(&mut events)` drains all currently-ready tokens into the buffer.
Each drained token has its flag cleared on pop, re-arming it for future
notifications. The buffer is not reset on entry — use `events.clear()`
before polling, or treat `events` as append-only and slice by length.

### Budgeted poll

```rust
# use nexus_notify::{event_queue, Token, Events};
# let (notifier, poller) = event_queue(256);
# for i in 0..100 { notifier.notify(Token::new(i)).unwrap(); }
let mut events = Events::with_capacity(256);

// Drain at most 32 tokens. Remaining stay in the queue for next time.
poller.poll_limit(&mut events, 32);
assert_eq!(events.len(), 32);
```

`poll_limit(&mut events, limit)` caps how many tokens are drained in one
call. The remaining tokens stay in the queue, preserved in FIFO order,
and drain on the next call. Use this in a poll loop to keep per-tick
latency bounded: "dispatch at most 32 events per event-loop iteration,
then get back to other work."

## The `Events` buffer

```rust
use nexus_notify::Events;

let mut events = Events::with_capacity(256);
assert_eq!(events.len(), 0);
assert!(events.is_empty());

// After poll:
for evt in events.as_slice() {
    let idx = evt.index();
    let _ = idx;
}
```

`Events` is a mio-style pre-allocated event buffer. `as_slice()` gives
you a borrowed slice of the events drained by the last `poll` call.
Keep reusing the same buffer — don't allocate a new one per iteration.

## Capacity sizing

`max_tokens` should be the highest token index you'll use plus one. For
example, if you have 100 WebSocket sessions and assign tokens 0..100,
pass `100` (or round up to the next power of two for friendlier
internals).

Memory cost for `max_tokens = 4096`:
- Dedup flags: 4 KB (one `AtomicBool` per token).
- MPSC ring buffer: ~64 KB (rounded to the next power of two).
- Total: ~68 KB.

This is fixed at construction time — no growth, no reallocation.
