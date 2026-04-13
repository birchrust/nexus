# Architecture

Single-threaded async runtime for nexus-rt applications. Designed for
deterministic, low-latency event processing — no work stealing, no thread
pools (except the optional tokio bridge for cold-path IO).

## Execution Model

One thread. One event loop. All state in thread-local storage.

```
Runtime::block_on(root_future)
  │
  ├── Install TLS contexts (World, IO, Timer, Slab, CrossWake)
  │
  └── run_loop:
        ╭─────────────────────────────────────╮
        │  1. Poll root future (if woken)     │
        │  2. Drain cross-thread inbox        │
        │  3. Poll executor (ready tasks)     │
        │  4. Fire expired timers             │
        │  5. Drain cross-thread inbox again  │
        │  6. Non-blocking IO poll (periodic) │
        │  7. Has work? → loop to 1           │
        │  8. No work → park in epoll_wait    │
        ╰─────────────────────────────────────╯
```

**Park vs Spin:** `block_on` parks the thread in `epoll_wait` when idle
(CPU-friendly). `block_on_busy` spins with `epoll(timeout=0)` (minimum
wake latency, burns a core). Choose based on whether this core is dedicated.

**Event interval:** IO is polled non-blocking every `event_interval` ticks
(default: 61). This avoids a syscall on every loop iteration while keeping
IO latency bounded. Tunable via `RuntimeBuilder::event_interval()`.

## Component Map

```
┌─────────────────────────────────────────────────────┐
│                     Runtime                          │
│                                                     │
│  ┌───────────┐  ┌──────────┐  ┌──────────────────┐ │
│  │ Executor  │  │ IoDriver │  │   TimerDriver    │ │
│  │           │  │  (mio)   │  │ (nexus-timer     │ │
│  │ incoming  │  │          │  │  wheel)          │ │
│  │ draining  │  │ tokens → │  │                  │ │
│  │ deferred  │  │  wakers  │  │ deadlines →      │ │
│  │ all_tasks │  │          │  │  wakers          │ │
│  └─────┬─────┘  └────┬─────┘  └────────┬─────────┘ │
│        │             │                  │           │
│  ┌─────┴─────────────┴──────────────────┴─────────┐ │
│  │              TLS Context Layer                  │ │
│  │  CTX_WORLD │ CTX_IO │ CTX_TIMER │ SLAB │ SPAWN │ │
│  └─────────────────────────────────────────────────┘ │
│                                                     │
│  ┌──────────────────┐  ┌────────────────────────┐   │
│  │  CrossWakeContext │  │    ShutdownHandle      │   │
│  │  (Arc, eventfd)   │  │  (AtomicBool + waker)  │   │
│  └──────────────────┘  └────────────────────────┘   │
│                                                     │
│  ┌──────────────────┐  ┌────────────────────────┐   │
│  │    WorldCtx      │  │   Tokio Bridge         │   │
│  │  (Copy handle)   │  │  (optional, lazy init) │   │
│  └──────────────────┘  └────────────────────────┘   │
└─────────────────────────────────────────────────────┘
```

## Task Lifecycle

```
spawn_boxed(future) or spawn_slab(future)
  │
  ├── Allocate task (Box or slab copy_nonoverlapping)
  ├── Write header: poll_fn, drop_fn, free_fn, refcount=1
  ├── Enqueue to incoming
  │
  ▼
Executor::poll()
  │
  ├── Swap incoming ↔ draining
  ├── For each task in draining (up to tasks_per_cycle):
  │     ├── Create waker from task pointer (zero-alloc vtable)
  │     ├── Poll the future
  │     ├── Poll::Pending → task stays alive, waker may fire later
  │     └── Poll::Ready → complete_task()
  │
  ▼
complete_task(ptr)
  │
  ├── Joinable? → store output, wake JoinHandle, dec ref
  ├── Fire-and-forget? → drop output, dec ref, free if last
  └── Aborted? → drop future, notify, dec ref, free if last
```

**Refcount rules:**
- Executor holds 1 ref (released in `complete_task`)
- Each waker clone holds 1 ref (released on waker drop)
- JoinHandle holds 1 ref (released on handle drop or detach)
- Exactly one `ref_dec` returns `should_free=true` — that caller pushes
  to `deferred_free`

**Deferred free:** Task slots are not freed during `poll()` — they're
queued in `deferred_free` and freed at the start of the NEXT poll cycle.
This avoids invalidating the draining iterator or TLS pointers mid-cycle.

## Waker Design

Zero-allocation wakers. The `RawWaker` data pointer IS the task pointer.
The vtable is a static `&'static RawWakerVTable` shared by all wakers.

```
┌────────────────────┐
│      Waker         │
│  vtable: &VTABLE   │  ← static, shared
│  data: *mut u8     │  ← task pointer directly
└────────────────────┘
```

