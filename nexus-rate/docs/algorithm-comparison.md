# Algorithm comparison

GCRA vs Token Bucket vs Sliding Window, side by side.

## Summary

| Property | GCRA | Token Bucket | Sliding Window |
|---|---|---|---|
| State size | 1 × u64 | 1 × u64 | N × u64 (per bucket) + counters |
| Hot-path cycles (local) | ~5-10 | ~8-12 | ~15-30 (scales with sub_windows) |
| Sync variant | yes | yes | no |
| Smooth rate | yes | yes | no (boundary burst) |
| Exact count in window | no | no | yes |
| Burst spec | `burst` = extra allowance | `burst` = bucket capacity | implicit via sub-windows |
| Rebate floor | can't bank past `now` | capped at `burst` | can't go below zero |
| Exchange spec match | best for "rate + burst" | best for "bucket of N" | best for "N per rolling window" |

## Decision tree

```text
How does the rate limit spec phrase itself?
│
├─ "X requests per Y, with burst" ──────────────────▶ GCRA
│
├─ "Bucket of X tokens, refill at Y/s" ─────────────▶ Token Bucket
│
├─ "At most X in any rolling window of Y" ──────────▶ Sliding Window
│
└─ "Just don't exceed X/s" (no burst mentioned) ────▶ GCRA (burst=0)

Do you need multi-threaded sharing?
│
├─ No, single owner ────────────────────────────────▶ local::*
├─ Yes, small number of threads ────────────────────▶ sync::Gcra / sync::TokenBucket
└─ Yes, sliding window required ────────────────────▶ Mutex<local::SlidingWindow>
                                                       (or reconsider — do you really need sliding?)

Do you need rebate on some outcomes?
│
└─ Yes ────────────────────────────────────────────▶ any algorithm supports release()

If still unsure:
    Default to local::Gcra. It's the fastest, handles smooth rates
    and bursts, and has no pathological cases.
```

## Burst behavior comparison

Given `rate = 10/sec, burst = 5`:

### GCRA
- Start: idle for 10s → full burst available.
- Fire 6 requests back-to-back: all pass (burst + 1 steady).
- Fire 7th request immediately: rejected.
- Wait 100ms: one more request passes.
- Steady state: exactly 10/sec, spaced ~100ms apart.

### Token Bucket (burst=6 to match)
- Same behavior. The math is equivalent to GCRA.
- Configuration differs: `.burst(6)` vs GCRA's `.burst(5)`.

### Sliding Window (window=1s, sub_windows=10, limit=10)
- Fire 10 requests in the first 100ms: all pass.
- 11th request immediately: rejected.
- At boundary crossover (1.0s mark): the oldest 100ms bucket
  drops out, allowing up to 10 more requests in the next
  100ms. **You can fire 20 requests in a 200ms span** at the
  window boundary.
- This is the "2x boundary burst" — a property of any
  non-smooth window algorithm.

With sub_windows=60, the boundary burst drops to ~10 * (1 +
1/60) ≈ 10.17 → much tighter, but more computation.

## Which matches which exchange?

| Exchange | Usual doc phrasing | Match |
|---|---|---|
| CME iLink | "N new orders per second per session" | GCRA, burst=small |
| Binance REST | "weight per minute, bucket of M" | Token Bucket |
| Binance WebSocket | "N messages per second" | GCRA |
| Kraken | "counter with decay rate" | Token Bucket |
| FTX/OKX | "N requests per second, hard cap" | Sliding Window or GCRA |
| Custom internal | whatever you want | GCRA unless there's a reason |

When in doubt, re-read the exchange's rate limit docs **very
carefully** — small details change which algorithm matches.
Some exchanges mix models (e.g., "rate per second AND count per
minute"), in which case you run multiple limiters and AND their
results.

## "I want the smoothest possible enforcement"

GCRA. There is no smoother algorithm than "one event every
`emission_interval` units of time". GCRA gives you exactly
that, plus a configurable burst at the start.

## "I want the simplest mental model"

Token Bucket. "I have N tokens, I spend one per request, they
refill at a rate." That's it.

## "I want exact enforcement with no tricks"

Sliding Window with `sub_windows = 60` or higher. You get
exact count within a rolling window, at the cost of a small
boundary burst (1/60 = 1.67% over the limit at worst).

## Performance summary

All three are O(1) per check. The cycle counts below are for
`local::*` on a 3.1 GHz Intel Core i9.

| Algorithm | p50 cycles | State bytes |
|---|---|---|
| GCRA | 5-10 | 32 |
| Token Bucket | 8-12 | 32 |
| Sliding Window (10 sub) | 15-30 | ~100 |
| Sliding Window (60 sub) | 40-80 | ~500 |

Sync variants add ~15-30 cycles for the CAS loop under light
contention. Under heavy contention (many threads hammering
one limiter), the tail can spike significantly — consider
sharding instead.

## The default choice

Pick `local::Gcra` unless you have a specific reason not to.
It's the fastest, the most compact, the most flexible, and
maps to every smooth rate limit out there. Only reach for the
others when the exchange spec or your application logic
forces your hand.
