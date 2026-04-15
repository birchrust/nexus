# Cancellation

Two cancellation primitives:

- **`CancellationToken`** — hierarchical, user-triggered, multi-waiter.
  The building block for graceful shutdown of specific subsystems.
- **`ShutdownSignal`** — single global signal tied to SIGTERM/SIGINT.
  The top-level "time to stop" event.

Use `ShutdownSignal` at the root of your application. Use
`CancellationToken` for everything below it.

## `CancellationToken`

```rust
impl CancellationToken {
    pub fn new() -> Self;
    pub fn child(&self) -> Self;
    pub fn cancel(&self);
    pub fn is_cancelled(&self) -> bool;
    pub fn cancelled(&self) -> Cancelled;  // await-able
    pub fn drop_guard(self) -> DropGuard;
}
```

`.clone()` produces another reference to the **same** token. `.child()`
produces a token that is cancelled when its parent is cancelled (but the
parent is **not** cancelled when the child is). This is hierarchical
cancellation: cancel the root to cancel everything; cancel a child to
cancel just that subtree.

### Basic Pattern: Cancel a Task From Outside

```rust
use nexus_async_rt::{CancellationToken, Runtime, spawn_boxed, sleep};
use nexus_rt::WorldBuilder;
use std::time::Duration;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let token = CancellationToken::new();

        let t = token.clone();
        let worker = spawn_boxed(async move {
            loop {
                if t.is_cancelled() {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
            "clean exit"
        });

        sleep(Duration::from_millis(50)).await;
        token.cancel();
        let result = worker.await;
        assert_eq!(result, "clean exit");
    });
}
```

### `cancelled().await` — Race Against Cancellation

Prefer `cancelled()` over `is_cancelled()` polling when the task is
already waiting on something else. Combine both in a select-style loop.

Since nexus-async-rt has no built-in `select!`, the common pattern is to
split concerns across two spawned tasks joined by a channel, or to poll
both via `Future::poll` manually.

```rust
use nexus_async_rt::{CancellationToken, Runtime, sleep, spawn_boxed};
use nexus_rt::WorldBuilder;
use std::time::Duration;

async fn session_loop(token: CancellationToken) {
    loop {
        if token.is_cancelled() {
            break;
        }
        // Do one unit of work.
        sleep(Duration::from_millis(100)).await;
    }
}

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let root = CancellationToken::new();
        let _h = spawn_boxed(session_loop(root.child()));
        sleep(Duration::from_secs(1)).await;
        root.cancel(); // cancels the child too
    });
}
```

### Hierarchical: Parent + Child Tokens

```rust
use nexus_async_rt::{CancellationToken, Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let root = CancellationToken::new();

        // Each session gets a child — we can cancel one without touching
        // the others, or cancel root to kill everything.
        for i in 0..4 {
            let child = root.child();
            spawn_boxed(async move {
                child.cancelled().await;
                println!("session {i} shutting down");
            });
        }

        // Cancel everything at once.
        root.cancel();
    });
}
```

### `DropGuard` — RAII Cancellation

```rust
impl CancellationToken {
    pub fn drop_guard(self) -> DropGuard;
}

impl DropGuard {
    pub fn disarm(self) -> CancellationToken;
}
```

A `DropGuard` cancels its token on drop unless disarmed. Use it when a
task's lifetime should bound the work of its children.

```rust
use nexus_async_rt::{CancellationToken, Runtime, spawn_boxed, sleep};
use nexus_rt::WorldBuilder;
use std::time::Duration;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let supervisor = async {
            let token = CancellationToken::new();
            let _guard = token.clone().drop_guard();

            // Spawn children tied to the token.
            for _ in 0..3 {
                let t = token.clone();
                spawn_boxed(async move {
                    t.cancelled().await;
                    // cleanup
                });
            }

            // Supervisor does its work...
            sleep(Duration::from_millis(100)).await;

            // Returning normally drops the guard, which cancels the token,
            // which wakes all the child tasks.
        };

        supervisor.await;
    });
}
```

## `ShutdownSignal`

Top-level shutdown driven by SIGTERM/SIGINT. Installed automatically when
`RuntimeBuilder::signal_handlers(true)` (the default).

```rust
pub fn shutdown_signal() -> ShutdownSignal; // from module root
```

`ShutdownSignal` is single-waiter — exactly one task can `.await` it. For
broadcast shutdown, convert the signal into a `CancellationToken::cancel()`
call and distribute the token.

```rust
use nexus_async_rt::{CancellationToken, Runtime, shutdown_signal, spawn_boxed};
use nexus_rt::WorldBuilder;

fn main() {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::builder(&mut world)
        .signal_handlers(true)
        .build();

    rt.block_on(async {
        let root = CancellationToken::new();

        // Distribute children.
        for _ in 0..4 {
            let child = root.child();
            spawn_boxed(async move {
                child.cancelled().await;
                // drain + close
            });
        }

        // Single-waiter: wait for SIGTERM, then fan out.
        shutdown_signal().await;
        root.cancel();

        // Give children a grace period — in real code, join them on a
        // timeout here.
    });
}
```

## When to Use Which

| Problem                                  | Use                              |
|------------------------------------------|----------------------------------|
| Stop one spawned task                    | `JoinHandle::abort()`            |
| Stop a group of related tasks            | `CancellationToken` + `.child()` |
| Propagate "server is shutting down"      | `ShutdownSignal` → Token         |
| Tie child lifetimes to a scope           | `DropGuard`                      |
| Cancellation that survives clone         | `CancellationToken`              |
| Cancellation that kills all descendants  | Parent `CancellationToken`       |

`JoinHandle::abort()` is the most forceful — it drops the future on the
next poll, no chance to clean up. `CancellationToken::cancel()` is
cooperative — the task observes cancellation and runs its own shutdown
logic (flush buffers, close sockets, emit a final log). For anything
touching IO or external state, prefer the token.

## Example: Per-Session Tokens With Parent Cancellation

A gateway spawns one task per accepted connection. Each session has its
own child token so the supervisor can cancel individual sessions (e.g. on
auth failure) without affecting the rest. A global parent token is tied
to `ShutdownSignal` for server shutdown.

```rust
use nexus_async_rt::{
    CancellationToken, Runtime, TcpListener, TcpStream,
    shutdown_signal, spawn_boxed,
};
use nexus_rt::WorldBuilder;

async fn session(stream: TcpStream, token: CancellationToken) {
    loop {
        if token.is_cancelled() {
            break;
        }
        // read + dispatch...
        let _ = &stream;
        break;
    }
}

fn main() -> std::io::Result<()> {
    let mut world = WorldBuilder::new().build();
    let mut rt = Runtime::builder(&mut world)
        .signal_handlers(true)
        .build();

    rt.block_on(async {
        let listener = TcpListener::bind("0.0.0.0:8080".parse().unwrap())?;
        let root = CancellationToken::new();

        let accept_token = root.clone();
        let _accept = spawn_boxed(async move {
            loop {
                if accept_token.is_cancelled() {
                    break;
                }
                let (stream, _) = listener.accept().await?;
                let child = accept_token.child();
                spawn_boxed(session(stream, child));
            }
            #[allow(unreachable_code)]
            Ok::<_, std::io::Error>(())
        });

        shutdown_signal().await;
        root.cancel();
        // optionally wait for accept + sessions to drain
        Ok(())
    })
}
```

## See Also

- [Task Spawning](task-spawning.md) — `JoinHandle::abort`
- [Timers and Time](timers-and-time.md) — combining cancellation with
  deadlines
- [Patterns](patterns.md) — graceful shutdown cookbook
