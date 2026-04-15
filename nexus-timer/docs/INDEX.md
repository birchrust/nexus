# nexus-timer documentation

Hierarchical, no-cascade timer wheel with O(1) insert and cancel. Modeled
on the Linux kernel timer infrastructure (Gleixner 2016).

## Contents

- [overview.md](overview.md) — when a timer wheel is the right tool, and why
  the no-cascade design matters
- [wheel.md](wheel.md) — `Wheel`, `BoundedWheel`, `WheelBuilder`, scheduling
  and cancellation
- [bounded-vs-unbounded.md](bounded-vs-unbounded.md) — the two storage
  backends and their tradeoffs
- [patterns.md](patterns.md) — cookbook: timeouts, heartbeats, periodic
  tasks, deadline scheduling

## Related crates

- [`nexus-slab`](../../nexus-slab) — the underlying slab allocator
- [`nexus-collections`](../../nexus-collections) — includes a simpler
  binary heap if you just need one priority queue
- [`nexus-rt`](../../nexus-rt) — the runtime layer that typically *owns*
  the wheel and exposes it as a resource
