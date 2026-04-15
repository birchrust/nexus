# Task Spawning

nexus-async-rt offers two allocation strategies for spawned tasks: **Box**
(default) and **slab** (zero-alloc hot path). Both produce the same
`JoinHandle<T>` and run on the same executor — the only difference is where
the task's memory lives.

## The Two Strategies

| Strategy   | Allocator       | Setup                          | Use when                        |
|------------|-----------------|--------------------------------|---------------------------------|
| `spawn_boxed` | `Box::new`   | None                           | Default. Long-lived tasks.      |
| `spawn_slab`  | pre-allocated slab | `RuntimeBuilder::slab_*`  | Hot-path task churn, determinism |

You can mix them freely in the same runtime — each task's header knows how
to free itself.

## `spawn_boxed` — The Default

```rust
use nexus_async_rt::{Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);

    rt.block_on(async {
        let handle = spawn_boxed(async {
            // Some work.
            compute_pnl().await
        });
        let pnl = handle.await;
        println!("pnl = {pnl}");
    });
}

async fn compute_pnl() -> f64 { 0.0 }
```

Signature:

```rust
pub fn spawn_boxed<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
```

One `Box::new` per spawn. Use it for anything that isn't on the latency
hot path — connection setup, REST calls, periodic jobs, shutdown tasks.

## `spawn_slab` — Zero-Alloc Hot Path

Requires a slab configured at runtime build time. The slot size `S` must be
large enough for the task header (64 bytes) plus `max(size_of::<F>(),
size_of::<Output>())`.

```rust
use nexus_async_rt::{Runtime, spawn_slab, ByteSlab, MIN_SLOT_SIZE};
use nexus_rt::WorldBuilder;

fn main() {
    // SAFETY: single-threaded runtime.
    let slab = unsafe { ByteSlab::<256>::with_chunk_capacity(1024) };

    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::builder(&mut world)
        .slab_unbounded(slab)
        .build();

    rt.block_on(async {
        // Zero allocation on this path — slot comes from the slab.
        let handle = spawn_slab(async { 42u32 });
        assert_eq!(handle.await, 42);
    });
}
```

Bounded variant (`slab_bounded`) fails fast when the slab is full instead
of growing. Use it in systems that must reject work above a known capacity
ceiling (trading gateways, exchange sessions).

### Minimum Slot Size

```rust
pub const MIN_SLOT_SIZE: usize = 128;
```

128 bytes covers small futures. Larger futures (state machines with buffered
reads, nested awaits, big enums) need proportionally larger slots. A too-small
`S` panics at spawn time with a message naming the required size — you only
find this once per future type, then you know your number.

## Two-Phase Claim: `claim_slab` / `try_claim_slab`

Sometimes you want to reserve slab space **before** you know what future to
spawn — for example, when accepting a connection you want to fail fast if
the task slab is full, then do the expensive handshake work, then spawn.

```rust
use nexus_async_rt::{claim_slab, try_claim_slab, SlabClaim};

async fn accept_loop(listener: nexus_async_rt::TcpListener) {
    loop {
        let (stream, _) = listener.accept().await.unwrap();

        // Reserve a slot FIRST. If the slab is full, drop the
        // connection before we spend cycles on it.
        let claim = match try_claim_slab() {
            Some(c) => c,
            None => {
                drop(stream); // reject — slab exhausted
                continue;
            }
        };

        // Now it's safe to build the session future.
        let _handle = claim.spawn(session_task(stream));
    }
}

async fn session_task(_stream: nexus_async_rt::TcpStream) { /* ... */ }
```

`claim_slab()` blocks (panics if called outside a runtime); `try_claim_slab()`
returns `Option<SlabClaim>`. A dropped `SlabClaim` releases the reservation.

```rust
impl SlabClaim {
    pub fn spawn<F>(self, future: F) -> JoinHandle<F::Output>
    where F: Future + 'static;
    pub fn slot_size(&self) -> usize;
    pub fn as_ptr(&self) -> *mut u8;
}
```

## `JoinHandle<T>` — Awaiting the Output

