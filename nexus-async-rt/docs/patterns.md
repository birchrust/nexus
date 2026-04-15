# Patterns

Cookbook of common shapes. Each pattern: the context, a compilable
sketch, and the gotchas worth remembering.

## 1. Event Loop With Shutdown Signal

**Context:** A long-running service that does some work and stops cleanly
on SIGTERM.

```rust
use nexus_async_rt::{Runtime, shutdown_signal, spawn_boxed, sleep};
use nexus_rt::WorldBuilder;
use std::time::Duration;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::builder(&mut world)
        .signal_handlers(true)
        .build();

    rt.block_on(async {
        let worker = spawn_boxed(async {
            loop {
                do_work().await;
                sleep(Duration::from_millis(100)).await;
            }
        });

        shutdown_signal().await;
        let was_running = worker.abort();
        eprintln!("shutdown (worker_running={was_running})");
    });
}

async fn do_work() {}
```

**Gotchas:** `shutdown_signal()` is single-waiter. If you have multiple
subsystems, fan out via `CancellationToken::cancel()`.

## 2. Per-Connection Task With Cancellation

**Context:** Accept connections and run each in its own task, with a
parent token tied to SIGTERM.

```rust
use nexus_async_rt::{
    CancellationToken, Runtime, TcpListener, TcpStream,
    shutdown_signal, spawn_boxed,
};
use nexus_rt::WorldBuilder;

async fn session(mut stream: TcpStream, token: CancellationToken) {
    loop {
        if token.is_cancelled() { break; }
        let mut buf = [0u8; 1024];
        match nexus_async_rt::AsyncRead::read(&mut stream, &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => { let _ = &buf[..n]; }
        }
    }
}

fn main() -> std::io::Result<()> {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::builder(&mut world).signal_handlers(true).build();
    rt.block_on(async {
        let listener = TcpListener::bind("0.0.0.0:9000".parse().unwrap())?;
        let root = CancellationToken::new();

        let accept_token = root.clone();
        spawn_boxed(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(x) => x,
                    Err(_) => break,
                };
                spawn_boxed(session(stream, accept_token.child()));
            }
        });

        shutdown_signal().await;
        root.cancel();
        Ok(())
    })
}
```

**Gotchas:** `child()` tokens die with the parent but not vice versa.
Don't call `cancel()` on the child if you mean to stop a whole subsystem.

## 3. Heartbeat + Read Loop (Split Tasks)

**Context:** A session that must send heartbeats AND read frames
concurrently. Split into two spawned tasks + a channel.

```rust
use nexus_async_rt::{
    Runtime, channel::local, interval, spawn_boxed, timeout,
    MissedTickBehavior,
};
use nexus_rt::WorldBuilder;
use std::time::Duration;

async fn write_frame(_bytes: &[u8]) -> std::io::Result<()> { Ok(()) }
async fn read_frame() -> std::io::Result<Vec<u8>> { Ok(Vec::new()) }

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let (frame_tx, frame_rx) = local::channel::<Vec<u8>>(64);

        // Reader task — drives recv with a read timeout for liveness.
        let reader = spawn_boxed(async move {
            loop {
                match timeout(Duration::from_secs(30), read_frame()).await {
                    Ok(Ok(f)) => if frame_tx.send(f).await.is_err() { break; },
                    _ => break,
                }
            }
        });

        // Heartbeat task — periodic, independent cadence.
        let hb = spawn_boxed(async {
            let mut i = interval(Duration::from_secs(15));
            i.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                i.tick().await;
                if write_frame(b"PING").await.is_err() { break; }
            }
        });

        // Consumer — processes frames.
        let consumer = spawn_boxed(async move {
            while let Ok(_frame) = frame_rx.recv().await {
                // dispatch via with_world(|w| handler.run(w, _frame));
            }
        });

        let _ = reader.await;
        let _ = hb.abort();
        let _ = consumer.await;
    });
}
```

**Gotchas:** Don't try to do both in one task with a manual
select-poll — use two tasks and a channel. Clearer code, same
performance, and cancellation is trivial.

## 4. Producer/Consumer With Backpressure

**Context:** Bounded channel IS the backpressure. Don't add a semaphore.

```rust
use nexus_async_rt::{Runtime, channel::local, spawn_boxed};
use nexus_rt::WorldBuilder;

async fn fetch(i: u64) -> Vec<u8> { vec![0; i as usize % 1024] }
async fn persist(_: Vec<u8>) {}

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let (tx, rx) = local::channel::<Vec<u8>>(32);

        let p = spawn_boxed(async move {
            for i in 0..10_000 {
                let payload = fetch(i).await;
                // Blocks when rx is 32 ahead — that's the backpressure.
                if tx.send(payload).await.is_err() { break; }
            }
        });

        let c = spawn_boxed(async move {
            while let Ok(v) = rx.recv().await { persist(v).await; }
        });

        p.await;
        c.await;
    });
}
```

