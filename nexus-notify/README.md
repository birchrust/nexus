# nexus-notify

Cross-thread event queue with conflation and FIFO delivery.

## What it does

An IO thread writes data into shared storage and calls `notify(token)`. The
main event loop calls `poll()` to discover which tokens fired. Duplicate
notifications between polls are suppressed — the consumer sees each token at
most once.

Two primitives:

- **`event_queue(n)`** → `(Notifier, Poller)` — non-blocking. The consumer
  polls when it chooses.
- **`event_channel(n)`** → `(Sender, Receiver)` — blocking. The consumer
  blocks when idle and is woken by the producer.

## Quick Start

```rust
use nexus_notify::{event_queue, Token, Events};

let (notifier, poller) = event_queue(256);
let mut events = Events::with_capacity(256);

// Producer: signal readiness
notifier.notify(Token::new(42)).unwrap();
notifier.notify(Token::new(7)).unwrap();

// Consumer: discover what's ready
poller.poll(&mut events);
for token in &events {
    println!("token {} is ready", token.index());
}
```

## Conflation

If token 42 is notified 100 times between polls, the consumer sees it once.
The per-token `AtomicBool` flag gates admission to the queue — if already
set, `notify()` is a no-op.

## Budgeted Polling

`poll_limit` drains up to N tokens per call. Remaining items stay in the
queue for the next poll. Oldest notifications drain first (FIFO) — no
starvation under budget.

```rust
// Process at most 32 notifications per loop iteration
poller.poll_limit(&mut events, 32);
```

## Blocking Channel

For consumers that want to sleep when idle:

```rust
use nexus_notify::{event_channel, Token, Events};
use std::thread;

let (sender, receiver) = event_channel(256);
let mut events = Events::with_capacity(256);

// Producer thread
let s = sender.clone();
thread::spawn(move || {
    s.notify(Token::new(42)).unwrap();
});

// Consumer blocks until events arrive
receiver.recv(&mut events);
```

Three-phase wait: fast poll → backoff → park. The sender wakes a parked
receiver automatically.

## Architecture

Per-token `AtomicBool` dedup flags + nexus-queue MPSC ring buffer. The flag
gates admission (conflation). The queue delivers tokens in FIFO order.

- **Notify (conflated):** single atomic swap, ~16 cycles
- **Notify (new):** swap + CAS push, ~16 cycles
- **Poll empty:** ~2 cycles
- **Poll N tokens:** ~5 cycles/token
- **Poll limit:** O(limit), not O(capacity)

## Design

Follows the mio pattern: `poll(&mut Events)` fills a reusable buffer.
`Token` is an opaque handle the user creates from their own key space
(slab keys, array indices). The event queue never assigns tokens — it
only validates bounds.

Tokens are stable as long as the underlying key remains valid. If a slab
key is freed and reassigned, the consumer must tolerate spurious wakeups
during the transition — same contract as mio.