```rust
pub struct JoinHandle<T> { /* !Send + !Sync */ }

impl<T: 'static> Future for JoinHandle<T> {
    type Output = T; // the task's output, NOT Result<T, _>
}

impl<T> JoinHandle<T> {
    pub fn is_finished(&self) -> bool;
    pub fn abort(self) -> bool; // consumes handle
}
```

Note: `JoinHandle::Output` is `T`, not `Result<T, JoinError>`. nexus-async-rt
does not catch panics — a panicking task unwinds the whole executor thread,
same as any other Rust code. If you want panic isolation, use
`std::panic::catch_unwind` inside the task body. This is deliberate: trading
systems should crash-and-restart on panic rather than silently continue with
a half-initialized world.

## Detachment

Dropping the `JoinHandle` detaches the task: it keeps running, but the
output is dropped on completion.

```rust
spawn_boxed(async {
    log::info!("fire and forget");
});
// handle dropped here — task still runs
```

`JoinHandle` is `#[must_use]`, so the compiler will warn you. Silence it
with `let _ = spawn_boxed(...)` if detachment is intentional.

## `JoinHandle::abort`

```rust
let handle = spawn_boxed(async_loop());
// later...
let was_running = handle.abort();
```

Abort consumes the handle (you cannot await it after — enforced at the type
level). The future is dropped on the next poll cycle. Any resources the
future owned (TCP streams, channel senders, timer registrations) are dropped
then, cleanly.

**Warning:** Attempting to poll an aborted `JoinHandle` before consuming it
would panic. Since `abort` takes `self`, the type system prevents this.

## Mixing Box and Slab

Common pattern: slab for high-rate per-message tasks, Box for the long-lived
control tasks that own them.

```rust
rt.block_on(async {
    // Long-lived supervisor — Box is fine.
    let _supervisor = spawn_boxed(async move {
        loop {
            let msg = recv_next().await;

            // Per-message work — slab avoids malloc on the hot path.
            let _handle = spawn_slab(process(msg));
        }
    });

    nexus_async_rt::shutdown_signal().await;
});

async fn recv_next() -> u64 { 0 }
async fn process(_msg: u64) {}
```

## Performance Notes

- **Box spawn**: one `malloc`, ~150-300ns on glibc. Fine for < 10K spawns/sec.
- **Slab spawn**: no malloc, ~25-50ns. Matters when you spawn per market tick
  or per order update.
- Slab wins measurably at > 100K spawns/sec or when tail-latency jitter from
  malloc matters (e.g. running on `malloc_trim`-unfriendly workloads).
- If your task count is small and fixed (10 connections, 5 background jobs),
  Box has no observable cost.

## Complete Example: Exchange Gateway

```rust
use nexus_async_rt::{
    ByteSlab, Runtime, TcpListener, TcpStream,
    spawn_boxed, try_claim_slab,
};
use nexus_rt::WorldBuilder;

fn main() -> std::io::Result<()> {
    // Enough slots for max concurrent sessions.
    let slab = unsafe { ByteSlab::<512>::with_chunk_capacity(4096) };
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::builder(&mut world)
        .slab_unbounded(slab)
        .tasks_per_cycle(128)
        .build();

    rt.block_on(async {
        let listener = TcpListener::bind("0.0.0.0:8080".parse().unwrap())?;

        // Supervisor is long-lived — Box.
        let _sup = spawn_boxed(async move {
            loop {
                let (stream, peer) = listener.accept().await?;

                // Reserve BEFORE doing handshake work.
                let Some(claim) = try_claim_slab() else {
                    log::warn!("slab full, rejecting {peer}");
                    drop(stream);
                    continue;
                };

                // Slab-allocated session task.
                let _ = claim.spawn(handle_session(stream));
            }
            #[allow(unreachable_code)]
            Ok::<_, std::io::Error>(())
        });

        nexus_async_rt::shutdown_signal().await;
        Ok(())
    })
}

async fn handle_session(_s: TcpStream) { /* ... */ }
```

## See Also

- [Integration with nexus-rt](integration-with-nexus-rt.md) — accessing the
  World from spawned tasks
- [Cancellation](cancellation.md) — `CancellationToken` for graceful shutdown
- [Patterns](patterns.md) — spawn + channel + timeout cookbook
