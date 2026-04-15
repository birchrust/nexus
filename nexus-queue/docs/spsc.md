# SPSC

Single producer, single consumer. The simplest and fastest variant.

## API

```rust
use nexus_queue::spsc;

// Capacity is rounded up to the next power of two.
let (tx, rx) = spsc::ring_buffer::<u64>(1024);

// Producer: returns Err(Full(value)) when full.
tx.push(42).unwrap();

// Consumer: returns None when empty.
assert_eq!(rx.pop(), Some(42));

// Check the other side.
assert!(!tx.is_disconnected());
assert!(!rx.is_disconnected());
assert_eq!(tx.capacity(), 1024);
```

`Producer<T>` and `Consumer<T>` are each `Send` but not `Sync` — the
whole point is that exactly one thread uses each. Move `tx` to the
producer thread, `rx` to the consumer thread.

## Semantics

- `push(value)`: writes to the slot at `tail`, stamps the lap
  counter, advances `tail`. Returns `Err(Full(value))` if the slot's
  lap counter is not yet `tail` (consumer hasn't drained it).
- `pop()`: reads from the slot at `head`, stamps the lap counter,
  advances `head`. Returns `None` if the slot's lap counter is not
  yet `head + 1` (producer hasn't written it).

Both operations are wait-free: no loops, no retries, no spinning.
If the queue is full/empty you get the failure result immediately.

## Wrap-around

There is no wrap-around bookkeeping. Indices are free-running
`usize` values — they wrap naturally every `2^64` operations on a
64-bit system. The slot is computed as `index & mask`. At 100M
pushes per second you wrap in 5800 years.

## Performance notes

SPSC has two optimizations that matter:

1. **Cache-line isolation.** `head` and `tail` live on separate
   cache lines (128 bytes of padding) so the producer writing tail
   doesn't invalidate the consumer's view of head, and vice versa.
   Without this, you pay a coherence round-trip on every push/pop.

2. **Cached opposite-side index.** The producer remembers the last
   value of `head` it saw, and only re-reads the atomic when its
   cache says the queue looks full. Same for the consumer with
   `tail`. Under load, this turns almost every push into a
   local-only operation.

Result: p50 of ~200 cycles for a push-pop pair, 113M msg/sec
sustained throughput (see [performance.md](./performance.md)).

## Example: feed handler to matcher

```rust
use nexus_queue::spsc;
use std::thread;

#[derive(Debug, Clone, Copy)]
struct Tick {
    symbol_id: u32,
    price: i64,
    size: u64,
    ts_ns: u64,
}

let (tx, rx) = spsc::ring_buffer::<Tick>(16 * 1024);

// Feed handler thread: parses exchange frames, pushes ticks.
let feed = thread::spawn(move || {
    loop {
        let tick = Tick { symbol_id: 1, price: 100_00, size: 10, ts_ns: 0 };
        // If the matcher is backed up, we drop the tick and record it.
        // Blocking the feed handler is never acceptable.
        if tx.push(tick).is_err() {
            // record.drop_count += 1;
        }
        # break;
    }
});

// Matching engine thread: pops ticks, updates book.
let matcher = thread::spawn(move || {
    while let Some(tick) = rx.pop() {
        // update_book(tick);
        # let _ = tick;
        # break;
    }
});

feed.join().unwrap();
matcher.join().unwrap();
```

## Drop behavior

- Dropping `Producer<T>`: `rx.is_disconnected()` becomes true.
  Remaining values in the buffer can still be popped.
- Dropping `Consumer<T>`: `tx.is_disconnected()` becomes true.
  Further pushes still succeed until the queue fills, then fail
  with `Full`.
- Dropping both: the backing buffer is freed and any unread values
  are dropped in place.
