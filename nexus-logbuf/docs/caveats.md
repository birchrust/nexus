# Caveats

## 1. `mem::forget` deadlocks the buffer

Covered in detail in [claim-api.md](./claim-api.md). **Do not
leak `WriteClaim` or `ReadClaim` via `mem::forget`, `ManuallyDrop`,
or any other mechanism that skips the destructor.**

Consequences:

- Forgotten `WriteClaim`: the slot is reserved but never
  committed (len stays zero). The consumer blocks at this slot
  forever. All future producers eventually fill the buffer and
  return `Full` permanently.
- Forgotten `ReadClaim`: the consumer never advances `head`.
  Subsequent reads see the same record. The producer fills the
  buffer and returns `Full`.

There is no recovery. This is not undefined behavior, but it's
an unrecoverable logical deadlock.

**Normal drop handles everything correctly**, including drops
triggered by panic unwinding. The only way to hit this is to
actively call `mem::forget` or use unsafe tricks — which is
always a smell, and here it's a bug.

## 2. Capacity is in bytes, not records

`spsc::new(64 * 1024)` gives you a 64 KiB buffer, not a
64-record buffer. Size for peak burst in bytes:

- Measure your worst observed second of traffic in bytes.
- Multiply by at least 2x (consumer lag budget).
- Round up to the next power of two.

For WebSocket feeds, 1-4 MiB is typical. For in-process
telemetry, 256 KiB is usually plenty.

## 3. Zero-length payloads are rejected

`try_claim(0)` returns `Err(TryClaimError::ZeroLength)`. This is
because `len == 0` is the "not yet committed" sentinel in the
record header; allowing zero-length records would break the
commit protocol. If you need a "marker" record, write at least
one byte.

## 4. Skip markers waste space near buffer wrap

When a claim would cross the end of the buffer, the producer
writes a skip marker for the remaining space and advances to
the beginning. This is transparent to the caller but means:

- A 4 KiB record near the wrap boundary of a 16 KiB buffer can
  produce up to 4 KiB of skip padding.
- Worst case: ~50% space efficiency for records that are half
  the buffer size.

For typical configurations (records << buffer size), skip
overhead is well under 1%. If you see significant skip overhead,
your buffer is sized wrong (too small) — bump it up.

## 5. Record alignment is 8 bytes

Each record's start offset is aligned to 8 bytes so the `len`
field is word-aligned for atomic access. This means:

- A 7-byte payload plus 8-byte header takes 16 bytes on the wire.
- A 13-byte payload plus 8-byte header takes 24 bytes.

Most workloads don't care about this. If you're packing tiny
records (<8 bytes each), you'll see ~50% overhead and should
probably use a typed [`nexus-queue`](../../nexus-queue/)
instead.

## 6. MPSC: one slow producer stalls the consumer

MPSC commits can arrive out of `tail` order. The consumer reads
in `head` order, so if producer A claims first and commits last,
the consumer waits on A's slot even if B has already committed.

**Never hold a `WriteClaim` across:**
- yield points
- blocking I/O
- long computations
- lock acquisitions

Claim, write, commit, all within a few microseconds. If you
can't, use SPSC per producer and merge.

## 7. No priorities, no ordering guarantees beyond FIFO

logbuf is strictly FIFO by `tail` order. There is no priority
queue, no out-of-band channel, no "urgent" marker. If you need
priority, run two logbufs (urgent + normal) and poll urgent
first.

## 8. Consumer drop does not disconnect producer (raw queue)

In the raw `queue::spsc` / `queue::mpsc` API, there is no
built-in disconnect tracking. If you need producer-side
`SendError::Disconnected`, use the [`channel`](./channels.md)
wrapper. The raw API assumes both sides live for the duration
of the program, which is the common case for archival and
journaling loops.

## 9. The consumer zeros. That's O(n) in record size.

The consumer pays `O(len)` work to zero each record before
advancing. For tiny records (~64 bytes) this is negligible; for
large records (>64 KiB) it can dominate consumer cost. If your
records are huge and you trust the next producer to fully
overwrite, you might want a different primitive — talk to me
about it first.

## 10. Don't share a `Consumer` across threads

`Consumer` is `Send` but not `Sync`. Only one thread at a time
may consume. If you need fan-out consumption, either:

- Duplicate the producer side and feed two logbufs.
- Have the single consumer forward records via
  [`nexus-queue::spmc`](../../nexus-queue/) to a worker pool.
