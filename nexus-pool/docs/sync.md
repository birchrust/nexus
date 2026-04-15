# sync::Pool

Thread-safe pool with **single-acquirer, multi-returner** semantics.
One thread owns the `Pool` handle and calls `try_acquire`. Any
thread can drop a `Pooled<T>` guard, which returns the value.

## API

```rust
use nexus_pool::sync::Pool;
use std::thread;

let pool = Pool::new(
    100,
    || Vec::<u8>::with_capacity(1024),
    |v| v.clear(),
);

// Acquirer thread owns `pool`. Pool is Send but not Sync or Clone.
let mut buf = pool.try_acquire().expect("pool exhausted");
buf.extend_from_slice(b"hello from acquirer");

// Hand the guard to another thread; it returns to the pool on drop.
let worker = thread::spawn(move || {
    // use buf...
    println!("{:?}", &*buf);
    // buf drops here → reset(&mut v) runs on the worker thread,
    // then the slot is pushed back onto the free list via CAS.
});
worker.join().unwrap();

// Acquirer can now re-acquire the same slot.
let _next = pool.try_acquire();
```

## How it works

- Slots live in a `Box<[Slot<T>]>` allocated once at construction.
- The free list is an intrusive stack with a single `AtomicUsize`
  head and a `next: AtomicUsize` in each slot.
- `try_acquire` (acquirer thread only) pops via CAS on the head.
  Spin-retries on CAS failure (bounded by concurrent returners).
- `drop(Pooled<T>)` (any thread) runs `reset`, then pushes via CAS
  on the head.

Because only one thread acquires, there's no ABA risk on the
acquire path — there's only ever one popper, and pops always
happen after the pusher's `Release` synchronizes the value write
with the acquirer's `Acquire`.

## Performance

| Operation | p50 cycles |
|---|---|
| `try_acquire` | ~42 |
| release (drop guard) | ~68 p50, ~86 p99 |

Acquire is slightly cheaper than release because release has to
run the `reset` closure and do a CAS push. Both are sub-100
cycles even under concurrent return from several threads.

Compared to `crossbeam::ArrayQueue` used as a pool, sync::Pool is
~1.5-2x faster at p50 and much better at p99 under contention.

## Constraints

- **One acquirer.** `Pool<T>` is `Send` but not `Sync` or `Clone`.
  You physically cannot share it between acquirer threads.
- **T: Send.** Values cross threads when guards are shipped to
  worker threads.
- **reset: Fn + Send + Sync.** The reset closure runs on the
  returning thread, so it must be thread-safe.

## Example: market data parser with worker fan-out

```rust
use nexus_pool::sync::Pool;
use nexus_queue::spmc;
use std::thread;

struct ParsedFrame {
    buf: Vec<u8>,  // owned payload, pooled
}

// Parser thread owns the pool. Workers receive frames via SPMC
// queue and drop them (which returns buffers to the pool).
let pool = Pool::new(
    1024,
    || Vec::<u8>::with_capacity(4096),
    |v| v.clear(),
);

let (tx, rx) = spmc::ring_buffer::<ParsedFrame>(1024);

let workers: Vec<_> = (0..4).map(|_| {
    let rx = rx.clone();
    thread::spawn(move || {
        while let Some(frame) = rx.pop() {
            // process(&frame);
            drop(frame);  // buffer returns to pool from this thread
            # break;
        }
    })
}).collect();

// Parser (acquirer) thread — but in this sketch we run inline.
let mut buf = pool.try_acquire().unwrap();
buf.extend_from_slice(b"market frame");
let frame = ParsedFrame { buf: std::mem::take(&mut *buf) };
// In real code you'd keep the Pooled guard and push the guard itself,
// or use an index-based scheme. See patterns.md.
drop(buf);
let _ = tx.push(frame);

drop(tx);
for w in workers { w.join().unwrap(); }
```

See [patterns.md](./patterns.md) for the proper index-based pattern
that keeps the `Pooled` guard alive across the queue.

## When to use

- You have a producer thread that allocates work and N worker
  threads that consume it.
- The worker threads need to release the object back to the pool.
- You don't want to send the object back via another queue.

If workers never release (they forward to another stage), use a
`local::Pool` on the producer and send values by index.

If multiple threads need to acquire, you're in MPMC territory —
see [overview.md](./overview.md) for why that's not supported.
