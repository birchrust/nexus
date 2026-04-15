# Overview

A timer wheel is a data structure for scheduling large numbers of timers
that mostly get cancelled before they fire. This is the dominant pattern in
network software (request timeouts, keepalives, retransmits) and trading
systems (order TTLs, heartbeats, stale-data deadlines).

## When to use a timer wheel

Use `nexus-timer` when:

- You schedule thousands of timers per second.
- Most timers are cancelled before firing (TCP retransmit is the canonical
  example — typically cancelled by the ACK).
- You want O(1) insert and cancel.
- You don't need precise firing time — "within one tick of the deadline"
  is fine.

Use a binary heap (e.g. `nexus_collections::Heap`) when:

- You have tens, not thousands, of timers.
- You want the next-deadline query to be O(1) rather than O(active slots).
- You need exact ordering by deadline.

Use `std::thread::sleep` or `tokio::time::sleep` when:

- You're scheduling a handful of one-shot delays.
- Timer overhead is not in your flame graph.

## The no-cascade design

Traditional hierarchical timer wheels (Linux pre-2016, tokio's
`time::driver`) *cascade*: when a slot in a higher level expires, the
entries inside it are walked and re-inserted into lower levels. This keeps
the representation tight, but produces latency spikes — poll becomes
proportional to the number of cascading entries, not just the number of
firing entries.

`nexus-timer` doesn't cascade. An entry is placed in a slot based on how
far in the future it fires, and it *stays there* until poll visits the
slot. When poll visits a slot, it walks the entries and checks each one's
exact deadline:

```text
poll(now):
    for each active level:
        for each active slot in level:
            for each entry in slot:
                if entry.deadline <= now:
                    fire(entry)
```

The cost is that slots in higher levels hold entries that fire across a
range of ticks — poll has to check deadlines rather than firing whole slots
blindly. In exchange, there are no cascading latency spikes, and the worst
case is bounded by `active_slots × entries_per_slot`, not the total
population.

For most workloads this is a much better tradeoff. The entries that
*actually fire* are always near the current wheel position; entries in
distant slots get cancelled before poll ever visits them.

## Default configuration

`Wheel::unbounded(chunk_capacity, now)` gives you the Linux-kernel default:

- 1 ms tick
- 64 slots per level
- 8× multiplier per level (`clk_shift = 3`)
- 7 levels
- Total range: ~4.7 hours

Customize via `WheelBuilder` if you need sub-ms resolution or a longer
range. See [wheel.md](wheel.md).
