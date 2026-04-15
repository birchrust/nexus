# RAII vs manual return

Pools offer two return patterns. Choose based on how the object's
lifetime fits into your code.

## RAII: `Pooled<T>` guards

The default. `acquire` / `try_acquire` return a `Pooled<T>` guard
that derefs to `T` and returns the value to the pool on drop.

```rust
use nexus_pool::local::Pool;

let pool = Pool::new(|| Vec::<u8>::with_capacity(1024), |v| v.clear());

{
    let mut buf = pool.acquire();     // Pooled<Vec<u8>>
    buf.extend_from_slice(b"work");
    process(&buf);
} // <- buf drops here; reset runs; slot returns to pool

fn process(_: &[u8]) {}
```

**Use RAII when:**

- The object's useful life is a single lexical scope.
- You want drop ordering to match acquire ordering naturally.
- You want the compiler to prove you can't forget to return it.

Available on all three pool types: `BoundedPool::try_acquire`,
`Pool::acquire` / `Pool::try_acquire`, `sync::Pool::try_acquire`.

## Manual: `take` and `put`

Only `local::Pool` (the growable local pool) exposes this.

```rust
use nexus_pool::local::Pool;

let pool = Pool::new(|| Vec::<u8>::with_capacity(1024), |v| v.clear());

let mut buf: Vec<u8> = pool.take();        // plain T, no guard
buf.extend_from_slice(b"hello");

// Store buf in a struct, move it across a pipeline stage, or send
// it through a queue by value — the pool does not care.

pool.put(buf); // explicit return; reset runs
```

Fast path: `try_take()` returns `Option<T>` without running the
factory.

**Use manual when:**

- The object needs to live inside a struct whose lifetime exceeds
  the acquire scope.
- The object passes through a queue, a callback, or a state
  machine that doesn't know about the pool.
- You want to conditionally return the object (e.g., discard if
  corrupted, otherwise return).

## Why no manual mode for `BoundedPool` or `sync::Pool`?

**`BoundedPool`**: the RAII guard is part of the safety story —
without it, you'd need to track indices manually, and the pool
has no way to verify that a `put(value)` corresponds to a prior
`take`. The bounded case is simple enough that RAII is always
sufficient.

**`sync::Pool`**: values returned from an arbitrary thread must
be tied to a specific slot index (the pool is slot-based, not
stack-based). The `Pooled<T>` guard carries that index. Without
the guard, there's no way to know which slot to return to.

If you need manual-style release for `sync::Pool`, store the
guard itself (perhaps inside your work-item struct) and drop it
when done:

```rust
use nexus_pool::sync::Pool;

struct WorkItem {
    buf: Option<nexus_pool::sync::Pooled<Vec<u8>>>,
    // other fields
}

let pool = Pool::new(1024, || vec![0u8; 4096], |v| v.clear());
let buf = pool.try_acquire().unwrap();
let mut item = WorkItem { buf: Some(buf) };

// ...pass item around; when you're done:
item.buf = None; // drops the guard, returns to pool
```

## Decision tree

```text
                    Do you need to return
                    from a different thread?
                           |
              ┌────────────┴────────────┐
              no                         yes
              |                          |
    Does the lifetime fit           sync::Pool
    in one lexical scope?           (RAII only)
              |
      ┌───────┴────────┐
      yes               no
      |                 |
  RAII on         local::Pool
  any local          take / put
  pool
```
