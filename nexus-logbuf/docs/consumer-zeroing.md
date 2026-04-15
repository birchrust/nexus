# Consumer zeroing

Why the consumer must zero record bytes before advancing `head`,
and why this is not wasted work.

## The invariant

In logbuf, the `len` field of each record is the **commit
marker**:

- `len == 0` → "not yet committed, consumer waits here"
- `len > 0` (high bit clear) → "committed record, payload is
  `len` bytes"
- `len` high bit set → "skip marker, advance `len & LEN_MASK`
  bytes"

The producer commits a record by storing a non-zero `len`. For
this to work, the slot **must have `len == 0` before** the
producer writes into it. Otherwise, a stale committed record
from a prior lap would look already-committed to the consumer on
a new lap.

So: the consumer zeros the region before advancing. The producer
then sees `len == 0` when it reclaims the slot and can stamp a
fresh commit.

## Why not generation counters?

A common alternative is per-slot generation counters or lap
numbers (like `nexus-queue` uses). logbuf does not, because:

1. **Variable-length records don't have fixed slots.** A record
   is `header + payload + padding`, where payload length varies.
   Per-slot metadata is awkward when slots overlap and merge.
2. **The producer already writes the header.** Reusing `len` as
   the commit marker costs zero extra space and one extra
   release store.
3. **Consumer zeroing is effectively free.** On a modern x86
   CPU, zeroing 128 bytes is a handful of `rep stos` micros or
   AVX2 `vmovdqu` writes. Compared to the cost of reading the
   record, zeroing is noise.

## Why not triple buffering?

Triple buffering (writer → middle → reader) eliminates the need
for commit markers but doubles or triples the buffer's memory
footprint. It also limits you to fixed-size records, which
defeats the whole point of logbuf.

## The Aeron precedent

This pattern comes directly from Aeron's Log Buffer design (see
Martin Thompson's Aeron papers). Aeron uses the same approach:

- Records have a header with a length field.
- The length is written atomically as the commit marker.
- The consumer zeros records before releasing space.
- Skip markers handle wrap and abort.

Aeron has proven the pattern at exchange-scale (millions of
messages per second, microsecond-class latency) over many years.
logbuf is a direct port of the core mechanism into an
SPSC/MPSC Rust primitive.

## Performance cost

Measured on a 3.1 GHz Intel Core i9 with 128-byte records:

- Producer `try_claim + copy + commit`: ~40 cycles p50
- Consumer `try_claim + deref + drop (zero + advance)`: ~55 cycles p50

Zeroing a 128-byte record adds ~15 cycles on the consumer side
— a cost the consumer pays, not the producer. Because the
consumer is usually off the hot path (archival thread, logger
thread), this is the correct trade-off: shift work from the
latency-critical producer to the throughput-oriented consumer.

## Does zeroing kill cache?

Concern: zeroing a record "dirties" the cache line, which might
hurt throughput.

Answer: the cache line is already dirty — the consumer just
read the record from it, and before that, the producer wrote
into it. Zeroing is an additional store to the same cache line
the consumer just touched. It's essentially free in terms of
cache pressure.

## What about security?

Zeroing is a nice side effect: previously-committed records are
erased as soon as they're consumed. If your buffer carries
sensitive data (API keys, session tokens, personal data), the
producer's payload only exists in the buffer from commit until
consume, which is typically microseconds. After consume, it's
zero. This is not a security guarantee — for that, use explicit
crypto and zeroize — but it's a better starting point than
queues that leave stale data lying around.
