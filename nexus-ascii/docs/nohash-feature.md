# The `nohash` feature

Identity hashing turns hash-map lookups into single-load operations by using
the key's bits directly as the bucket index. This is unsound for general
keys (bad distribution, trivial collisions), but `AsciiString` already
stores a precomputed 48-bit XXH3 hash in its header — we know the bits are
well-distributed. Feeding them to the hasher directly is the fastest
possible path.

## Enabling

```toml
[dependencies]
nexus-ascii = { version = "1", features = ["nohash"] }
```

This pulls in `nohash_hasher` and wires up `IsEnabled` impls for
`AsciiString<CAP>` and `AsciiText<CAP>`, plus the `AsciiHashMap` and
`AsciiHashSet` aliases.

## Usage

```rust
use nexus_ascii::{AsciiString32, AsciiHashMap};

let mut prices: AsciiHashMap<32, f64> = AsciiHashMap::default();

let btc: AsciiString32 = AsciiString32::try_from("BTC-USD").unwrap();
let eth: AsciiString32 = AsciiString32::try_from("ETH-USD").unwrap();

prices.insert(btc, 45_000.0);
prices.insert(eth, 2_500.0);

assert_eq!(prices.get(&btc), Some(&45_000.0));
```

The lookup cost is:

1. Load the 48-bit hash from the key's header (one cache load).
2. Mask to bucket index (one AND).
3. Walk the bucket's linked list until a header-word compare matches.
4. `memcmp` the content for the final winner.

No hashing at lookup time. No bytes touched unless there's a header collision.
For workloads dominated by hot-map reads — order books by symbol, client
state by session ID, quote cache by instrument — this is the largest single
win `nexus-ascii` gives you.

## `AsciiHashSet`

Same deal for sets of symbols or IDs:

```rust
use nexus_ascii::{AsciiString32, AsciiHashSet};

let mut tradeable: AsciiHashSet<32> = AsciiHashSet::default();
tradeable.insert(AsciiString32::try_from("BTC-USD").unwrap());
tradeable.insert(AsciiString32::try_from("ETH-USD").unwrap());

let probe = AsciiString32::try_from("BTC-USD").unwrap();
assert!(tradeable.contains(&probe));
```

## Is the 48-bit hash enough?

Yes, for non-adversarial inputs. 2⁴⁸ ≈ 2.8 × 10¹⁴. Birthday-collision
probability on 10,000 distinct keys is ~1.7 × 10⁻⁷. Production trading
systems typically have a few hundred to a few thousand instruments —
collisions are effectively impossible in practice.

If your workload is adversarial (public API, untrusted input), use a real
hasher instead (`FxHashMap<AsciiString32, V>` or `HashMap` with SipHash).
You still get the fast header comparison on equality checks; you just pay
for real hashing on lookups.

## What about `AsciiText`?

Same story — `AsciiTextHashMap<CAP, V>` and `AsciiTextHashSet<CAP>` are
available when the `nohash` feature is on.

```rust
use nexus_ascii::{AsciiText32, AsciiTextHashMap};

let mut names: AsciiTextHashMap<32, u64> = AsciiTextHashMap::default();
names.insert(AsciiText32::try_from("Alice").unwrap(), 1);
```
