# nexus-slab

A SLUB-style slab allocator for **predictable latency** and **cache-friendly allocation patterns**.

## Why SLUB?

SLUB (the Linux kernel's default allocator) uses a simple but powerful design: **per-type freelists with LIFO allocation order**. This gives you:

### 1. LIFO Cache Locality

When you free a slot and immediately allocate again, you get the same slot back — still hot in L1 cache. This is the common pattern in request handlers, event loops, and state machines: allocate, process, free, repeat.

```
Free slot A → A goes to head of freelist
Alloc       → Get A back, still in cache (5-8 cycles)

vs heap allocation:
Free A → tcache/arena bookkeeping
Alloc  → possibly different address, cold cache (40-60+ cycles)
```

### 2. Zero Fragmentation

Every slot is the same size. No external fragmentation, no coalescing, no compaction. A freed slot is immediately reusable by any allocation of that type.

### 3. Global Allocator Isolation

Your hot path doesn't compete with logging, serialization, or background tasks for the same allocator. The slab is yours alone — no lock contention, no tcache evictions from unrelated allocations.

### 4. Placement-New Optimization

The `Claim` API enables true placement construction — values are written directly into the slot without intermediate copies. Combined with `#[inline]`, LLVM can construct your struct in-place, eliminating memcpy overhead.

## Quick Start

```rust
// Define a type-specific allocator via macro
mod order_alloc {
    nexus_slab::bounded_allocator!(super::Order);
}

// Initialize once per thread (slab is thread-local)
order_alloc::Allocator::builder()
    .capacity(10_000)
    .build()?;

// 8-byte RAII handle — half the size of Box
let slot = order_alloc::BoxSlot::try_new(Order { id: 1, price: 100.0 })?;
assert_eq!(slot.id, 1);  // Deref to &Order

// Slot drops automatically, returns to freelist head (LIFO)
```

## Performance

All measurements in CPU cycles. See [BENCHMARKS.md](./BENCHMARKS.md) for methodology.

### Slot vs Box — Churn (alloc + deref + drop)

| Size | Slot p50 | Box p50 | Slot p99.9 | Box p99.9 |
|------|----------|---------|------------|-----------|
| 32B | **5** | 14 | **8** | 49 |
| 64B | **7** | 16 | **16** | 63 |
| 128B | **11** | 21 | **35** | 75 |
| 4096B | **157** | 175 | **264** | 329 |

**Slot is 2.8x faster at 32B.** The gap widens at tail latencies — Slot has no syscall path, no lock contention, no allocator state to maintain.

### Deallocation

| Size | Slot p50 | Box p50 |
|------|----------|---------|
| 32B | **2** | 12 |
| 64B | **2** | 12 |
| 1024B | **4** | 24 |

**Slot deallocation is 6x faster.** A slab free is a single pointer write to the freelist head. `free()` must update bin metadata, potentially coalesce, and manage tcache.

### Under Contention (Global Allocator Traffic)

| Size | Slab p50 | Box p50 | Slab p99.9 | Box p99.9 |
|------|----------|---------|------------|-----------|
| 64B | **2** | 18 | **16** | 106 |
| 4096B | **52** | 112 | **367** | 809 |

**Slab is 9x faster at 64B** when the global allocator is busy. This is the real-world advantage: your hot path stays fast even when background tasks are allocating.

## API

### Macros

| Macro | Description |
|-------|-------------|
| `bounded_allocator!(Type)` | Fixed capacity, returns `Err(Full)` when exhausted |
| `unbounded_allocator!(Type)` | Grows via chunks, never fails |
| `bounded_rc_allocator!(Type)` | Bounded + reference counting (`RcSlot`, `WeakSlot`) |
| `unbounded_rc_allocator!(Type)` | Unbounded + reference counting |

### Generated Types

Each macro generates:

| Type | Description |
|------|-------------|
| `Allocator` | Unit struct with `builder()` and `is_initialized()` |
| `BoxSlot` | 8-byte RAII handle (like `Box`, but from the slab) |
| `RcSlot` | Reference-counted handle (rc macros only) |
| `WeakSlot` | Weak reference (rc macros only) |

### BoxSlot API

```rust
// Construction
let slot = BoxSlot::try_new(value)?;  // Bounded: returns Result
let slot = BoxSlot::new(value);       // Unbounded: always succeeds

// Access (Deref + DerefMut)
let x = slot.field;
slot.field = new_value;

// Consumption
let value = BoxSlot::into_inner(slot);  // Extract value, free slot
let leaked = slot.leak();               // Leak to LocalStatic (permanent)

// Drop: automatic, returns slot to freelist
```

### Builder API

```rust
// Bounded: fixed capacity, fail-fast
mod order_alloc {
    nexus_slab::bounded_allocator!(super::Order);
}
order_alloc::Allocator::builder()
    .capacity(10_000)
    .build()?;

// Unbounded: grows in chunks
mod quote_alloc {
    nexus_slab::unbounded_allocator!(super::Quote);
}
quote_alloc::Allocator::builder()
    .chunk_size(4096)  // slots per chunk (default: 4096)
    .build()?;
```

## Bounded vs Unbounded

| | Bounded | Unbounded |
|---|---------|-----------|
| Growth | Fixed capacity | Adds chunks |
| Full behavior | Returns `Err(Full)` | Always succeeds |
| Allocation | ~5-8 cycles | ~7-10 cycles (chunk lookup) |
| Use case | Known capacity | Variable load |

**Use bounded** when you know your capacity — it's faster and fully deterministic.

**Use unbounded** when you need overflow headroom. Growth adds chunks without copying existing data (unlike `Vec` reallocation).

## Architecture

### SlotCell Union (SLUB-style)

Each slot is a `repr(C)` union — either a freelist pointer or a value:

```rust
#[repr(C)]
pub union SlotCell<T> {
    pub next_free: *mut SlotCell<T>,
    pub value: ManuallyDrop<MaybeUninit<T>>,
}
```

- **Vacant**: `next_free` points to next free slot (or null for end of list)
- **Occupied**: `value` contains the user's `T`

No tag, no metadata — writing a value overwrites the freelist pointer. The `Slot` RAII handle is the proof of occupancy.

### Freelist Layout

```
Bounded:
  free_head → SlotCell[2] → SlotCell[7] → SlotCell[4] → null
              (most recently freed, hottest in cache)

Unbounded:
  free_head → SlotCell in Chunk 1 → SlotCell in Chunk 0 → null
              (freelists are intra-chunk, chunk lookup on access)
```

### 8-Byte Handle

`BoxSlot` is 8 bytes — just a pointer to the `SlotCell`:

```
BoxSlot (8 bytes):
┌─────────────────────────┐
│ *mut SlotCell<T>        │  ← Direct pointer to value
└─────────────────────────┘
```

Compare to `Box<T>` at 8 bytes but with heap allocation overhead, or `Rc<T>` at 8 bytes with atomic refcount overhead.

## Thread Safety

Allocators are **thread-local** and `!Send + !Sync`. Each thread gets its own slab — no locking, no contention.

```rust
// This won't compile:
std::thread::spawn(move || {
    let slot = order_alloc::BoxSlot::try_new(order);  // Error: !Send
});
```

For cross-thread scenarios, use `RcSlot` with `into_slot_unchecked()` for controlled ownership transfer (unsafe).

## When to Use This

**Use nexus-slab when:**
- You churn same-type objects (orders, connections, timers, nodes)
- You need predictable tail latency
- Your hot path competes with background allocations
- You want stable pointers without `Pin`

**Use Box when:**
- Allocation is infrequent
- Types vary widely
- Simplicity matters more than performance

**Use the `slab` crate when:**
- You need key-based access and iteration
- You need a general-purpose slab data structure

## License

MIT OR Apache-2.0
