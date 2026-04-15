# MPSC

Multi-producer, single-consumer. Clone the `Producer` to share it
across threads; only one `Consumer` exists.

## API

```rust
use nexus_queue::mpsc;
use std::thread;

let (tx, rx) = mpsc::ring_buffer::<u64>(1024);
let tx2 = tx.clone();

let a = thread::spawn(move || { tx.push(1).unwrap(); });
let b = thread::spawn(move || { tx2.push(2).unwrap(); });
a.join().unwrap();
b.join().unwrap();

// Both values are in the queue, in some order.
let first = rx.pop().unwrap();
let second = rx.pop().unwrap();
assert!((first == 1 && second == 2) || (first == 2 && second == 1));
```

`Producer<T>: Clone + Send + Sync`. `Consumer<T>: Send` but not
`Sync` — only one thread pops.

## How it works (Vyukov MPSC)

Producers race to claim a tail slot via CAS on the tail index. On a
successful claim, the producer has exclusive ownership of that
slot's index, but the slot itself may still be in use by an earlier
wrap (if the queue is full or a prior producer is slow). The
producer then:

1. Reads the slot's lap counter (sequence).
2. If `sequence == claimed_index`, the slot is free — write the
   value and stamp `sequence = claimed_index + 1`.
3. If `sequence < claimed_index`, the slot is still occupied from a
   previous lap. The queue is full; the claim is released logically
   by noting the slot isn't advanced. `try_push` returns `Full`.

The consumer does the mirror: wait for `sequence == head + 1`,
read, stamp with `head + capacity`, advance `head`.

## Contention and backoff

When many producers hammer the same tail, the CAS loop will spin.
`try_push` does **not** block or park — if the CAS fails repeatedly
because the queue is full, you get `Full<T>` back. The spin is
bounded by the number of concurrent producers, not by how full the
queue is.

If you need a blocking MPSC with parking semantics, wrap this in
your own condvar/eventfd notifier (or use `std::sync::mpsc` if
latency doesn't matter).

## Disconnect detection

- `Producer::is_disconnected()` — true when the consumer has been
  dropped. Pushes still succeed until full.
- `Consumer::is_disconnected()` — true when **all** producers have
  been dropped. Pops still return buffered values until empty, then
  return `None`.

The producer count is tracked internally; cloning increments it,
dropping a producer decrements it.

## Example: log aggregation

```rust
use nexus_queue::mpsc;
use std::thread;

#[derive(Debug)]
struct LogRecord {
    thread_id: u32,
    ts_ns: u64,
    msg: &'static str,
}

let (tx, rx) = mpsc::ring_buffer::<LogRecord>(64 * 1024);

// Four worker threads, each with their own clone of the producer.
let workers: Vec<_> = (0..4).map(|id| {
    let tx = tx.clone();
    thread::spawn(move || {
        for i in 0..100 {
            let _ = tx.push(LogRecord {
                thread_id: id,
                ts_ns: i,
                msg: "work done",
            });
        }
    })
}).collect();

// Drop the original producer so the consumer can observe disconnect
// once all workers finish.
drop(tx);

// Consumer thread (single): drain and write to disk.
let drain = thread::spawn(move || {
    let mut count = 0;
    while !rx.is_disconnected() || !matches!(rx.pop(), None) {
        if let Some(rec) = rx.pop() {
            count += 1;
            // write_to_file(rec);
            let _ = rec;
        }
    }
    count
});

for w in workers { w.join().unwrap(); }
let _ = drain.join().unwrap();
```

For high-throughput logging, consider [`nexus-logbuf`](../../nexus-logbuf/)
instead — it handles variable-length byte records directly and
avoids the per-message `T` overhead.
