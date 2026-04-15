# Overview

`nexus-pool` provides object pools: a bounded or growable stash of
pre-constructed `T` values that callers borrow, use, and return.
The goal is to eliminate allocation on the hot path by paying the
construction cost once, at startup.

## Pool vs slab vs allocator

| Primitive | What it gives you | Key property |
|---|---|---|
| [`nexus-pool`](../../nexus-pool/) | A LIFO stack of reusable objects | Each acquire returns one object; each release pushes it back |
| [`nexus-slab`](../../nexus-slab/) | Fixed storage with stable keys | You get a key back; lookup by key gives you the object |
| Global allocator | Arbitrary-size allocations | Unbounded, variable latency, fragmentation |

Use **pool** when you have a stream of identical work items (orders,
buffers, message structs) and you only need "give me one" / "I'm
done with this one" semantics.

Use **slab** when you need long-lived objects referenced by key
(open orders, sessions, subscriptions) — you want to look them up
later, not just return them.

Use the **global allocator** when the work is not hot-path or the
lifetime is unknown.

## The three pool types

| Type | Thread model | Capacity | Returns |
|---|---|---|---|
| `local::BoundedPool<T>` | Single thread | Fixed at `new` | `Option<Pooled<T>>` (None if exhausted) |
| `local::Pool<T>` | Single thread | Growable via factory | `Pooled<T>` (always) or `Option<Pooled<T>>` (fast path) |
| `sync::Pool<T>` | One acquirer, any returner | Fixed | `Option<Pooled<T>>` |

All three use LIFO reuse (most-recently-returned goes first),
which is cache-friendly: the object you're about to use was just
touched by the caller that returned it.

## Why no MPMC pool?

**Short answer:** MPMC pools solve the ABA problem with generation
counters, hazard pointers, or epoch reclamation. All three either
add significant overhead on the acquire/release hot path or
require background threads for reclamation. Both outcomes
contradict the purpose of using a pool.

**Architectural answer:** If N threads all acquire from and return
to the same pool, you have N-way contention on a single hot data
structure. The allocator you're trying to replace is *also* a
contention point — you haven't solved anything, you've just moved
the contention. Worse, pool contention is often more painful
because the CAS loop is tighter than a malloc call.

**Better alternatives:**

1. **Per-thread pools.** Each thread owns a `local::Pool`. Work
   items that cross thread boundaries carry an identifier so they
   can be returned to the pool they came from.
2. **Sharded pools.** Hash-based dispatch to one of N pools,
   reducing per-pool contention.
3. **sync::Pool.** One thread acquires, any thread returns. This
   covers the common trading-system pattern of "acquire on the
   hot path, release on a worker."
4. **Message passing.** Send the object (or an index) via
   [`nexus-queue`](../../nexus-queue/) and let the receiver
   return it to the pool on its own thread.

## Construction: `init`, `factory`, `reset`

All pools take two closures:

- **init / factory** — called at pool construction (or on growth
  for the growable `local::Pool`). Produces a `T`.
- **reset** — called every time a `T` is returned to the pool.
  Typically `Vec::clear`, a field zero, or nothing.

The reset closure is the whole trick: you're reusing an object,
not re-creating it, so you need to put it back into a known-good
state. See [reset-closures.md](./reset-closures.md) for the
contract and the panic warning.

## RAII and manual modes

`Pooled<T>` is the RAII guard — when it drops, it returns the
value to the pool (calling reset). This is what you want 95% of
the time.

`local::Pool` additionally exposes `take()` / `put()` for cases
where the RAII lifetime doesn't fit — storing pooled objects in
structs, moving them across pipeline stages, or passing through a
queue. See [raii-vs-manual.md](./raii-vs-manual.md).

## Pool outlives its guards

All `Pooled<T>` guards hold a `Weak` reference to the pool's
inner. If the pool is dropped while guards are still live:

- Existing guards continue to work — they own the `T` via
  `ManuallyDrop`.
- When they drop, they drop the `T` directly instead of returning
  it to the (now-gone) pool.
- No panic, no leak, no use-after-free.

This is important for shutdown paths where drop ordering is
hard to control.
