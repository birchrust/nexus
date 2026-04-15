# nexus-logbuf docs

Variable-length byte ring buffer. SPSC or MPSC. Claim-based API
with RAII commit. Designed for getting data off the hot path
without allocation, formatting, or syscalls.

## Start here

- [overview.md](./overview.md) — When to use logbuf vs nexus-queue
- [spsc.md](./spsc.md) — SPSC byte ring (the fast path)
- [mpsc.md](./mpsc.md) — MPSC byte ring
- [claim-api.md](./claim-api.md) — `WriteClaim` / `ReadClaim`, commit, **mem::forget deadlock warning**
- [consumer-zeroing.md](./consumer-zeroing.md) — Why the consumer zeros, Aeron pattern
- [channels.md](./channels.md) — Blocking wrappers with backoff + park
- [patterns.md](./patterns.md) — WebSocket archival, event sourcing, FIX journaling
- [caveats.md](./caveats.md) — mem::forget, sizing, skip markers

## TL;DR

```rust
use nexus_logbuf::queue::spsc;

let (mut tx, mut rx) = spsc::new(64 * 1024);

// Producer (hot path): claim, fill, commit.
let payload = b"tick: BTC 52100.50";
if let Ok(mut claim) = tx.try_claim(payload.len()) {
    claim.copy_from_slice(payload);
    claim.commit();
}

// Consumer (background): read, drop.
if let Some(record) = rx.try_claim() {
    // consume(&*record);
    // drop zeros the record and advances the read head.
}
```

## Related

- [`nexus-queue`](../../nexus-queue/) — typed variant, `T`-oriented
- [`nexus-channel`](../../nexus-channel/) — blocking SPSC of typed values
- [`nexus-net`](../../nexus-net/) — often uses logbuf for WebSocket frame archival
