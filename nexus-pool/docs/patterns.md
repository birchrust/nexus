# Patterns

## 1. Order pool (single-threaded event loop)

Pre-allocated orders for the entire session. The matching engine
pulls from the pool when a new client order arrives, fills in
the fields, and returns the order when it's filled or cancelled.

```rust
use nexus_pool::local::BoundedPool;

#[derive(Default)]
struct Order {
    id: u64,
    client_id: u32,
    symbol_id: u32,
    price: i64,
    qty: u64,
    filled: u64,
    side: u8,
    state: u8,
}

let orders = BoundedPool::new(
    8192,
    Order::default,
    |o| {
        *o = Order::default();
    },
);

// On new order:
if let Some(mut order) = orders.try_acquire() {
    order.id = 42;
    order.price = 100_00;
    order.qty = 10;
    // route into book...
} else {
    // Rate limit breached — reject back to client.
}
```

A fixed 8192-order pool is ~1 MiB (128 bytes/order), fits in L2,
and lets you reject overload at a bounded limit instead of
growing.

## 2. Reusable buffer pool (hot-path I/O)

WebSocket frames are variable-sized but almost always fit in a
4 KiB buffer. Pool them.

```rust
use nexus_pool::local::Pool;

let buffers = Pool::with_capacity(
    256,
    || Vec::<u8>::with_capacity(4096),
    |v| v.clear(),
);

loop {
    let mut buf = buffers.acquire();
    // read_frame(&mut buf);
    // decode_and_dispatch(&buf);
    drop(buf);
    # break;
}
```

Growable `Pool` will allocate more buffers if you exceed 256, but
the factory call is rare once the pool reaches its high-water mark.
Pre-populating via `with_capacity` avoids first-use jitter.

## 3. Parser → worker (sync::Pool, index-based)

The cleanest way to use `sync::Pool` with a queue: send an index
or opaque handle, not the `T` itself. The guard stays on the
worker side and drops when the work completes.

```rust
use nexus_pool::sync::Pool;
use nexus_queue::spmc;
use std::thread;

struct Frame {
    guard: nexus_pool::sync::Pooled<Vec<u8>>,
    // other fields...
}

let pool = Pool::new(
    1024,
    || Vec::<u8>::with_capacity(4096),
    |v| v.clear(),
);

let (tx, rx) = spmc::ring_buffer::<Frame>(1024);

let workers: Vec<_> = (0..4).map(|_| {
    let rx = rx.clone();
    thread::spawn(move || {
        while let Some(frame) = rx.pop() {
            // use frame.guard as &[u8]
            let _ = &*frame.guard;
            // drop(frame) returns buffer to pool on this worker thread
            # break;
        }
    })
}).collect();

// Parser thread:
let mut guard = pool.try_acquire().expect("pool exhausted");
guard.extend_from_slice(b"ws frame payload");
let frame = Frame { guard };
let _ = tx.push(frame);

drop(tx);
for w in workers { w.join().unwrap(); }
```

Key property: `Pooled<T>` is `Send` when `T: Send`. The guard
travels with the work item.

## 4. Per-strategy state pool

Strategy state is expensive to construct (opens files, allocates
lookup tables), but each tick only needs a short-lived scratch
copy.

```rust
use nexus_pool::local::Pool;
use std::collections::HashMap;

struct StrategyState {
    lookups: HashMap<u32, f64>,
    scratch: Vec<f64>,
}

impl StrategyState {
    fn new() -> Self {
        Self {
            lookups: HashMap::with_capacity(1024),
            scratch: Vec::with_capacity(256),
        }
    }
    fn reset(&mut self) {
        // Keep lookups (they're expensive to rebuild); clear scratch.
        self.scratch.clear();
    }
}

let states = Pool::with_capacity(
    16,
    StrategyState::new,
    |s| s.reset(),
);

let mut state = states.acquire();
state.scratch.push(1.0);
// run a computation...
drop(state);
```

Note the reset only clears the scratch buffer — the expensive
lookup table is preserved across reuses. This is pool-as-cache
rather than pool-as-initialize-every-time.

## 5. Interaction with `nexus-queue`

When passing pooled objects through a queue, you have two options:

### Option A: send the guard itself

Works for `sync::Pool` (guard is `Send`). The receiver drops the
guard, which returns the object to the pool. See pattern 3.

### Option B: send an index, keep the pool local

Works for any pool. Store the `T` (or its index) in a slab,
send the index through the queue, and let the receiver look it
up and return it on its own thread via a response queue.

```rust
use nexus_slab::bounded_slab::BoundedSlab;
use nexus_queue::spsc;

struct Msg { slot: u32 }

let mut slab: BoundedSlab<Vec<u8>> = BoundedSlab::new(1024);
let (tx, _rx) = spsc::ring_buffer::<Msg>(1024);

let key = slab.insert(b"payload".to_vec());
let _ = tx.push(Msg { slot: key.index() as u32 });
# let _ = slab;
```

This pattern lets you use a `local::Pool` or `BoundedSlab` even
when the work crosses thread boundaries — the slab stays on one
thread, indices travel.
