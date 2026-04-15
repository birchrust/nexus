# Caveats

Things that will trip you up if you're not watching for them.

## 1. Reset closures must not panic

Covered in depth in [reset-closures.md](./reset-closures.md). If
reset panics:

- The value is leaked.
- The pool slot does not return.
- Over time, the pool exhausts.

Stick to `Vec::clear`, field assignments, and other infallible
operations. No `.unwrap()`, no indexing, no user callbacks.

## 2. Pool drop with outstanding guards

If you drop the pool while `Pooled<T>` guards are still alive:

- Existing guards continue to work normally.
- When each guard drops, it drops its `T` directly instead of
  returning it to the (now-gone) pool.
- No panic, no leak, no use-after-free.

This is safe but operationally surprising: if your shutdown path
drops the pool first and then waits for worker threads to finish,
the worker threads will drop values on the allocator instead of
returning them. If you care (e.g., because drop is expensive), keep
the pool alive until all workers have exited.

Under the hood, guards hold `Weak<Inner>` (local) or `Weak<Inner>`
(sync) — the `Drop` impl upgrades and returns, or falls back to
direct drop.

## 3. Sizing the pool

Under-sizing gives you `None` from `try_acquire` — which means
dropped work, rejected orders, or fallback paths on the hot path.

Over-sizing wastes memory and can push working-set out of cache.

**Rule of thumb:** measure your peak concurrent in-flight object
count in a stress test, then size the pool at 2x that. For a
bursty feed with 300-tick peak bursts, a 1024-entry pool is
comfortable.

**Rule of thumb #2:** `local::Pool::with_capacity(n, ...)` warms
the pool to `n` at startup, avoiding factory calls during the
first burst.

## 4. `BoundedPool` really is bounded

It won't grow. When `available() == 0`, `try_acquire` returns
`None` forever until someone releases. This is the whole point —
you asked for backpressure, you got it.

If you find yourself wishing `BoundedPool` could grow, you want
`local::Pool` instead.

## 5. `sync::Pool` is Send, not Sync

The acquirer thread owns the `Pool` handle. You cannot wrap it
in an `Arc` and clone it across threads — it isn't `Sync`, and
it isn't `Clone`. This is intentional: the whole design enforces
single-acquirer at the type level.

If you need multi-acquirer, you don't actually want a pool — see
[overview.md](./overview.md) "Why no MPMC pool?".

## 6. Pooled<T> is not Copy and (usually) not Clone

You can only have one live handle to a pooled value at a time.
If you need to share read access across multiple consumers,
either:

- Dereference to `&T` and pass the reference around under a
  scoped lifetime.
- Store the guard in an `Rc`/`Arc` if you really need shared
  ownership of the *guard*. The guard's drop still returns the
  value to the pool when the last `Arc` clone drops.

## 7. Reset runs on the returning thread (sync::Pool)

For `sync::Pool`, the reset closure runs on whichever thread
drops the guard — not on the acquirer thread. This is why the
bound is `Fn + Send + Sync` instead of `FnMut`. Make sure your
reset does not touch data that lives on the acquirer thread.

## 8. Factory calls are not on the hot path (mostly)

`BoundedPool::new` and `Pool::with_capacity` call the factory `n`
times at construction. Time this at startup, not during trading
hours. `Pool::acquire` can call the factory on-demand if the pool
is empty — pre-warm with `with_capacity` to avoid hot-path
factory calls.

## 9. Pool objects outlive pool only as long as their guards

If you keep the value alive (via a `Pooled<T>` guard) longer than
the pool, that's fine — the value drops directly on guard drop.
But you cannot "rescue" a value from a dropped pool and
reattach it to a different pool. The guard is one-shot.

## 10. Don't pool huge objects

If a single object is >1 MiB, pooling provides negligible benefit
— the allocator handles large blocks well and the cache-locality
benefit disappears. Pool objects that are small, numerous, and
allocated on the hot path. Order structs, buffers, HashMaps, yes.
Multi-MB decoding tables, no.
