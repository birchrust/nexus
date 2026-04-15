# Channels

nexus-async-rt ships five channel variants, all under `nexus_async_rt::channel`:

| Module           | Shape | Payload     | Send          | Recv            |
|------------------|-------|-------------|---------------|-----------------|
| `local`          | SPSC  | `T`         | `&Sender<T>`  | `&Receiver<T>`  |
| `spsc`           | SPSC  | `T: Send`   | `&Sender<T>`  | `&Receiver<T>`  |
| `mpsc`           | MPSC  | `T: Send`   | `&Sender<T>`  | `&Receiver<T>`  |
| `spsc_bytes`     | SPSC  | `&[u8]`     | `WriteClaim`  | `ReadClaim`     |
| `mpsc_bytes`     | MPSC  | `&[u8]`     | `WriteClaim`  | `ReadClaim`     |

**Pick the narrowest constraint that matches your topology.** `local` is
the fastest (no atomics — single-threaded) but can only be used within
one thread. `spsc` / `mpsc` are cross-thread (for the tokio bridge and
background workers). The `_bytes` variants carry variable-length byte
messages without per-message allocation — use them for wire protocols,
archival, and structured binary logs.

## `local::channel` — Single-Threaded

```rust
use nexus_async_rt::{channel::local, Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let (tx, rx) = local::channel::<u64>(64);

        let producer = spawn_boxed(async move {
            for i in 0..10 {
                tx.send(i).await.unwrap();
            }
            // tx dropped here — receiver will see RecvError
        });

        let consumer = spawn_boxed(async move {
            while let Ok(v) = rx.recv().await {
                println!("got {v}");
            }
        });

        producer.await;
        consumer.await;
    });
}
```

Cheapest option. Used for pipeline stages on a single executor.

## `spsc::channel` / `mpsc::channel` — Cross-Thread

Same shape, different atomicity. Use these when a background thread
(e.g. a tokio-owned thread pool) needs to hand work to an async task.

```rust
use nexus_async_rt::{channel::mpsc, Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let (tx, rx) = mpsc::channel::<String>(128);

        // Multiple producers.
        for i in 0..4 {
            let tx = tx.clone();
            std::thread::spawn(move || {
                // NOTE: send is async — in a thread, use blocking try_send
                // or run a tokio runtime. For real code, usually the
                // producer lives in spawn_on_tokio and uses send().await.
                let _ = tx.try_send(format!("worker {i}"));
            });
        }
        drop(tx);

        spawn_boxed(async move {
            while let Ok(msg) = rx.recv().await {
                println!("{msg}");
            }
        }).await;
    });
}
```

### `try_send` / `try_recv`

Non-blocking variants. `try_send` returns `Result<(), TrySendError<T>>`
and gives back the value on full-or-closed. `try_recv` returns
`Result<T, TryRecvError>` where empty and closed are distinguishable.

```rust
use nexus_async_rt::channel::{spsc, TrySendError, TryRecvError};

async fn drain_available(rx: &spsc::Receiver<u32>) -> Vec<u32> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(v) => out.push(v),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        }
    }
    out
}
```

## Sender Drop Semantics

When all senders drop, pending `recv().await` calls wake and return
`RecvError` (`Disconnected` for `try_recv`). Consumers that loop on
`while let Ok(...)` exit cleanly — that's the idiomatic shutdown path.

Conversely, if the receiver drops, pending `send().await` calls resolve
to `SendError<T>` with the value returned so the caller can recover or
forward it.

## Byte Channels

`spsc_bytes` / `mpsc_bytes` carry variable-length byte payloads without
heap allocation per message. They're backed by `nexus-logbuf` and use a
**claim-based API**: reserve `N` bytes, write into the slice, commit.

```rust
use nexus_async_rt::channel::{spsc_bytes, ClaimError};
use nexus_async_rt::{Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let (mut tx, mut rx) = spsc_bytes::channel(64 * 1024);

        let producer = spawn_boxed(async move {
            for i in 0u32..100 {
                let mut claim = tx.claim(4).await;       // WriteClaim<'_>
                claim[..].copy_from_slice(&i.to_le_bytes());
                claim.commit();
            }
        });

        let consumer = spawn_boxed(async move {
            for _ in 0..100 {
                let claim = rx.recv().await.unwrap();    // ReadClaim<'_>
                let n = u32::from_le_bytes(claim[..4].try_into().unwrap());
                println!("got {n}");
                drop(claim);
            }
        });

        producer.await;
        consumer.await;
    });
}
```

### `ClaimError::ZeroLength`

Claiming zero bytes is an error (it would produce an empty message that's
indistinguishable from no message). Validate your `len > 0` precondition
before claiming. For real wire protocols this falls out naturally — you
always know the frame size before claiming.

### When to Use Byte Channels

- **WebSocket frame hand-off:** decoder produces a `ReadClaim`, consumer
  borrows `&[u8]` directly — zero-copy.
- **FIX message archival:** each order gets a claim-sized slot in a ring
  buffer, flushed to disk by a background task.
- **Log multiplexer:** multiple producers fan into one archival writer.

Use typed channels (`spsc`/`mpsc`) when you own the struct; use byte
channels when you're moving wire-format bytes and want to avoid
serialization round-trips.

## RecvFut Drop Safety

Dropping a `RecvFut` that has been polled (and therefore registered a
waker) safely unregisters the waker slot before returning. You can mix
channels with timeouts, select-style combinators, and cancellation
without leaking waker slots. This is covered in detail in
[ARCHITECTURE.md](ARCHITECTURE.md).

## Example: Producer/Consumer With Backpressure

Channel capacity IS the backpressure mechanism. Producer blocks on
`send().await` when the channel is full; there is no need for an
explicit semaphore.

```rust
use nexus_async_rt::{channel::local, Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;

async fn fetch_one(i: u64) -> Vec<u8> { vec![0u8; i as usize] }
async fn persist(_data: Vec<u8>) {}

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        // Bounded to 16: producer never gets more than 16 jobs ahead.
        let (tx, rx) = local::channel::<Vec<u8>>(16);

        let producer = spawn_boxed(async move {
            for i in 0..1000 {
                let data = fetch_one(i).await;
                tx.send(data).await.unwrap(); // backpressure here
            }
        });

        let consumer = spawn_boxed(async move {
            while let Ok(data) = rx.recv().await {
                persist(data).await;
            }
        });

        producer.await;
        consumer.await;
    });
}
```

## Example: Fan-In With `mpsc`

Multiple async tasks feeding into one serialized consumer — canonical
"split IO from processing" pattern.

```rust
use nexus_async_rt::{channel::mpsc, Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;

struct Tick { source: u32, price: f64 }

async fn subscribe(source: u32) -> Tick { Tick { source, price: 0.0 } }
async fn apply_tick(_t: Tick) {}

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let (tx, rx) = mpsc::channel::<Tick>(256);

        for source in 0..4 {
            let tx = tx.clone();
            spawn_boxed(async move {
                loop {
                    let tick = subscribe(source).await;
                    if tx.send(tick).await.is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx); // so rx completes when all producers exit

        spawn_boxed(async move {
            while let Ok(tick) = rx.recv().await {
                apply_tick(tick).await;
            }
        }).await;
    });
}
```

## See Also

- [Architecture](ARCHITECTURE.md) — cross-thread waker design, channel
  internals
- [Patterns](patterns.md) — fan-in, backpressure, supervisor-worker
- [Tokio Compatibility](tokio-compat.md) — bridging background threads
