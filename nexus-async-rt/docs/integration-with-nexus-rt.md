# Integration with nexus-rt

nexus-rt is a **dispatch framework** — Handlers, Pipelines, DAGs, Templates,
Reactors. It has no executor: you drive it from a poll loop.

nexus-async-rt is an **async executor** — single-threaded, slab-backed,
mio-driven. It provides a poll loop that can host nexus-rt's world.

Together they form a complete runtime: async tasks drive IO and drive
nexus-rt handlers, all on one thread.

## The Big Picture

```text
+----------------------------------------+
|  Runtime::block_on()                   |  <- nexus-async-rt
|                                        |
|    async tasks   (sockets, timers)     |
|        |                               |
|        | with_world(|w| handler.run()) |
|        v                               |
|    World / Resources  (nexus-rt)       |
+----------------------------------------+
```

The `World` is owned by the `Runtime`. Async tasks access it via
`with_world` — a synchronous, scoped borrow — whenever they need to update
state or dispatch a handler.

## `with_world` and `with_world_ref`

```rust
pub fn with_world<R>(f: impl FnOnce(&mut nexus_rt::World) -> R) -> R;
pub fn with_world_ref<R>(f: impl FnOnce(&nexus_rt::World) -> R) -> R;
```

Both borrow the World from thread-local storage (the Runtime puts it there
during `block_on`). Outside a `block_on` they panic.

```rust
use nexus_async_rt::{Runtime, spawn_boxed, with_world, with_world_ref};
use nexus_rt::{Resource, WorldBuilder};

#[derive(Resource, Default)]
struct Counter(u64);

fn main() {
    let mut world = WorldBuilder::new()
        .with_resource(Counter::default())
        .build();
    let mut rt = Runtime::new(&mut world);

    rt.block_on(async {
        spawn_boxed(async {
            // Mutable access — scoped to the closure.
            with_world(|w| {
                w.resource_mut::<Counter>().0 += 1;
            });

            // Read-only access.
            let n = with_world_ref(|w| w.resource::<Counter>().0);
            assert_eq!(n, 1);
        })
        .await;
    });
}
```

**Rules:**

- Do not call `with_world` recursively from inside another `with_world`
  closure — it will panic (World is already borrowed).
- Do not hold references across `.await`. The borrow must end before any
  yield point.
- Handlers, Pipelines, and Reactors are `!Send` — that's fine, the runtime
  is single-threaded.

## Pre-resolving Handlers at Setup Time

The fastest dispatch pattern: resolve Handler parameters **once** at setup,
then dispatch repeatedly from async tasks without re-resolution.

```rust
use nexus_async_rt::{Runtime, spawn_boxed, with_world};
use nexus_rt::{IntoHandler, Handler, Res, ResMut, Resource, WorldBuilder};

#[derive(Resource, Default)]
struct OrderBook { /* ... */ }

#[derive(Resource, Default)]
struct Metrics { updates: u64 }

struct Tick { price: f64, qty: f64 }

fn on_tick(mut book: ResMut<OrderBook>, mut m: ResMut<Metrics>, tick: Tick) {
    // Update the book.
    let _ = (&mut *book, tick);
    m.updates += 1;
}

fn main() {
    let mut world = WorldBuilder::new()
        .with_resource(OrderBook::default())
        .with_resource(Metrics::default())
        .build();

    // Resolve the handler ONCE — IDs are cached inside.
    let mut handler = on_tick.into_handler(world.registry());

    let mut rt = Runtime::new(&mut world);
    rt.block_on(async move {
        spawn_boxed(async move {
            let tick = recv_tick().await;

            // Dispatch is one World borrow + the handler's pre-resolved fetch.
            with_world(|w| handler.run(w, tick));
        })
        .await;
    });
}

async fn recv_tick() -> Tick { Tick { price: 0.0, qty: 0.0 } }
```

This is the canonical pattern for market data loops: the async task owns
the socket and the parsing state, and every parsed message goes through a
pre-resolved handler. Dispatch cost is one `with_world` borrow plus the
handler's parameter fetch (~1 cycle per `Res`/`ResMut`).

## `WorldCtx` — A Copy Handle for Capturing

`WorldCtx` is a zero-cost `Copy` handle that implements `with_world` /
`with_world_ref` without needing the thread-local. It captures cleanly into
multiple tasks and makes the "I need World access here" contract explicit
in signatures.

```rust
pub struct WorldCtx { /* Copy */ }

impl WorldCtx {
    pub fn new(world: &mut World) -> Self;
    pub fn with_world<R>(&self, f: impl FnOnce(&mut World) -> R) -> R;
    pub fn with_world_ref<R>(&self, f: impl FnOnce(&World) -> R) -> R;
}
```

