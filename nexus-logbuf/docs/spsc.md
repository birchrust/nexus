# SPSC byte ring

`nexus_logbuf::queue::spsc` — single producer, single consumer,
variable-length byte records.

## API

```rust
use nexus_logbuf::queue::spsc;

// Capacity is rounded up to the next power of two. Measured in bytes.
let (mut producer, mut consumer) = spsc::new(64 * 1024);

// Producer (hot path).
let payload: &[u8] = b"tick: BTC 52100.50 qty 1.2";
match producer.try_claim(payload.len()) {
    Ok(mut claim) => {
        claim.copy_from_slice(payload);
        claim.commit();
    }
    Err(e) => {
        // Full or zero-length.
        let _ = e;
    }
}

// Consumer (background).
if let Some(record) = consumer.try_claim() {
    // record derefs to &[u8]
    assert_eq!(&*record, b"tick: BTC 52100.50 qty 1.2");
    // Drop zeros the record region and advances the read head.
}
```

## Record layout

Each record on the wire is:

```text
┌──────────────┬─────────────────────────────┐
│ len (usize)  │  payload (len bytes, pad 8) │
└──────────────┴─────────────────────────────┘
```

- `len` is a `usize` stored atomically. Zero means "not yet
  committed" — the consumer waits.
- Payloads are padded up to an 8-byte boundary so the next
  record's `len` field is word-aligned.
- The high bit of `len` is the skip marker (see
  [consumer-zeroing.md](./consumer-zeroing.md)).

## Producer — `try_claim`

```rust
pub fn try_claim(&mut self, len: usize) -> Result<WriteClaim<'_>, TryClaimError>;
```

Reserves space for a record of `len` bytes (plus header and
alignment padding).

Returns:
- `Ok(WriteClaim)` — you now own a `&mut [u8]` of exactly `len`
  bytes. Write your payload, then call `claim.commit()`.
- `Err(TryClaimError::Full)` — not enough contiguous space.
- `Err(TryClaimError::ZeroLength)` — `len == 0` is not allowed
  because zero is the "not committed" sentinel.

If the remaining space to the end of the buffer is too small,
the producer writes a **skip marker** of that size and advances
`tail` to the wrap point, then re-attempts the claim from the
beginning. This is transparent to the caller but costs one extra
atomic store on the wrap boundary.

## Producer — `commit` / `abort`

`WriteClaim` is RAII:

- `claim.commit()` — writes `len` atomically with release
  ordering, making the record visible.
- `drop(claim)` without commit — writes a skip marker with the
  same length, so the consumer advances past the region.

Either way, the slot is always either a valid record or a valid
skip. There is no "torn" state.

**Critical:** see [claim-api.md](./claim-api.md) for the
`mem::forget` deadlock warning.

## Consumer — `try_claim`

```rust
pub fn try_claim(&mut self) -> Option<ReadClaim<'_>>;
```

Reads the record at the current `head`. Returns:

- `Some(ReadClaim)` — a `&[u8]` view of the payload. Drop it
  (explicitly or by scope end) to zero the region and advance.
- `None` — either the buffer is empty, or the next record is not
  yet committed (len still zero).

Skip markers are handled inside `try_claim`: if the head points
at a skip, the consumer zeros the skip region, advances `head`
by the skip length, and retries automatically. You never see
skips at the public API.

## Performance

On a 3.1 GHz Intel Core i9, SPSC logbuf hits:

- ~40 cycles p50 for `try_claim + copy_from_slice + commit` on
  128-byte records.
- 20.7 GB/s sustained throughput with 2 KiB records (the buffer
  is the bottleneck, not the protocol).

Compared to naive "allocate, fill, send on mpsc channel":
~50-100x faster at p50 for short records, because there's no
allocation and the synchronization is a single release store.

## Capacity sizing

logbuf is a **byte** buffer, not a record buffer. Size it for
the peak burst **in bytes**, not in record count.

Rule of thumb: at least 4x the largest burst you've observed in
bytes, and always a power of two. For WebSocket archival, 1-4
MiB is typical. For in-memory telemetry, 256 KiB is usually
plenty.
