# Overview

`nexus-logbuf` is a byte ring buffer. Where [`nexus-queue`](../../nexus-queue/)
moves `T` values with a fixed shape, logbuf moves **raw variable-length
byte records**. You claim `n` bytes of space, write into it, and
commit — the consumer reads the record as a `&[u8]`.

## When to use logbuf vs nexus-queue

**Use `nexus-queue` when:**
- You have a fixed-size struct `T` and want to move it between threads.
- You want `Option<T>` / `Result<(), Full<T>>` semantics.
- Your payload is small (<64 bytes) and known at compile time.

**Use `nexus-logbuf` when:**
- Records are variable-length (WebSocket frames, FIX messages, log lines).
- The raw bytes *are* the canonical form (you already have them on the wire).
- You're archiving, journaling, or shipping data off the hot path to a
  different subsystem that will parse it later.
- You want one producer buffer that serves many record types (WebSocket
  text + binary + control frames, all in one stream).

## Core design

- **Flat byte buffer** with power-of-two capacity. Free-running
  `head` and `tail` offsets, wrap via mask.
- **Per-record header** containing the record length. The length
  is written **last** via an atomic store — this is the commit
  marker. Until len is non-zero, the consumer waits.
- **Skip markers**: the high bit of the length field marks a
  "skip this region" entry, used for padding past the end of the
  buffer and for aborted claims.
- **Consumer zeroing**: after consuming a record, the consumer
  zeros the header and payload before advancing the read head.
  This is required because len==0 is "not ready yet", so we must
  restore that invariant before the slot is reused by a future
  producer.

## Claim-based API

Producing a record is a three-step dance:

1. **Claim** space: `try_claim(len) -> Result<WriteClaim, ...>`.
   The producer advances `tail` to reserve the region.
2. **Write** the payload: `WriteClaim` derefs to `&mut [u8]`.
3. **Commit**: `claim.commit()` writes the len field atomically
   with release ordering, making the record visible.

The `WriteClaim` is RAII: if you drop it without calling
`commit`, it writes a skip marker so the consumer can advance
past the dead region. This keeps the buffer usable even when
producer code panics mid-write.

On the consumer side: `try_claim()` returns a `ReadClaim` that
derefs to `&[u8]`. Dropping the `ReadClaim` zeros the region and
advances the read head.

## Why variable-length matters

A WebSocket session sees frames from 2 bytes ("ping") to 64+ KiB
("trade batch"). Typed queues force you to either:

- Allocate a `Vec<u8>` per frame (kills the hot path).
- Fix a max size and waste memory on short frames.
- Use multiple queues by size class (operational complexity).

logbuf handles all of these naturally: write 2 bytes, next writer
writes 8000 bytes, next writer writes 12 bytes. The consumer sees
exactly what was written.

## Two variants

| Variant | Module | Producer model |
|---|---|---|
| SPSC | `queue::spsc` | Single producer, single consumer. ~40 cycle p50 claim. |
| MPSC | `queue::mpsc` | Multi-producer via CAS-based claim. ~95 cycle p50 under contention. |

Both share the same record format and consumer interface — you
could swap SPSC for MPSC without changing consumer code.

## Channels (blocking wrapper)

`channel::spsc` and `channel::mpsc` wrap the raw queue with
backoff and parking for the consumer:

- Producer spins briefly on `Full`, never syscalls.
- Consumer parks on empty with optional timeout.
- Disconnection is detected and returned as an error.

Use channels when the consumer is a background thread that should
sleep when idle. Use the raw `queue` API when you're polling from
a hot loop and want every cycle of control.

## Performance

| Variant | p50 | Throughput |
|---|---|---|
| SPSC queue | ~40 cycles | 20.7 GB/s |
| MPSC queue (2 producers) | ~95 cycles | 7.27 GB/s |
| MPSC queue (4 producers) | ~105 cycles | 6.29 GB/s |

Measured with 128-byte records on a 3.1 GHz Intel Core i9 with
turbo disabled.
