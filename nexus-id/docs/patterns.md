# Patterns

Realistic recipes for common ID needs.

## Client order IDs on a trading hot path

One generator per writer thread. Worker bits encode the thread index (or the
matching engine shard). Sequence bits sized so you never run out within a
millisecond.

```rust
use nexus_id::{Snowflake64, SnowflakeId64};

const WORKER_ID: u32 = 5;

// 42 timestamp bits, 6 worker bits, 16 sequence bits.
// 65,536 orders per ms per worker = 65M orders/sec/worker peak.
pub type OrderGen = Snowflake64<42, 6, 16>;
pub type OrderId = SnowflakeId64<42, 6, 16>;

pub struct OrderIssuer {
    gen: OrderGen,
    epoch_ms: u64,
}

impl OrderIssuer {
    pub fn new(epoch_ms: u64) -> Self {
        Self { gen: OrderGen::new(WORKER_ID), epoch_ms }
    }

    pub fn next(&mut self, now_ms: u64) -> OrderId {
        let rel_ms = now_ms.saturating_sub(self.epoch_ms);
        // Sequence exhaustion at 65k/ms is effectively impossible; crash if it happens.
        self.gen.next_id(rel_ms).expect("order sequence exhausted within 1 ms")
    }
}
```

Store these directly in protocol messages. At the hash-map boundary, use
`MixedId64` + `nohash_hasher` — see [mixed-id.md](mixed-id.md).

## Request trace IDs

Use `UuidV7` for distributed traces: time-sortable makes log aggregation
trivial, and the V7 format is standard enough that OpenTelemetry and
friends understand it out of the box.

```rust
use nexus_id::{UuidV7, Uuid};

pub struct TraceSource {
    gen: UuidV7,
}

impl TraceSource {
    pub fn new() -> Self { Self { gen: UuidV7::from_entropy() } }
    pub fn span(&mut self) -> Uuid { self.gen.generate() }
}
```

When the same log line needs to appear in both structured logs and a
human-facing UI, use `TypeId<8>` with a `trace` prefix:

```rust
use nexus_id::{TypeId, UuidV7};

let uuid = UuidV7::from_entropy().generate();
let compact = u64::from_le_bytes(uuid.as_bytes()[0..8].try_into().unwrap());
let trace: TypeId<8> = TypeId::from_parts("trace", compact).unwrap();
```

## Content hashes

For cache keys or deduplication, you don't want a monotonic ID — you want
a hash of the content. `nexus-id` doesn't hash for you (use `xxhash-rust`,
`ahash`, or `blake3` depending on your needs), but `HexId64` gives you a
cheap string form of the result:

```rust
use nexus_id::HexId64;

let content_hash: u64 = xxhash_rust::xxh3::xxh3_64(payload);
let cache_key = HexId64::from_u64(content_hash);
// cache_key.as_str() is a 16-char hex string, zero-alloc.
```

## HashMap-heavy workloads

If your hot loop is dominated by map lookups keyed by order ID:

```rust
use std::collections::HashMap;
use nohash_hasher::BuildNoHashHasher;
use nexus_id::{MixedId64, SnowflakeId64};

type OrderMap<V> =
    HashMap<MixedId64<42, 6, 16>, V, BuildNoHashHasher<u64>>;

// Key conversion is ~1 cycle — do it at the call site.
fn lookup(map: &OrderMap<Order>, id: SnowflakeId64<42, 6, 16>) -> Option<&Order> {
    map.get(&id.mixed())
}
```

## Public-facing API IDs

Stripe-style `prefix_<base32>` IDs make your API surface self-documenting.
Generate the value from `UuidV7` so resources are still time-sortable in
your database:

```rust
use nexus_id::{TypeId, UuidV7};

pub fn new_order_id(gen: &mut UuidV7) -> TypeId<4> {
    let u = gen.generate();
    let hi = u64::from_be_bytes(u.as_bytes()[0..8].try_into().unwrap());
    TypeId::from_parts("ord", hi).unwrap()
}
```

Users see `ord_01J2MB6KFF2TMXYZ...`. Your index still gets sequential
insertions. Everyone wins.
