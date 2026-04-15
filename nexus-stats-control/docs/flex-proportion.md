# FlexProportion

**Types:** `FlexProportion`, `FlexProportionAtomic`
**Import:** `use nexus_stats_control::frequency::FlexProportion;`
**Feature flags:** None required.

## What it does

Tracks an entity's share of a total count over a streaming period. An entity increments its count; another counter tracks total events. Query returns the entity's fraction.

Two variants:

- **`FlexProportion`** — single-owner, `&mut self`.
- **`FlexProportionAtomic`** — thread-safe, atomic counters. Slightly more expensive per update.

Both are O(1), zero allocation.

## When to use it

- **Per-user load share.** "User 42 is 18% of our load right now."
- **Per-client quota tracking** in rate limiters.
- **Market-maker quoting share** on an exchange.

Not for: top-K ranking (use `TopK`). Not for: time-weighted rather than event-weighted shares (use `DecayAccum` + totals).

## API

```rust
impl FlexProportion {
    pub fn new(half_life_events: u64) -> Result<Self, ConfigError>;
    pub fn update(&mut self);
    pub fn count(&self) -> u64;
    pub fn reset(&mut self);
    // fraction / period tracking — see source
}
```

The `half_life_events` parameter controls how old events fade. Pass a large value for near-total counting, smaller for recency-weighted.

## Example — per-client quota share

```rust
use nexus_stats_control::frequency::FlexProportion;

let mut client_a = FlexProportion::new(10_000).unwrap();
let mut client_b = FlexProportion::new(10_000).unwrap();

// Each time client_a makes a request:
client_a.update();

// Each time client_b makes a request:
client_b.update();

// Query counts — fraction = count / total_count
let total = client_a.count() + client_b.count();
if total > 0 {
    let share_a = client_a.count() as f64 / total as f64;
    println!("client A share = {share_a:.3}");
}
```

## Caveats

- **Not a full leaderboard.** You must hold a `FlexProportion` per tracked entity. For dynamic-entity top-K, use `TopK`.
- **Atomic version contends** on the atomic counter under heavy multi-writer load. For single-writer hot paths, use the plain version.
- **Half-life in events**, not time. If you want time-based decay, use `DecayAccum`.

## Cross-references

- [`TopK`](topk.md) — ranked heavy hitters.
- [`DecayAccum`](decay-accum.md) — time-decayed per-entity score.
