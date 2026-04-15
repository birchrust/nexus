# Reset closures

Every pool takes a `reset` closure that runs when a value is
returned. Its job is to put the object back into a known-good
state so the next caller can use it without surprises.

## Contract

```rust
// local pools:
R: FnMut(&mut T) + 'static

// sync pool:
R: Fn(&mut T) + Send + Sync + 'static
```

The closure:

1. Runs every time a `Pooled<T>` is dropped, or when `put(value)`
   is called on `local::Pool`.
2. Receives `&mut T` — the value is still fully valid; you are
   resetting it, not destroying it.
3. **Must not panic.** See the warning below.

## Typical resets

```rust
use nexus_pool::local::BoundedPool;

// Vec buffer: clear the length, keep the capacity.
let _ = BoundedPool::new(
    1024,
    || Vec::<u8>::with_capacity(4096),
    |v| v.clear(),
);

// HashMap: clear entries, keep the table.
use std::collections::HashMap;
let _ = BoundedPool::new(
    64,
    || HashMap::<u32, u64>::with_capacity(128),
    |m| m.clear(),
);

// Struct: reset fields explicitly.
struct Order { id: u64, qty: u64, price: i64, flags: u32 }
let _ = BoundedPool::new(
    1024,
    || Order { id: 0, qty: 0, price: 0, flags: 0 },
    |o| {
        o.id = 0;
        o.qty = 0;
        o.price = 0;
        o.flags = 0;
    },
);

// No-op: the caller will overwrite every field before reading.
let _ = BoundedPool::<[u8; 64]>::new(
    1024,
    || [0u8; 64],
    |_| {},
);
```

## The panic warning

**If the reset closure panics, the value is leaked and the pool
slot is not returned.** The panic propagates normally out of the
`Drop` for `Pooled<T>` (or the `put` call), which means:

- The object is never returned to the free list.
- `available()` permanently decreases.
- Over time, a panicking reset closure will exhaust the pool.

This is a deliberate choice. The alternative — catching panics —
would mask bugs and leave the pool in an inconsistent state
(reset partially run). The right answer is **don't panic in
reset.**

### What can panic?

Obvious offenders:

- `v[idx]` on a `Vec` or slice (out of bounds).
- `.unwrap()` / `.expect()` on `Option` or `Result`.
- Integer overflow in debug builds.
- `assert!` / `debug_assert!`.
- Calling user code via trait objects or closures.

Safe choices:

- `Vec::clear`, `Vec::truncate(0)`, `HashMap::clear`, etc.
- Plain assignment of primitive fields.
- `u8::fill(0)`, `slice::fill_with(Default::default)`.
- `std::mem::take(&mut field)` where `field: Default`.

### If you really need fallible reset

Don't. Instead:

1. Make the reset infallible and do the fallible work
   at **acquire** time, where you can handle the error.
2. Or store a `Result` inside the pooled object and set an
   error flag during reset; the next acquirer checks the flag
   and either skips or re-initializes.

## Reset is not Drop

It's easy to conflate reset with `Drop`. They're different:

- **Drop** runs when the `T` is actually destroyed (when the pool
  itself drops, or when `Pooled<T>` drops after the pool is gone).
- **Reset** runs when the `T` is being reused — it's a
  zero-argument "make me fresh again" operation.

If your type has a meaningful `Drop` impl (file handles, network
sockets, etc.), you generally **don't** want to pool it. Pooling
is for plain data: buffers, structs, small collections.

## Reset and security

If the pooled object held sensitive data (keys, PII, authentication
tokens), reset must zero it out before the next acquirer sees it.
`Vec::clear` does not zero the backing memory — it only resets the
length. Use `v.iter_mut().for_each(|b| *b = 0)` or a crate like
`zeroize` if this matters.

For market data and order buffers, normal clear is fine — the next
writer will overwrite everything anyway.