```rust
use nexus_async_rt::{Runtime, WorldCtx, spawn_boxed};
use nexus_rt::{Resource, WorldBuilder};

#[derive(Resource, Default)]
struct Stats { msgs: u64 }

fn main() {
    let mut world = WorldBuilder::new()
        .with_resource(Stats::default())
        .build();

    let mut rt = Runtime::new(&mut world);
    rt.block_on(async {
        let ctx = WorldCtx::new(nexus_async_rt::with_world(|w| w as *mut _ as usize) as _);
        // (in practice WorldCtx::new is called from the runtime setup)
        let _ = ctx;

        for _ in 0..4 {
            spawn_boxed(async move {
                nexus_async_rt::with_world(|w| {
                    w.resource_mut::<Stats>().msgs += 1;
                });
            });
        }
    });
}
```

In practice you construct `WorldCtx` before entering `block_on` and pass it
into your task-spawning code. It's `Copy`, so it captures into as many
closures as you like with no reference-counting cost.

## Driving a Pipeline From an Async Task

Pipelines are just `Handler`s — the same pattern works.

```rust
use nexus_async_rt::{Runtime, spawn_boxed, with_world};
use nexus_rt::{Handler, Pipeline, Res, ResMut, Resource, WorldBuilder};

#[derive(Resource, Default)]
struct Book { bids: u64, asks: u64 }

fn validate(tick: u64) -> Option<u64> {
    if tick > 0 { Some(tick) } else { None }
}

fn apply(mut book: ResMut<Book>, tick: u64) {
    book.bids += tick;
}

fn main() {
    let mut world = WorldBuilder::new()
        .with_resource(Book::default())
        .build();

    let reg = world.registry();
    let mut pipeline = Pipeline::<u64>::new()
        .filter_map(validate, &reg)
        .then(apply, &reg)
        .build();

    let mut rt = Runtime::new(&mut world);
    rt.block_on(async move {
        spawn_boxed(async move {
            for tick in 1..=10 {
                with_world(|w| pipeline.run(w, tick));
            }
        }).await;
    });
}
```

## Complete Example: WebSocket → OrderBook

Market data websocket task parses messages and updates a nexus-rt Resource.
A separate task subscribes via an `EventReader`-style reactor (out of scope
here — see nexus-rt `reactors.md`).

```rust
use nexus_async_rt::{
    Runtime, TcpStream, spawn_boxed, with_world, shutdown_signal,
};
use nexus_rt::{Handler, IntoHandler, ResMut, Resource, WorldBuilder};

#[derive(Resource, Default)]
struct OrderBook {
    best_bid: f64,
    best_ask: f64,
}

struct Quote { bid: f64, ask: f64 }

fn apply_quote(mut book: ResMut<OrderBook>, q: Quote) {
    book.best_bid = q.bid;
    book.best_ask = q.ask;
}

fn main() -> std::io::Result<()> {
    let mut world = WorldBuilder::new()
        .with_resource(OrderBook::default())
        .build();

    // Resolve once, dispatch many.
    let mut handler = apply_quote.into_handler(world.registry());

    let mut rt = Runtime::new(&mut world);
    rt.block_on(async move {
        spawn_boxed(async move {
            let mut stream = connect_ws().await?;
            loop {
                let q = read_quote(&mut stream).await?;
                with_world(|w| handler.run(w, q));
            }
            #[allow(unreachable_code)]
            Ok::<_, std::io::Error>(())
        });

        shutdown_signal().await;
        Ok(())
    })
}

async fn connect_ws() -> std::io::Result<TcpStream> { todo!() }
async fn read_quote(_s: &mut TcpStream) -> std::io::Result<Quote> { todo!() }
```

## Anti-patterns

- **Holding a World reference across `await`:** the borrow checker will stop
  you at compile time. Good.
- **Recursive `with_world`:** runtime panic. Refactor the inner call to take
  `&mut World` as a parameter, or do the work after the outer borrow ends.
- **Spawning a task, then awaiting it from inside a `with_world` closure:**
  you cannot `await` inside the closure — the closure is synchronous. Spawn
  outside, await outside, call `with_world` at the boundaries.
- **Resolving handlers on every dispatch:** use `into_handler(registry)`
  once at setup. The cost is the HashMap lookups, which are zero per
  subsequent `run`.

## See Also

- [Task Spawning](task-spawning.md) — spawn strategies
- [nexus-rt handlers.md](../../nexus-rt/docs/handlers.md) — Handler traits
  and `IntoHandler`
- [nexus-rt world.md](../../nexus-rt/docs/world.md) — Resource model
- [Patterns](patterns.md) — end-to-end recipes