**Vtable operations:**
- `clone`: `ref_inc(task_ptr)`, return new `RawWaker` with same data
- `wake_by_ref`: check `is_completed` → check `try_set_queued` → push to ready
- `wake`: `wake_by_ref` + `ref_dec` (consumes the waker)
- `drop`: `ref_dec`, if `should_free` → push to deferred_free via TLS

**Cross-thread wakers** (tokio bridge only): Heap-allocated
`CrossTaskWakerData` with a separate vtable. Push to intrusive Vyukov
MPSC queue + poke eventfd. The executor drains this queue each cycle.

## IO Integration

Mio-backed. The `IoDriver` owns a `mio::Poll` and maps tokens to wakers.

```
Task registers socket:
  io.register(&mut socket, Interest::READABLE, cx.waker()) → Token

Runtime polls IO:
  io.poll_io(timeout) → fires wakers for ready tokens

Task receives wakeup:
  re-polls socket, reads data
```

**Token lifecycle:** Tokens are reused from a freelist. Stale wakeups
(token reused by a different socket) produce spurious wakeups — this is
expected and handled by the async contract (futures must re-check readiness).

**Epoll timeout:** Derived from `TimerDriver::next_deadline()`. If no
timers are pending, timeout is unbounded (park until IO or cross-thread wake).

## Timer Integration

`nexus-timer::Wheel` provides O(1) insert/cancel. The `TimerDriver` wraps
it with a pre-allocated expired-waker buffer.

```
Task creates Sleep:
  timer.schedule(deadline, cx.waker())

Runtime fires timers:
  timer.fire_expired(now) → wakes expired wakers

Task receives wakeup:
  checks Instant::now() >= deadline → Ready
```

**Waker update on re-poll:** If a `Sleep` future is polled with a different
waker (moved between tasks), it re-registers with the timer wheel. The
`will_wake()` comparison avoids redundant registration.

## Slab Allocation

Optional. Eliminates heap allocation on the spawn hot path by pre-allocating
fixed-size slots.

```
RuntimeBuilder::slab_unbounded(slab)  // growable
RuntimeBuilder::slab_bounded(slab)    // fixed capacity

spawn_slab(future)
  ├── Construct task on stack
  ├── copy_nonoverlapping into slab slot
  └── forget stack copy (slab owns the bytes)
```

**Three levels of control:**
- `spawn_slab(future)` — allocate + enqueue in one call
- `claim_slab()` → `SlabClaim` → `.spawn(future)` — reserve first, spawn later
- `try_claim_slab()` — non-blocking reserve (returns `None` if full)

The task header's `free_fn` is set at spawn time to the slab's free function.
The executor doesn't know which allocator was used — it calls `free_fn(ptr)`
and the function pointer handles the rest.

## World Integration

`WorldCtx` is a `Copy` handle (8 bytes — one raw pointer) to the nexus-rt
`World`. Tasks capture it cheaply and access ECS resources synchronously.

```rust
ctx.with_world(|world| {
    let books = world.resource_mut::<Books>();
    books.update(quote);
});
```

**Pre-resolved handlers:** For hot-path dispatch, resolve `Handler` parameter
IDs at setup time (one HashMap lookup per type), then dispatch with single-deref
access during the event loop:

```rust
// Setup (cold path):
let mut on_quote = handler_fn.into_handler(world.registry());

// Per-event (hot path):
ctx.with_world(|world| on_quote.run(world, quote));
```

## Tokio Bridge

Optional (`tokio-compat` feature). Two modes:

**`with_tokio(|| async { ... })`** — Run a tokio future on our executor.
Tokio provides reactor + timers; readiness fires our cross-thread waker.
Good for single operations (e.g., TLS handshake via `tokio-rustls`).

**`spawn_on_tokio(future)`** — Run on tokio's thread pool, deliver result
back via cross-thread waker. Good for blocking or CPU-heavy cold-path work
(e.g., `reqwest`, database queries).

A lazy tokio runtime (1 worker thread) is created on first use via `OnceLock`.

## Cross-Thread Wake

When a waker fires from a non-executor thread (tokio reactor, user thread):

```
Foreign thread:                     Executor thread:
  wake_task_cross_thread(ptr)         drain_cross_thread(inbox):
    try_set_queued(CAS)                 pop() → task_ptr
    queue.push(ptr)                     if completed → deferred_free
    poke eventfd                        else → incoming
```

The cross-wake queue is an intrusive Vyukov MPSC queue — each task's
`cross_next` field (AtomicPtr at offset 32) serves as the link pointer.
Zero allocation per wake.

## Shutdown

`ShutdownHandle` owns an `AtomicBool` flag and a waker slot. Signal handlers
(SIGTERM, SIGINT) set the flag and wake the stored waker.

Tasks await shutdown via `shutdown_signal().await`. The runtime checks the
flag at the top of each loop iteration and re-polls the root future.

Single-waiter design. For multi-waiter shutdown, use `CancellationToken`
(which supports hierarchical cancellation via parent/child relationships).