**Gotchas:** Cap the channel to a number that bounds memory usage under
worst-case backlog. "Unbounded" is a design smell — you're deferring the
decision about what happens when the consumer stalls.

## 5. Reconnect With Exponential Backoff

**Context:** Exchange session that auto-reconnects. Reset backoff on
successful connect; cap the max delay.

```rust
use nexus_async_rt::{Runtime, sleep, spawn_boxed};
use nexus_rt::WorldBuilder;
use std::time::Duration;

async fn run_session() -> std::io::Result<()> {
    Err(std::io::Error::other("dropped"))
}

async fn reconnect_loop() {
    let mut delay = Duration::from_millis(250);
    let cap = Duration::from_secs(30);
    loop {
        match run_session().await {
            Ok(()) => { delay = Duration::from_millis(250); }
            Err(e) => {
                eprintln!("session error: {e}; retry in {delay:?}");
                sleep(delay).await;
                delay = (delay * 2).min(cap);
            }
        }
    }
}

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async { spawn_boxed(reconnect_loop()).await; });
}
```

**Gotchas:** Reset the delay **only on real success** (session actually
connected, not just "DNS succeeded"). Otherwise you get tight-loop retries
against a half-broken endpoint.

## 6. Tokio Bridge for Cold-Path HTTP

**Context:** Periodic REST refresh that updates World state. Hot path
stays on our executor.

```rust
use nexus_async_rt::{
    Runtime, interval, spawn_boxed, with_world,
    tokio_compat::with_tokio,
};
use nexus_rt::{Resource, WorldBuilder};
use std::time::Duration;

#[derive(Resource, Default)]
struct RefPrice(f64);

fn main() {
    let mut world = WorldBuilder::new()
        .with_resource(RefPrice::default())
        .build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        spawn_boxed(async {
            let mut tick = interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                let body: String = with_tokio(|| async {
                    reqwest::get("https://api.example.com/price")
                        .await.unwrap()
                        .text().await.unwrap()
                }).await;
                let px: f64 = body.parse().unwrap_or(0.0);
                with_world(|w| w.resource_mut::<RefPrice>().0 = px);
            }
        }).await;
    });
}
```

**Gotchas:** Keep the `with_world` call synchronous — don't hold the
borrow across an `.await`. The borrow checker enforces this.

## 7. Pre-Resolved Handler Dispatch

**Context:** Market data loop that dispatches a nexus-rt Handler once
per message. Resolve once, dispatch many.

```rust
use nexus_async_rt::{Runtime, spawn_boxed, with_world};
use nexus_rt::{IntoHandler, Handler, ResMut, Resource, WorldBuilder};

#[derive(Resource, Default)]
struct Book { mid: f64 }

struct Tick { mid: f64 }

fn on_tick(mut book: ResMut<Book>, t: Tick) {
    book.mid = t.mid;
}

async fn recv_tick() -> Tick { Tick { mid: 100.0 } }

fn main() {
    let mut world = WorldBuilder::new()
        .with_resource(Book::default())
        .build();
    let mut handler = on_tick.into_handler(world.registry());

    let mut rt = Runtime::new(&mut world);
    rt.block_on(async move {
        spawn_boxed(async move {
            for _ in 0..100 {
                let t = recv_tick().await;
                with_world(|w| handler.run(w, t));
            }
        }).await;
    });
}
```

**Gotchas:** Handlers are `!Send`, so the task stays on the executor
thread — that's always true for nexus-async-rt anyway. Move `handler`
into the async block once, don't re-resolve per message.

## 8. Spawning and Joining Many Tasks

**Context:** Launch N parallel async operations and wait for all.
nexus-async-rt has no `join_all` helper — use a vec of handles.

```rust
use nexus_async_rt::{Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;

async fn query(i: u32) -> u32 { i * 2 }

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let handles: Vec<_> = (0..8)
            .map(|i| spawn_boxed(query(i)))
            .collect();

        let mut total = 0u32;
        for h in handles {
            total += h.await;
        }
        assert_eq!(total, (0..8).map(|i| i * 2).sum());
    });
}
```

**Gotchas:** The handles are awaited sequentially, but the tasks run
concurrently — by the time the first `.await` returns, subsequent tasks
are already done (or close to it). For true fan-out-fan-in with errors,
wrap each task in a `Result` and collect.

## See Also

- [Task Spawning](task-spawning.md) — `spawn_boxed` vs `spawn_slab`
- [Cancellation](cancellation.md) — tokens, DropGuard, shutdown signal
- [Channels](channels.md) — backpressure, fan-in, byte channels
- [Integration with nexus-rt](integration-with-nexus-rt.md) — the World
  from async tasks
- [Tokio Compatibility](tokio-compat.md) — bridging ecosystem crates
