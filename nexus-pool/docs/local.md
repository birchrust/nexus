# Local pools

`local::BoundedPool<T>` and `local::Pool<T>` are single-threaded.
They use `Rc` internally and are neither `Send` nor `Sync`. Use
them when the pool, all acquires, and all returns happen on the
same thread (e.g., inside a single-threaded event loop).

## `BoundedPool` — fixed capacity

```rust
use nexus_pool::local::BoundedPool;

let pool = BoundedPool::new(
    1024,                                // capacity, pre-allocated
    || Vec::<u8>::with_capacity(4096),   // init: called 1024 times now
    |v| v.clear(),                       // reset: called on return
);

assert_eq!(pool.available(), 1024);

let buf = pool.try_acquire().unwrap();
assert_eq!(pool.available(), 1023);

drop(buf); // returned; reset closure runs
assert_eq!(pool.available(), 1024);
```

- `try_acquire()` returns `None` when all objects are checked out.
- `available()` reports how many are currently in the pool.
- No growth: if you configured 1024, you get exactly 1024 objects.

**Use when:** you have a hard upper bound on in-flight objects
and want the pool to reject at that bound instead of growing.

## `Pool` — growable

```rust
use nexus_pool::local::Pool;

let pool = Pool::new(
    || Vec::<u8>::with_capacity(4096),
    |v| v.clear(),
);

// acquire() always succeeds: returns a pooled object or creates one.
let mut a = pool.acquire();
a.extend_from_slice(b"hello");

// try_acquire() is the fast path: None if pool is empty, no allocation.
let b = pool.try_acquire(); // likely None on first call
drop(b);

drop(a); // returned; reset clears the Vec
```

`Pool::with_capacity(n, factory, reset)` pre-populates with `n`
objects, avoiding first-use allocation latency.

**Use when:** you don't know the exact in-flight count and want
the pool to grow once (to the watermark) and then reuse forever.
Call `Pool::with_capacity` to pre-warm it and avoid growth jitter
on the hot path.

## Manual take / put

Only `local::Pool` exposes manual mode:

```rust
use nexus_pool::local::Pool;

let pool = Pool::new(|| vec![0u8; 1024], |v| v.clear());

let mut buf = pool.take();         // owned T, no guard
buf[0] = 42;
pool.put(buf);                     // manual return, reset runs

let maybe = pool.try_take();       // None if empty
if let Some(b) = maybe {
    pool.put(b);
}
```

Use manual mode when you can't let `Pooled<T>` govern the
lifetime — e.g., when storing the buffer in a struct that outlives
the RAII scope, or moving it through a pipeline stage that doesn't
know about the pool.

See [raii-vs-manual.md](./raii-vs-manual.md) for the full
discussion.

## Performance

On a 3.1 GHz x86-64:

| Operation | p50 cycles |
|---|---|
| `BoundedPool::try_acquire` | ~26 |
| `BoundedPool` release (drop guard) | ~26 |
| `Pool::try_acquire` (reuse) | ~26 |
| `Pool::acquire` (factory call) | ~32 + factory cost |

The hot path is two pointer operations (pop from a `Vec`, swap in
the guard's `ManuallyDrop`). There is no atomic, no lock, no
syscall.
