# SPMC

Single producer, multiple consumers. Each value is consumed by
**exactly one** consumer — this is work-stealing fan-out, not
broadcast.

## API

```rust
use nexus_queue::spmc;
use std::thread;

let (tx, rx) = spmc::ring_buffer::<u64>(1024);
let rx2 = rx.clone();
let rx3 = rx.clone();

tx.push(1).unwrap();
tx.push(2).unwrap();
tx.push(3).unwrap();

// Each consumer gets a distinct value; order of who-gets-what is
// decided by CAS race on head.
let a = thread::spawn(move || rx.pop());
let b = thread::spawn(move || rx2.pop());
let c = thread::spawn(move || rx3.pop());

let vals: std::collections::BTreeSet<_> =
    [a.join().unwrap(), b.join().unwrap(), c.join().unwrap()]
        .into_iter().flatten().collect();
assert_eq!(vals, [1, 2, 3].into_iter().collect());
```

`Producer<T>: Send` but not `Sync`. `Consumer<T>: Clone + Send + Sync`.

## How it works

Mirror of MPSC with roles swapped. The single producer writes
directly with no CAS (it has exclusive ownership of the tail).
Consumers race on the head index via CAS: whoever successfully
claims a head slot gets the value.

Per-slot lap counters synchronize producer and consumer as usual:
producer stamps on write, consumer stamps on read. A slow consumer
that has claimed a slot but hasn't yet finished reading it does
**not** block other consumers — they claim subsequent slots and
proceed. The producer only blocks when it catches up to the
oldest unread slot (i.e., the queue is actually full).

## Not broadcast

SPMC means work is distributed. If you push ticks into an SPMC and
want all strategies to see every tick, this is the wrong tool — use
SPSC per strategy and duplicate in the feed handler, or use a
different primitive designed for broadcast.

The classic SPMC use case is a single IO or parser thread producing
work items for a pool of workers, any of which can handle any item.

## Example: market data decode fan-out

```rust
use nexus_queue::spmc;
use std::thread;

#[derive(Debug, Clone, Copy)]
struct Frame {
    len: u16,
    payload_idx: u32, // index into a shared buffer pool
}

let (tx, rx) = spmc::ring_buffer::<Frame>(8 * 1024);

// Four decoder workers share the consumer handle.
let workers: Vec<_> = (0..4).map(|_| {
    let rx = rx.clone();
    thread::spawn(move || {
        while let Some(frame) = rx.pop() {
            // decode_and_dispatch(frame);
            let _ = frame;
            # break;
        }
    })
}).collect();

// IO thread pushes raw frames as they arrive.
let io = thread::spawn(move || {
    for i in 0..16 {
        let _ = tx.push(Frame { len: 64, payload_idx: i });
    }
});

io.join().unwrap();
for w in workers { w.join().unwrap(); }
```

## Disconnect

- `Consumer::is_disconnected()` — true when the producer is dropped.
  Consumers continue popping buffered items until empty.
- `Producer::is_disconnected()` — true when **all** consumers have
  been dropped. Pushes still succeed until full.

Consumer count is tracked by `Clone` and `Drop` impls.

## Load balancing

This is a FIFO queue, not a work-stealing deque. Workers don't
steal from each other — they all pull from the same head. That
means:

- No per-worker locality (any worker may get any item).
- No priority within workers.
- Fair-ish distribution under uniform contention, but a worker
  that's faster at CAS will tend to win more items.

If you need locality-aware scheduling, use a per-worker SPSC and
dispatch on the producer side with a hashing function.
