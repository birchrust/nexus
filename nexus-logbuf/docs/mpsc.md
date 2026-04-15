# MPSC byte ring

`nexus_logbuf::queue::mpsc` — multi-producer, single-consumer
variable-length byte records.

## API

```rust
use nexus_logbuf::queue::mpsc;
use std::thread;

let (producer, mut consumer) = mpsc::new(64 * 1024);
let p1 = producer.clone();
let p2 = producer.clone();

thread::spawn(move || {
    let mut p1 = p1;
    if let Ok(mut claim) = p1.try_claim(7) {
        claim.copy_from_slice(b"hello A");
        claim.commit();
    }
});

thread::spawn(move || {
    let mut p2 = p2;
    if let Ok(mut claim) = p2.try_claim(7) {
        claim.copy_from_slice(b"hello B");
        claim.commit();
    }
});

// Consumer side (background thread):
while let Some(rec) = consumer.try_claim() {
    // record derefs to &[u8]
    let _ = &*rec;
    # break;
}
```

## How it works

Producers claim space via CAS on the tail offset. Each producer
reserves its own contiguous region, writes into it, and commits
by storing the `len` field atomically. Because each producer
owns a disjoint slice, they don't interfere.

The consumer still sees records in `tail`-order, but **commits
can arrive out of order**. If producer A claims first but
commits second, the consumer blocks at A's record until A
commits — even if B has already committed later in the buffer.
This is correct FIFO semantics; the trade-off is that one slow
producer can stall the consumer.

## Contention behaviour

Under heavy contention (many producers, small records) the CAS
loop on the tail offset can become the bottleneck. Measurements
at 2 producers show ~95 cycles p50 per claim, 7.27 GB/s with
typical workloads. At 4 producers, ~105 cycles p50, 6.29 GB/s.

If you need more than ~4 producers sustaining high rates,
consider per-producer SPSC logbufs plus a dedicated merge thread
that consumes from each and writes to a single sink. This
eliminates producer contention at the cost of an extra hop.

## Out-of-order commits

The consumer uses the same "len is commit marker" pattern as
SPSC: it waits for `len != 0` at the current `head`. If producer
A reserved a slot but hasn't committed yet, the consumer stalls
until A commits, even if other producers have committed later
records.

This is usually fine — producers claim and commit within a few
hundred cycles — but it means **never hold a WriteClaim across
a yield point, a blocking call, or a long computation**. Claim,
write, commit, all in the same tight sequence.

## Producer handles

`Producer` is `Clone` and `Send`. Each clone is an independent
handle; they coordinate via the shared tail atomic. There is no
per-producer buffer; they all write into the same ring.

Dropping a producer decrements an internal count. When all
producers are dropped, the consumer can detect disconnect via
`is_empty()` plus external logic. The raw `queue::mpsc` API does
not include an explicit `is_disconnected()` on the consumer side
— the [`channel::mpsc`](./channels.md) wrapper adds that.

## When to use MPSC logbuf

- Multiple worker threads want to journal events into one
  archival sink.
- Multiple sessions each produce telemetry into a shared
  monitoring buffer.
- You need one FIX message journal fed by multiple parser
  threads.

If you have exactly one producer, use SPSC — it's 2-3x faster at
p50 and has no CAS contention.
