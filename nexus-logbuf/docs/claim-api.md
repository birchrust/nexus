# Claim API

Both producer and consumer use RAII claim types. Understanding
their contracts is essential because a leaked claim can deadlock
the buffer.

## `WriteClaim<'a>`

Returned by `Producer::try_claim(len)`. Derefs to `&mut [u8]` of
exactly `len` bytes.

```rust
use nexus_logbuf::queue::spsc;

let (mut tx, _rx) = spsc::new(4096);

let mut claim = tx.try_claim(11).unwrap();   // reserve 11 bytes
claim.copy_from_slice(b"hello world");       // write payload
claim.commit();                               // publish
```

- `commit()` — writes the `len` field atomically with release
  ordering. The record becomes visible to the consumer.
- `drop(claim)` without commit — writes a **skip marker** of the
  same length. The consumer sees the skip and advances past the
  dead region. Correct but wastes the reserved space.

### Why drop-without-commit writes a skip

Because `tail` has already advanced past the claimed region, the
producer can't simply "un-reserve" the space — another producer
might have already claimed beyond it (in MPSC), or the next
claim in this producer will be at the advanced tail. The skip
marker tells the consumer "there's nothing here, move on".

## `ReadClaim<'a>`

Returned by `Consumer::try_claim()`. Derefs to `&[u8]`.

```rust
use nexus_logbuf::queue::spsc;

let (_tx, mut rx) = spsc::new(4096);

if let Some(rec) = rx.try_claim() {
    // rec: &[u8]
    process(&rec);
    // drop(rec) zeros the region and advances head
}

fn process(_: &[u8]) {}
```

- `drop(rec)` zeros the header and payload, then advances `head`
  past the record.
- You cannot `commit` a read claim — it's read-only.

### Why zero on drop?

Because `len == 0` means "not committed". If the consumer
advances `head` without zeroing, a future producer might reclaim
this slot, but the old (non-zero) `len` bytes could confuse a
second consumer somewhere — or more practically, the producer's
commit store has to be the thing that flips `len` from zero to
non-zero, and that only works if the prior value was zero.

See [consumer-zeroing.md](./consumer-zeroing.md) for the full
rationale and the Aeron precedent.

## The `mem::forget` deadlock warning

**This is critical.** Do not `mem::forget` a `WriteClaim` or
`ReadClaim`. Doing so causes an unrecoverable deadlock of the
buffer.

### Why?

The entire protocol depends on every claim eventually running
its destructor:

- `WriteClaim::drop` — writes either the commit length or a skip
  marker, advancing the buffer into a well-defined state.
- `ReadClaim::drop` — zeros the record and advances `head`.

If you `mem::forget` the claim, neither happens. The
corresponding slot is stuck in limbo forever:

- **Forgotten WriteClaim**: the producer has advanced `tail`
  past the slot, but `len` is still zero. The consumer waits
  forever at this slot. Every future producer will eventually
  wrap back to this slot and find the tail beyond it, also
  waiting. The buffer is dead.
- **Forgotten ReadClaim**: the consumer has not advanced `head`.
  The next `try_claim` sees the same record again; the producer
  eventually fills the buffer and starts returning `Full`.

There is no recovery from this short of destroying and
recreating the buffer. The documentation in the source calls
this out as "not undefined behavior, but causes an unrecoverable
deadlock."

### How to avoid it

- **Don't call `mem::forget`.** It's almost never correct to
  forget *any* RAII type; doubly so for these.
- **Don't store claims in `ManuallyDrop`** unless you are
  certain you will drop them manually.
- **Don't leak claims through `Box::leak` or similar.** Leaking
  an `Rc`/`Arc` cycle that transitively contains a claim is also
  bad.
- **Don't let panics skip the claim drop.** Standard unwinding
  runs destructors, so normal panics are fine. The danger is
  custom code that intercepts panics and forgets the claim.

### What if I need to abort cleanly?

Just drop the `WriteClaim`. It writes a skip marker; the buffer
continues working. If you want a more explicit abort API, let me
know — the primitive is there, the public method is missing.

## Lifetime and borrow semantics

`WriteClaim<'a>` borrows `&'a mut Producer`. While a claim is
alive, you cannot call `try_claim` on the producer again — which
is what you want, because overlapping claims are meaningless.
Commit or drop the claim before claiming again.

`ReadClaim<'a>` borrows `&'a mut Consumer` similarly. You cannot
have two simultaneous read claims on one consumer. Records are
consumed strictly in order.

## The `copy_from_slice` helper

`WriteClaim` derefs to `&mut [u8]`, so all standard slice
methods work: `copy_from_slice`, indexing, iteration,
`chunks_mut`, etc. There is no special fast-path method — just
use `copy_from_slice`, which is already `memcpy` in release
builds.
