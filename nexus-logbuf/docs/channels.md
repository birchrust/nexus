# Channels

`nexus_logbuf::channel::spsc` and `channel::mpsc` wrap the raw
queue primitives with:

- **Producer-side backoff**: brief spin + yield on `Full`, no
  syscall.
- **Consumer-side parking**: `park_timeout` when empty, always
  with a timeout so disconnection is observed.
- **Disconnect detection**: both sides can report when the
  counterpart is gone.

Use channels when the consumer is a background thread that
should sleep when idle. Use the raw `queue` API when you're
polling from a hot loop on every iteration.

## SPSC channel

```rust
use nexus_logbuf::channel::spsc;
use std::time::Duration;

let (mut sender, mut receiver) = spsc::new(64 * 1024);

// Sender side (hot path).
match sender.send(11) {
    Ok(mut claim) => {
        claim.copy_from_slice(b"hello world");
        claim.commit();
        sender.notify(); // wake parked receiver if any
    }
    Err(e) => {
        // Disconnected or zero length.
        let _ = e;
    }
}

// Receiver side (background).
match receiver.recv(Some(Duration::from_millis(100))) {
    Ok(rec) => {
        // rec: ReadClaim, derefs to &[u8]
        let _ = &*rec;
    }
    Err(e) => {
        // Timeout or disconnected.
        let _ = e;
    }
}
```

### Sender semantics

- `send(len)` — claim `len` bytes, spin/yield on full, never
  syscall. Returns `WriteClaim` on success, or an error if the
  receiver has dropped.
- `try_send(len)` — single attempt, no spin. `Err(Full)` on
  transient full.
- `notify()` — wake a parked receiver. Call this after
  committing if you expect the receiver might be sleeping.

The sender never parks. This is the **"senders are never slowed
down"** principle: if the buffer is full, you'd rather back-pressure
the caller (return an error) than let it sleep.

### Receiver semantics

- `recv(Some(timeout))` — wait up to `timeout` for a record. On
  empty, parks (with timeout). Returns when data arrives, on
  timeout, or on disconnect.
- `recv(None)` — wait indefinitely.
- `try_recv()` — single attempt, `None` if empty.

The receiver uses `park_timeout` rather than raw condvar so it
can observe disconnect even if nobody calls `notify()`. The
timeout is usually small (1-10 ms) for responsive shutdown.

## MPSC channel

```rust
use nexus_logbuf::channel::mpsc;
use std::thread;

let (sender, mut receiver) = mpsc::new(64 * 1024);

let workers: Vec<_> = (0..4).map(|i| {
    let mut s = sender.clone();
    thread::spawn(move || {
        let msg = format!("worker {}", i);
        if let Ok(mut claim) = s.send(msg.len()) {
            claim.copy_from_slice(msg.as_bytes());
            claim.commit();
            s.notify();
        }
    })
}).collect();
drop(sender);

while let Ok(rec) = receiver.recv(Some(std::time::Duration::from_millis(100))) {
    // process rec
    let _ = &*rec;
    # break;
}

for w in workers { w.join().unwrap(); }
```

`channel::mpsc::Sender` is `Clone + Send`. Drop all clones to
let the receiver observe disconnect.

## Error types

- `SendError::Disconnected` — receiver is gone.
- `SendError::ZeroLength` — you passed `len == 0`.
- `TrySendError::Full` — transient, try again.
- `TrySendError::Disconnected` — permanent.
- `TrySendError::ZeroLength` — permanent user error.
- `RecvError::Timeout` — nothing arrived in time.
- `RecvError::Disconnected` — all senders are gone.

## When to use channel vs raw queue

| Scenario | Use |
|---|---|
| Consumer polls every loop iteration | raw `queue::spsc` |
| Consumer is a dedicated background thread that can sleep | `channel::spsc` |
| Many producers, one background consumer | `channel::mpsc` |
| You need explicit abort without producing a skip | raw `queue` + custom logic |
| You want backpressure signalling to the producer | raw `queue` returns `Full`, you react |

Channel adds ~20-50 cycles per op on the hot path due to the
notify/park bookkeeping — usually worth it for the simpler
control flow.
