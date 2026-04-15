# Patterns

Recipes for common topologies. Each example is a working sketch;
adapt to your own types and error handling.

## 1. Order entry queue (SPSC)

One session thread accepts client orders; the matching engine pops
them in FIFO order.

```rust
use nexus_queue::spsc;
use std::thread;

#[derive(Debug)]
struct Order {
    client_id: u32,
    symbol_id: u32,
    price: i64,
    qty: u64,
    side: u8,
}

let (tx, rx) = spsc::ring_buffer::<Order>(4096);

let session = thread::spawn(move || {
    // Client orders arrive via whatever protocol; push into the
    // matcher. On failure, reject the order back to the client.
    let order = Order { client_id: 7, symbol_id: 1, price: 100_00, qty: 10, side: 0 };
    match tx.push(order) {
        Ok(()) => { /* ACK */ }
        Err(full) => { /* NACK, matcher backed up */ let _ = full; }
    }
});

let matcher = thread::spawn(move || {
    while let Some(order) = rx.pop() {
        // match_order(order);
        let _ = order;
        # break;
    }
});

session.join().unwrap();
matcher.join().unwrap();
```

## 2. Market data fan-out (SPMC)

One feed handler parses frames and hands decoded ticks to a pool of
strategy workers. Each tick is processed by exactly one worker.

```rust
use nexus_queue::spmc;
use std::thread;

#[derive(Clone, Copy)]
struct Tick { symbol_id: u32, price: i64, qty: u64 }

let (tx, rx) = spmc::ring_buffer::<Tick>(16 * 1024);

let workers: Vec<_> = (0..8).map(|_| {
    let rx = rx.clone();
    thread::spawn(move || {
        while let Some(t) = rx.pop() {
            // update_model(t);
            let _ = t;
            # break;
        }
    })
}).collect();

let feed = thread::spawn(move || {
    let t = Tick { symbol_id: 1, price: 100_00, qty: 5 };
    let _ = tx.push(t);
});

feed.join().unwrap();
for w in workers { w.join().unwrap(); }
```

If you instead need every strategy to see every tick, use multiple
SPSCs and have the feed handler push into each. SPMC is for work
distribution, not broadcast.

## 3. Log aggregation (MPSC)

Many worker threads push log records into a single sink thread that
writes them to disk or ships them off-box.

```rust
use nexus_queue::mpsc;
use std::thread;

struct LogRec { level: u8, ts_ns: u64, msg: &'static str }

let (tx, rx) = mpsc::ring_buffer::<LogRec>(64 * 1024);

let workers: Vec<_> = (0..4).map(|_| {
    let tx = tx.clone();
    thread::spawn(move || {
        let _ = tx.push(LogRec { level: 2, ts_ns: 0, msg: "tick processed" });
    })
}).collect();
drop(tx); // let consumer observe disconnect once workers finish

let sink = thread::spawn(move || {
    while !rx.is_disconnected() {
        if let Some(rec) = rx.pop() {
            // writer.write(rec);
            let _ = rec;
        }
    }
    // Drain remaining.
    while let Some(rec) = rx.pop() {
        let _ = rec;
    }
});

for w in workers { w.join().unwrap(); }
sink.join().unwrap();
```

For high-volume log/archival workloads, [`nexus-logbuf`](../../nexus-logbuf/)
is a better fit — it stores raw bytes with variable length and
avoids per-record `T` overhead.

## 4. Backpressure: what to do on `Full`

`push` returns `Err(Full(value))` with the value intact. Your
choices:

| Strategy | When to use |
|---|---|
| Drop and count | Market data ticks where stale is worse than lost. Track drops as a health metric. |
| Retry with brief spin | Very short bursts; you have a clear bound on contention. |
| Reject upstream | Order entry — NACK back to client. Preserves determinism. |
| Switch topology | If you're Full often, you may need a bigger queue *or* a different consumer design. |

```rust
use nexus_queue::spsc;
let (tx, _rx) = spsc::ring_buffer::<u32>(16);
# let _rx = _rx;

let value = 42u32;
match tx.push(value) {
    Ok(()) => { /* happy path */ }
    Err(nexus_queue::Full(rejected)) => {
        // Record metric, fallback, or return to caller.
        let _ = rejected;
    }
}
```

**Never** sleep or park on the producer side of a hot-path feed.
The whole point of lock-free is that producers never block. If you
need blocking, use [`nexus-channel`](../../nexus-channel/).

## 5. Pairing with `nexus-pool`

Queues move `T`, but if `T` owns a heap allocation (e.g., a
`Vec<u8>` payload), you'll allocate on the producer and free on the
consumer — unacceptable on a hot path. Pair with a pool:

```rust
use nexus_pool::sync::Pool;
use nexus_queue::spsc;

struct Msg { buf_idx: u32, len: u16 } // index into the pool

let pool = Pool::new(1024, || vec![0u8; 4096], |v| v.clear());
let (tx, _rx) = spsc::ring_buffer::<Msg>(1024);

// Producer: acquire from pool, fill, push the index.
// Consumer: pop index, use buffer, release back to pool.
# let _ = (pool, tx);
```

The queue itself stays small (`Msg` is 8 bytes); the heavy payloads
live in the pool and get reused.
