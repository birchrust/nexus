# nexus-slab Documentation

Pre-allocated slab allocator for deterministic latency.

## User Guide

- [Overview](overview.md) — Architecture, when to use which slab type
- [Bounded Slab](bounded.md) — Fixed-capacity, zero-growth allocation
- [Unbounded Slab](unbounded.md) — Chunk-based growth, no copy on resize
- [BoxSlot & RcSlot](smart-handles.md) — RAII and reference-counted slab handles
- [Byte Slab](byte-slab.md) — Type-erased storage for heterogeneous types

## Internal Reference

- [SlotCell](slotcell.md) — Union-based slot design, freelist mechanics
