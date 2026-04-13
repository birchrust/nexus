# API Guide

How to use nexus-async-rt correctly. This covers the public API — what your
code looks like when you use this runtime.

## Quick Start

```rust
use nexus_async_rt::{Runtime, spawn_boxed};
use nexus_rt::WorldBuilder;

fn main() {
    let wb = WorldBuilder::new();
    let mut world = wb.build();
    let mut rt = Runtime::builder(&mut world).build();

    rt.block_on(async {
        let handle = spawn_boxed(async { 42 });
        let result = handle.await;
        assert_eq!(result, 42);
    });
}
```

## Building the Runtime

`RuntimeBuilder` configures the runtime before construction. All settings
have sensible defaults — you only need to set what you want to tune.

```rust
let mut rt = Runtime::builder(&mut world)
    .tasks_per_cycle(128)           // poll up to 128 tasks before checking IO
    .event_interval(31)             // check IO every 31 ticks (default: 61)
    .signal_handlers(true)          // install SIGTERM/SIGINT handlers
    .build();
```

### When to tune `tasks_per_cycle`

Higher values = more tasks processed per loop iteration = higher throughput.
Lower values = IO checked more frequently = lower IO latency. Default (64)
is a reasonable balance. For IO-heavy workloads (many sockets), lower it.
For compute-heavy workloads (many CPU tasks), raise it.

### When to tune `event_interval`

Controls how often `epoll(timeout=0)` is called between task batches. Lower
= more IO checks = lower IO tail latency but more syscalls. Default (61,
a prime to avoid resonance) is good for most workloads.

## Running the Event Loop

Two entry points. Choose based on your latency/CPU tradeoff:

```rust
// Parks thread when idle. CPU-friendly. Good for mixed workloads.
rt.block_on(async { ... });

// Spins when idle. Minimum wake latency. Burns a core.
rt.block_on_busy(async { ... });
```

Both return the root future's output. The runtime drives IO, timers, and
spawned tasks until the root future completes.

## Spawning Tasks

### Box-allocated (default)

```rust
use nexus_async_rt::spawn_boxed;

let handle = spawn_boxed(async {
    // your async work
    42
});
let result = handle.await; // JoinHandle<i32>
```

### Fire-and-forget

```rust
// Drop the handle — task runs to completion but output is discarded.
drop(spawn_boxed(async { do_work().await }));
```

### Slab-allocated (zero-alloc hot path)

Configure a slab at build time:

```rust
use nexus_slab::byte::unbounded::Slab;

let slab = unsafe { Slab::<256>::with_chunk_capacity(1024) };
let mut rt = Runtime::builder(&mut world)
    .slab_unbounded(slab)
    .build();
```

Then spawn into the slab:

```rust
use nexus_async_rt::spawn_slab;

let handle = spawn_slab(async { fast_path_work().await });
```

The `256` is the slot size in bytes. Must be >= 64 (task header size).
The future + output must fit in the remaining bytes.

### Two-phase slab allocation

For maximum control — reserve the slot first, spawn later:

```rust
use nexus_async_rt::{claim_slab, try_claim_slab};

// Guaranteed slot (panics if full):
let claim = claim_slab();
let handle = claim.spawn(async { ... });

// Non-blocking (returns None if full):
if let Some(claim) = try_claim_slab() {
    let handle = claim.spawn(async { ... });
} else {
    // fallback: spawn_boxed
}
```

Dropping a `SlabClaim` without calling `.spawn()` returns the slot to the
freelist. No leak.

## Timers

```rust
use nexus_async_rt::{sleep, timeout, interval};
use std::time::Duration;

// Sleep
sleep(Duration::from_millis(100)).await;

// Timeout a future
match timeout(some_future, Duration::from_secs(5)).await {
    Ok(result) => { /* completed in time */ }
    Err(_elapsed) => { /* timed out */ }
}

// Periodic interval
let mut ticker = interval(Duration::from_millis(10));
loop {
    ticker.next().await;
    do_periodic_work();
}
```

