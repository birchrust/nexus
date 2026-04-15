# nexus-pool docs

Pre-allocated object pools for hot-path workloads. No allocation
after startup; reset-on-return; RAII or manual return.

## Start here

- [overview.md](./overview.md) — Pool vs slab vs allocator, and why there is no MPMC pool
- [local.md](./local.md) — `local::Pool` / `local::BoundedPool` (single-threaded)
- [sync.md](./sync.md) — `sync::Pool` (one acquirer, any returner)
- [reset-closures.md](./reset-closures.md) — The reset closure contract and panic safety
- [raii-vs-manual.md](./raii-vs-manual.md) — `Pooled<T>` guards vs `take`/`put`
- [patterns.md](./patterns.md) — Order pool, buffer pool, per-strategy state
- [caveats.md](./caveats.md) — Drop ordering, panic warnings, sizing

## TL;DR

```rust
use nexus_pool::local::BoundedPool;

let pool = BoundedPool::new(
    1024,
    || Vec::<u8>::with_capacity(4096),
    |v| v.clear(),
);

if let Some(mut buf) = pool.try_acquire() {
    buf.extend_from_slice(b"payload");
    // Drop returns buf to the pool; reset closure clears it.
}
```

## Related

- [`nexus-slab`](../../nexus-slab/) — fixed-capacity storage with stable keys
- [`nexus-queue`](../../nexus-queue/) — move pool-allocated items between threads by index
