# Overview

`nexus-queue` is three bounded ring buffers sharing a common design:
a power-of-two-sized slot array, a producer index, a consumer index,
and per-slot lap counters that synchronize writer and reader without
taking locks.

## Why not `crossbeam::ArrayQueue`?

`crossbeam::ArrayQueue` is MPMC — every push and pop pays for full
multi-writer/multi-reader coordination. When your actual topology is
SPSC (one feed handler → one matching engine) or SPMC (one feed
handler → N strategy threads), you can do 2-3x better by paying only
for the synchronization you actually need.

`nexus-queue` picks the cheapest primitives for each topology:

| Variant | Producer side | Consumer side |
|---|---|---|
| SPSC | cached local index, one atomic store on wrap | cached local index, one atomic store on wrap |
| MPSC | CAS on tail, wait for slot turn | one atomic load + store |
| SPMC | one atomic load + store | CAS on head, wait for slot turn |

## Lap counter mechanism

Each slot holds a `(value, sequence)` pair. `sequence` is the turn
counter: it starts at the slot index and advances by `capacity` each
time the slot is reused.

- Producer waits for `sequence == tail` before writing; on publish,
  it stamps `sequence = tail + 1`.
- Consumer waits for `sequence == head + 1` before reading; on
  consume, it stamps `sequence = head + capacity`.

This is Dmitry Vyukov's bounded MPMC trick, specialized per
topology. The key property: a reader never observes a half-written
slot, and a writer never overwrites a slot the reader hasn't
finished with. No seqlock retries, no ABA, no generation counters.

## Capacity

All constructors (`spsc::ring_buffer`, `mpsc::ring_buffer`,
`spmc::ring_buffer`) round the requested capacity up to the next
power of two. The mask is `capacity - 1`. This means:

- Index wrap-around is a single `&` with the mask — no branch, no
  division.
- Asking for `1000` gives you `1024`. Asking for `1025` gives you
  `2048`.
- Zero capacity panics. Capacities near `usize::MAX` panic (overflow
  in `checked_next_power_of_two`).

## Ownership of `T`

The queue owns values between `push` and `pop`. On a failed
`push(value)`, the value is returned inside `Full<T>` — nothing is
ever leaked or dropped accidentally. On queue drop, any remaining
values in the buffer are dropped.

## When to use which

**SPSC**: default. Every time you have exactly one writer and one
reader, use SPSC. Examples: feed handler → parser, parser → matching
engine, worker → sink.

**MPSC**: multiple threads funnel work into one consumer. Examples:
log aggregation from worker threads, telemetry collection, order
entry from multiple client sessions into one gateway.

**SPMC**: one producer fans out work to a pool of consumers, and
**each item should be handled exactly once**. Example: a single IO
thread parses frames and pushes work items that any idle worker can
pick up. This is *not* for broadcasting the same value to N
consumers — every item is consumed once.

If you need broadcast semantics, that's a different primitive (not
in this crate).

## Disconnect detection

Each side exposes `is_disconnected()` — true if the other half has
been dropped. Both `push` and `pop` continue to work as long as
there's data in the queue; disconnection is advisory, letting you
tear down cleanly. For SPSC with blocking semantics, layer
`nexus-channel` on top.