## IO (TCP/UDP)

IO uses mio under the hood. Register sockets, await readiness.

```rust
use nexus_async_rt::net::{TcpListener, TcpStream};

let listener = TcpListener::bind("0.0.0.0:8080".parse()?)?;
let (stream, addr) = listener.accept().await?;

let mut buf = [0u8; 1024];
let n = stream.read(&mut buf).await?;
stream.write_all(&buf[..n]).await?;
```

## World Access

Access nexus-rt ECS resources from async tasks:

```rust
use nexus_async_rt::with_world;

// Synchronous, inline during task poll — no await point.
with_world(|world| {
    let config = world.resource::<Config>();
    println!("setting: {}", config.value);
});
```

### Pre-resolved handlers (hot path)

For event dispatch where HashMap lookup cost matters:

```rust
// At setup (cold path) — resolve resource IDs once:
let mut on_quote = (|mut books: ResMut<Books>, q: Quote| {
    books.update(q);
}).into_handler(world.registry());

// Per-event (hot path) — single deref per resource:
with_world(|world| on_quote.run(world, quote));
```

## Channels

In-process async channels for task-to-task communication:

```rust
use nexus_async_rt::channel::local;

// Bounded, single-producer single-consumer
let (tx, rx) = local::channel::<Quote>(64);

spawn_boxed(async move {
    tx.send(quote).await.unwrap();
});

spawn_boxed(async move {
    let quote = rx.recv().await.unwrap();
});
```

Also available: `spsc`, `mpsc` (typed), `spsc_bytes`, `mpsc_bytes` (byte buffers).

## Cancellation

Cooperative cancellation with hierarchical propagation:

```rust
use nexus_async_rt::CancellationToken;

let token = CancellationToken::new();
let child = token.child(); // cancelled when parent is

spawn_boxed({
    let token = token.clone();
    async move {
        token.cancelled().await;
        println!("shutting down");
    }
});

token.cancel(); // cancels token + all children
```

## Shutdown

For signal-based shutdown (SIGTERM/SIGINT):

```rust
let mut rt = Runtime::builder(&mut world)
    .signal_handlers(true)
    .build();

rt.block_on(async {
    // Completes when SIGTERM or SIGINT received
    nexus_async_rt::shutdown_signal().await;
    cleanup().await;
});
```

Single-waiter design. For multi-waiter shutdown, use `CancellationToken`.

## Tokio Bridge

For cold-path work that needs the tokio ecosystem (reqwest, database
drivers, etc.):

```rust
use nexus_async_rt::tokio_compat::{with_tokio, spawn_on_tokio};

// Run tokio future on our executor (tokio provides reactor):
let stream = with_tokio(|| {
    tokio::net::TcpStream::connect("127.0.0.1:6379")
}).await?;

// Run on tokio's thread pool, get result back:
let response = spawn_on_tokio(async {
    reqwest::get("https://api.example.com/data").await
}).await?;
```

Requires the `tokio-compat` feature.

## Common Patterns

### Gateway event loop

```rust
rt.block_on(async {
    let listener = TcpListener::bind(addr)?;
    let token = CancellationToken::new();

    loop {
        tokio::select! {
            conn = listener.accept() => {
                let (stream, _) = conn?;
                let token = token.child();
                spawn_boxed(handle_connection(stream, token));
            }
            _ = shutdown_signal() => {
                token.cancel();
                break;
            }
        }
    }
});
```

### Market data processing

```rust
let mut on_quote = quote_handler.into_handler(world.registry());
let mut on_trade = trade_handler.into_handler(world.registry());

rt.block_on(async {
    loop {
        let msg = ws.recv().await?;
        match msg.msg_type() {
            MsgType::Quote => with_world(|w| on_quote.run(w, parse_quote(&msg))),
            MsgType::Trade => with_world(|w| on_trade.run(w, parse_trade(&msg))),
        }
    }
});
```
