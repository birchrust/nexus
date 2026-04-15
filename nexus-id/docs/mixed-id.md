# MixedId64 — hash-map-friendly integer IDs

Snowflake IDs have terrible bit distribution for power-of-2 hash tables. The
low bits cycle through the sequence counter, the high bits are dominated by
the timestamp, and the middle bits (worker) rarely change. `std::collections::HashMap`
hides this by running SipHash on every lookup — which costs real cycles.

`MixedId64` fixes the distribution directly. It applies a Fibonacci multiply
(the golden ratio trick) to scatter a Snowflake ID across the full u64 range,
making it safe to use with identity hashers like `nohash_hasher`.

## The Fibonacci mix

```rust
use nexus_id::{Snowflake64, SnowflakeId64, MixedId64};

let mut gen: Snowflake64<42, 6, 16> = Snowflake64::new(5);
let id: SnowflakeId64<42, 6, 16> = gen.next_id(now_millis()).unwrap();

// One multiply (~1 cycle)
let mixed: MixedId64<42, 6, 16> = id.mixed();

// Reversible — same layout parameters round-trip cleanly
let recovered: SnowflakeId64<42, 6, 16> = mixed.unmix();
assert_eq!(recovered, id);
```

The mix is a bijection: multiplication by the golden ratio constant (mod 2⁶⁴),
which has a modular inverse. No collisions introduced, no information lost.

## Using with nohash_hasher

```rust
use std::collections::HashMap;
use nohash_hasher::BuildNoHashHasher;
use nexus_id::MixedId64;

type MixedHasher = BuildNoHashHasher<u64>;
type OrderMap<V> = HashMap<MixedId64<42, 6, 16>, V, MixedHasher>;

let mut orders: OrderMap<Order> = OrderMap::default();
orders.insert(id.mixed(), order);
```

`nohash_hasher` uses the key's bits *directly* as the hash. With a raw
Snowflake that would be a disaster (clustering, long probe chains); with
`MixedId64` the golden-ratio scatter gives you near-perfect distribution
for free.

Total lookup cost: pointer chase to the bucket, one integer compare. No
hashing, no SipHash rounds, no branches worth naming.

## When to use which

| Situation | Recommendation |
|-----------|---------------|
| Logging, serialization, wire format | Raw `SnowflakeId64` |
| `HashMap` / `HashSet` keys on the hot path | `MixedId64` + `nohash_hasher` |
| You can tolerate a real hasher | `SnowflakeId64` + `FxHashMap` or `AHashMap` |

Don't use `MixedId64` as your canonical storage type. Store `SnowflakeId64`
(or the raw `u64`) and mix on the way into the hash map. The mix is a
display-layer concern for the hash table, not a new identity.

## Why not just use `FxHashMap`?

`FxHashMap` (and `AHashMap`) hash u64 in 2–5 cycles. `MixedId64` + nohash is
~1 cycle. If you have millions of lookups per second on the hot path, it
adds up. If you don't, the convenience of `FxHashMap` wins. Measure first.
