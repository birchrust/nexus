# API reference

## Creating a channel

```rust
use nexus_channel::channel;

let (tx, rx) = channel::<u64>(1024);
```

`channel::<T>(capacity)` returns `(Sender<T>, Receiver<T>)`. Capacity is
rounded up to the next power of two internally (ring-buffer requirement),
so `channel::<u64>(100)` gives you a 128-slot queue.

Panics if `capacity == 0`.

For custom backoff configuration, use
[`channel_with_config`](backoff.md).

## `Sender<T>`

### `send`

```rust
# use nexus_channel::{channel, SendError};
# let (tx, _rx) = channel::<u64>(4);
match tx.send(42u64) {
    Ok(()) => {}
    Err(SendError(value)) => {
        // Receiver was dropped — `value` is the item you tried to send
        let _ = value;
    }
}
```

Blocks until either:

1. There's space in the queue (send succeeds, returns `Ok(())`).
2. The receiver is dropped (returns `Err(SendError(value))` with the
   unsent value).

The blocking path goes through spin → yield → park. See
[backoff.md](backoff.md).

### `try_send`

```rust
use nexus_channel::{channel, TrySendError};

let (tx, _rx) = channel::<u64>(4);

match tx.try_send(42u64) {
    Ok(()) => {}
    Err(TrySendError::Full(value)) => {
        // Queue full — value returned to caller
    }
    Err(TrySendError::Disconnected(value)) => {
        // Receiver dropped
    }
}
```

Never blocks. Either the queue had space (`Ok`) or it didn't (`Full`), or
the receiver is gone (`Disconnected`). The unsent value is returned in
both error variants so the caller can retry, drop, or requeue.

## `Receiver<T>`

### `recv`

```rust
# use nexus_channel::{channel, RecvError};
# let (_tx, rx) = channel::<u64>(4);
match rx.recv() {
    Ok(value) => { /* handle value */ }
    Err(RecvError) => { /* sender dropped, no more values coming */ }
}
```

Blocks until a value is available or the sender is dropped. If the sender
is dropped but there are still values in the queue, `recv` drains them
first and only returns `Err(RecvError)` once empty.

### `try_recv`

```rust
use nexus_channel::{channel, TryRecvError};

let (_tx, rx) = channel::<u64>(4);

match rx.try_recv() {
    Ok(value) => { /* got one */ }
    Err(TryRecvError::Empty) => { /* nothing right now */ }
    Err(TryRecvError::Disconnected) => { /* sender gone, queue drained */ }
}
```

Never blocks. Good for integrating with your own poll loop.

### `recv_timeout`

```rust
use std::time::Duration;
use nexus_channel::{channel, RecvTimeoutError};

let (_tx, rx) = channel::<u64>(4);

match rx.recv_timeout(Duration::from_millis(100)) {
    Ok(value) => { /* got one within 100ms */ }
    Err(RecvTimeoutError::Timeout) => { /* deadline elapsed */ }
    Err(RecvTimeoutError::Disconnected) => { /* sender dropped */ }
}
```

Blocks up to the given duration. Internally: fast path → spin →
`thread::park_timeout` with the remaining time.

## Error types summary

| Error | Meaning |
|-------|--------|
| `SendError<T>` | Receiver dropped. Carries the unsent value. |
| `TrySendError::Full(T)` | Queue full right now. |
| `TrySendError::Disconnected(T)` | Receiver dropped. |
| `RecvError` | Sender dropped, queue empty. |
| `TryRecvError::Empty` | No value right now. |
| `TryRecvError::Disconnected` | Sender dropped, queue empty. |
| `RecvTimeoutError::Timeout` | Deadline elapsed before any value arrived. |
| `RecvTimeoutError::Disconnected` | Sender dropped. |

## Thread safety

`Sender<T>` and `Receiver<T>` are `Send` but not `Sync`. Each side must
live on exactly one thread. Trying to share the sender across threads is
a type error — use `nexus_queue::mpsc` or `crossbeam_channel` if you need
multiple producers.
