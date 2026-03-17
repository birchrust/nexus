# Overview

nexus-slab provides pre-allocated storage where objects live in fixed
slots and are accessed via lightweight handles. No allocation after init.
No fragmentation. O(1) insert and remove.

## Why a Slab?

General-purpose allocators (`Box`, `malloc`) are designed for variable-size,
variable-lifetime allocations. They're good at being general. They're bad
at being predictable — allocation time varies, fragmentation accumulates,
and cache locality degrades over time.

Slabs solve this for the common case: **many objects of the same type,
created and destroyed frequently.** The allocator pre-allocates all slots
at init. Allocation is a freelist pop. Deallocation is a freelist push.
Both are O(1), deterministic, and cache-friendly (LIFO reuse means you
get back the slot you just freed — still hot in L1).

## Architecture

```
┌─────────────────────────────────────────────┐
│              User Code                       │
│                                              │
│  let slot = Allocator::try_alloc(value)?;    │
│  // use *slot                                │
│  unsafe { Allocator::free(slot); }           │
└──────────────┬──────────────────────────┬────┘
               │                          │
       ┌───────▼────────┐        ┌────────▼────────┐
       │  Macro-Generated │        │   Direct API     │
       │  TLS Allocator   │        │   (bounded::Slab, │
       │  (zero-sized,    │        │    unbounded::Slab)│
       │   thread-local)  │        │                   │
       └───────┬──────────┘        └────────┬──────────┘
               │                            │
               ▼                            ▼
       ┌──────────────────────────────────────────┐
       │            SlotCell<T> Array              │
       │                                           │
       │  [SlotCell] [SlotCell] [SlotCell] [...]    │
       │   value      next_free  value     next_free│
       │   (occupied) (vacant)   (occupied) (vacant)│
       └──────────────────────────────────────────┘
```

## Two Slab Types

| Type | Capacity | Growth | Use Case |
|------|----------|--------|----------|
| `bounded::Slab` | Fixed at init | Never | Known upper bound, zero surprise |
| `unbounded::Slab` | Grows in chunks | New chunk allocated | Unknown count, no copy on growth |

**Bounded** — you know the maximum count. The slab allocates once and
never grows. `try_alloc` returns `Err(Full)` at capacity. Zero
allocation jitter. Ideal for hot paths with known population.

**Unbounded** — you don't know the count upfront. The slab starts with
one chunk and adds more as needed. Each chunk is independent — no copying,
no reallocation of existing slots. Existing handles remain valid across
growth. A small latency spike (~40 cycles) on the allocation that triggers
growth; all other allocations are ~20 cycles.

## Two Usage Patterns

### Direct API — `bounded::Slab` / `unbounded::Slab`

Create a slab, allocate into it, free from it. You own the slab.

```rust
use nexus_slab::bounded;

let slab = bounded::Slab::<String>::with_capacity(1024);
let slot = slab.alloc("hello".to_string());

// Use the value
println!("{}", *slot);  // Deref to &String

// Free the slot
unsafe { slab.free(slot); }
```

### Macro Allocators — TLS-backed

Generate a thread-local allocator via macro. The allocator is a zero-sized
unit struct. Allocation goes through TLS.

```rust
mod my_alloc {
    nexus_slab::bounded_allocator!(MyType);
}

// Initialize (once per thread)
my_alloc::Allocator::builder().capacity(1024).build();

// Allocate via BoxSlot (RAII — auto-frees on drop)
let item = my_alloc::BoxSlot::try_new(MyType::default()).unwrap();
```

See [Macro Allocators](macros.md) for details.

## Handle Types

| Handle | Ownership | Drop behavior |
|--------|-----------|---------------|
| `RawSlot<T>` | Move-only, no RAII | Must call `free()` explicitly |
| `BoxSlot<T, A>` | Move-only, RAII | Frees slot on drop |
| `RcSlot<T, A>` | Reference-counted | Frees when last strong ref drops |
| `WeakSlot<T, A>` | Weak reference | Does not prevent deallocation |

See [BoxSlot & RcSlot](smart-handles.md) for details.

## Performance

| Operation | Bounded (p50) | Unbounded (p50) |
|-----------|--------------|----------------|
| Alloc | ~20 cycles | ~20 cycles (no growth) |
| Free | ~2 cycles | ~2 cycles |
| Deref | 0-1 cycles | 0-1 cycles |
| Growth | N/A | ~40 cycles (amortized) |

See `BENCHMARKS.md` for full methodology and results.
