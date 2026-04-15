# `event_channel` — blocking

```rust
use nexus_notify::{event_channel, Sender, Receiver, Token, Events};

let (sender, receiver): (Sender, Receiver) = event_channel(256);
```

`event_channel(max_tokens)` returns `(Sender, Receiver)`. Same dedup and
FIFO semantics as [`event_queue`](event-queue.md), but the `Receiver`
*blocks* when the queue is empty and wakes up when a `Sender` fires a
new notification.

Use `event_channel` when the consumer is dedicated to processing
notifications and has nothing else to do while waiting. Use `event_queue`
when the consumer has its own poll loop and wants to integrate notify
dispatch into its own scheduling.

## Sending

```rust
# use nexus_notify::{event_channel, Token};
# let (sender, _receiver) = event_channel(256);
sender.notify(Token::new(0)).unwrap();
```

Same semantics as `Notifier::notify` — atomic flag swap, conditional
push, conflation. Plus: if the `Receiver` is currently parked, send an
unpark after the queue push.

## Receiving

### Blocking `recv`

```rust
# use nexus_notify::{event_channel, Token, Events};
# let (sender, receiver) = event_channel(256);
# sender.notify(Token::new(1)).unwrap();
let mut events = Events::with_capacity(256);

receiver.recv(&mut events);
// `events` now contains at least one token (possibly more — the recv
// wakes up and then drains everything that's ready).
```

`recv(&mut events)` blocks until at least one token is ready, then
drains all currently-ready tokens. The first ready token wakes the
receiver; the drain scoops up any that arrived in the meantime.

### Budgeted blocking `recv_limit`

```rust
# use nexus_notify::{event_channel, Token, Events};
# let (sender, receiver) = event_channel(256);
# for i in 0..50 { sender.notify(Token::new(i)).unwrap(); }
# let mut events = Events::with_capacity(256);
receiver.recv_limit(&mut events, 16);
```

Blocks until at least one token is ready, then drains *up to* `limit`
tokens. Remaining tokens stay in the queue for the next call.

### Timed `recv_timeout`

```rust
use std::time::Duration;
# use nexus_notify::{event_channel, Token, Events};
# let (sender, receiver) = event_channel(256);
# let mut events = Events::with_capacity(256);
let got_any = receiver.recv_timeout(&mut events, Duration::from_millis(100));
if got_any {
    // Tokens were drained into `events`.
} else {
    // Timeout elapsed without any notifications.
}
```

Returns `true` if any tokens were drained before the deadline, `false`
on timeout. `recv_timeout_limit` is the budgeted variant.

### Non-blocking `try_recv`

```rust
# use nexus_notify::{event_channel, Events};
# let (_sender, receiver) = event_channel(256);
# let mut events = Events::with_capacity(256);
receiver.try_recv(&mut events);
// events.len() may be 0 if nothing was ready.
```

`try_recv` and `try_recv_limit` never block. They're equivalent to
`Poller::poll` on the non-blocking variant — useful when you want the
blocking behavior *most* of the time but occasionally need to poll
without committing to a wait.

## When to reach for `event_channel` vs `event_queue`

| Situation | Pick |
|-----------|------|
| Dedicated consumer thread, nothing else to do | `event_channel` |
| Consumer has its own poll loop (mio, custom) | `event_queue` |
| Integrating with a runtime that owns idle detection | `event_queue` |
| You want the simplest possible "wake me up" API | `event_channel` |

`event_channel` is strictly more expensive than `event_queue` on the fast
path (because of the parker/unparker bookkeeping) but the difference is
noise compared to the underlying queue cost.
