# nexus-slab

A high-performance slab allocator for **stable memory addresses** without heap allocation overhead.

## What Is This?

`nexus-slab` is a **custom allocator pattern**—not a replacement for Rust's global allocator, but a specialized allocator for specific use cases where you need:

- **Stable memory addresses** - pointers remain valid until explicitly freed
- **Box-like semantics without Box** - RAII ownership with pre-allocated backing storage
- **Node-based data structures** - linked lists, trees, graphs with internal pointers
- **Predictable tail latency** - no reallocation spikes during growth

Think of `Slot<T>` as analogous to `Box<T>`: an owning handle that provides access to a value and deallocates on drop. The difference is that `Box` allocates from the heap on every call, while `Slot` allocates from a pre-allocated slab—making allocation O(1) with no syscalls.

## Quick Start

```rust
use nexus_slab::Allocator;

// Create an allocator for your type
let alloc: Allocator<Order> = Allocator::builder()
    .bounded(1024)
    .build();

// Insert returns a 16-byte RAII Slot
let mut slot = alloc.new_slot(Order::new());
assert_eq!(slot.price, 100);

// Modify through the slot (implements DerefMut)
slot.quantity = 50;

// Slot auto-deallocates on drop
drop(slot);
```

## Performance

All measurements in CPU cycles. See [BENCHMARKS.md](./BENCHMARKS.md) for methodology.

### Allocator API vs slab crate (p50)

| Operation | Slot API | Key-based | slab crate | Notes |
|-----------|----------|-----------|------------|-------|
| GET | **2** | 2 | 3 | Direct pointer, no lookup |
| GET (hot) | **0** | - | 1 | ILP - CPU pipelines loads |
| GET_MUT | **2** | 2 | 3 | Direct pointer |
| INSERT | 8 | - | **5** | vtable indirection |
| REMOVE | **4** | - | 4 | Direct vtable pointer |
| REPLACE | **3** | - | 4 | Direct pointer, no lookup |
| CONTAINS | **2** | 3 | 2 | slot.is_valid() |

### vs Box (Isolation Advantage)

The killer feature: **slab is isolated from the global allocator**. In production, `Box::new()` shares malloc with everything else. Your slab is yours alone.

#### Hot Cache (realistic steady-state)

| Size | Box p50 | Slab p50 | Box p99 | Slab p99 | Box p99.9 | Slab p99.9 |
|------|---------|----------|---------|----------|-----------|------------|
| 64B | 13 | **8** | 19 | **11** | 29 | **15** |
| 256B | 16 | **16** | 48 | **20** | 53 | **49** |
| 4096B | 108 | **59** | 162 | **81** | 229 | **152** |

#### Cold Cache (single-op, true first-access latency)

| Size | Box p50 | Slab p50 | Box p99 | Slab p99 |
|------|---------|----------|---------|----------|
| 64B | 158 | **84** | 300 | **154** |
| 256B | 168 | **108** | 314 | **176** |

**Key findings:**
- **Hot cache**: Slab **1.8x faster** at p50 for 4096B (59 vs 108 cycles)
- **Cold single-op**: Slab **1.9x faster** at p50 for 64B (84 vs 158 cycles)
- **Tail latency**: Slab consistently 1.7-2x better at p99

See [BENCHMARKS.md](./BENCHMARKS.md) for full methodology and stress test results.

## Use Cases

### Node-Based Data Structures

```rust
use nexus_slab::{Allocator, Key};

struct Node {
    value: i32,
    next: Key,  // 4 bytes, not 8 for Option<Box<Node>>
    prev: Key,
}

let alloc: Allocator<Node> = Allocator::builder().bounded(1000).build();

let head = alloc.new_slot(Node {
    value: 1,
    next: Key::NONE,
    prev: Key::NONE,
});

// Keys are stable - safe to store in other nodes
let head_key = head.key();

let mut tail = alloc.new_slot(Node {
    value: 2,
    next: Key::NONE,
    prev: head_key,
});
```

### Stable Memory Addresses

```rust
use nexus_slab::Allocator;

let alloc: Allocator<[u8; 4096]> = Allocator::builder().bounded(100).build();

let slot = alloc.new_slot([0u8; 4096]);
let ptr = slot.as_ptr() as *const u8;

// Pointer remains valid as long as slot exists
// No reallocation, no movement

let key = slot.leak();  // Keep alive, return key for later cleanup
```

### Key Serialization

Keys can be converted to/from `u32` for external storage:

```rust
use nexus_slab::{Allocator, Key};

let alloc: Allocator<MyValue> = Allocator::builder().bounded(1000).build();

let slot = alloc.new_slot(value);
let key = slot.leak();

// Serialize
let raw: u32 = key.into_raw();
store_somewhere(raw);

// Deserialize
let raw = load_from_somewhere();
let key = Key::from_raw(raw);

// Access (caller must ensure validity)
let value = unsafe { alloc.get_unchecked(key) };
```

**Warning**: Keys are simple indices with no generation counter. If you store keys externally (databases, wire protocols), you must ensure the key is still valid before use. For wire protocols, prefer authoritative external identifiers (exchange order IDs, database primary keys) and use the slab key only for internal indexing.

## API

### Allocator

| Method | Returns | Description |
|--------|---------|-------------|
| `Allocator::builder()` | `Builder` | Start configuring an allocator |
| `new_slot(value)` | `Slot` | Insert, panics if full |
| `try_new_slot(value)` | `Option<Slot>` | Insert, returns None if full |
| `contains_key(key)` | `bool` | Check if key is valid |
| `get(key)` | `Option<&T>` | Get by key |
| `get_mut(key)` | `Option<&mut T>` | Get mut by key |
| `get_unchecked(key)` | `&T` | Get by key (unsafe) |
| `get_unchecked_mut(key)` | `&mut T` | Get mut by key (unsafe) |
| `remove_by_key(key)` | `T` | Remove by key (panics if invalid) |
| `try_remove_by_key(key)` | `Option<T>` | Remove by key |
| `len()` / `capacity()` | `usize` | Slot counts |

`Allocator` is `Copy` - it's just a pointer to leaked storage.

### Slot (16 bytes)

| Method | Returns | Description |
|--------|---------|-------------|
| `key()` | `Key` | Get the key for this slot |
| `leak()` | `Key` | Keep alive, return key |
| `into_inner()` | `T` | Remove and return value |
| `replace(value)` | `T` | Swap value, return old |
| `is_valid()` | `bool` | Check if slot is still valid |
| `as_ptr()` | `*const T` | Raw pointer to value |
| `as_mut_ptr()` | `*mut T` | Mutable raw pointer |

`Slot` implements `Deref` and `DerefMut` for ergonomic access.

### Key (4 bytes)

| Method | Returns | Description |
|--------|---------|-------------|
| `index()` | `u32` | The slot index |
| `into_raw()` | `u32` | For serialization |
| `from_raw(u32)` | `Key` | From serialized value |
| `is_none()` | `bool` | Check if sentinel |
| `is_some()` | `bool` | Check if valid |
| `Key::NONE` | `Key` | Sentinel value |

### Builder Pattern

```rust
use nexus_slab::Allocator;

// Bounded: fixed capacity, panics when full
let alloc: Allocator<MyType> = Allocator::builder()
    .bounded(1024)
    .build();

// Unbounded: grows by adding chunks (no copying)
let alloc: Allocator<MyType> = Allocator::builder()
    .unbounded()
    .chunk_capacity(4096)  // slots per chunk
    .prealloc(10_000)      // pre-allocate
    .build();
```

## Bounded vs Unbounded

| | Bounded | Unbounded |
|---|---------|-----------|
| Growth | Fixed capacity | Adds chunks |
| Full behavior | Panics | Always succeeds |
| Tail latency | Deterministic | +2-4 cycles chunk lookup |
| Use case | Known capacity | Unknown/variable load |

Use **bounded** when capacity is known—it's faster and fully deterministic.

Use **unbounded** when you need overflow headroom without `Vec` reallocation spikes.

## Architecture

### Slot Design

Each `Slot` is **16 bytes** (slot pointer + vtable pointer):

```
Slot (16 bytes):
┌─────────────────────────┐
│ *mut SlotCell<T>        │  ← Direct pointer to value
├─────────────────────────┤
│ &'static VTable<T>      │  ← Embedded vtable for deallocation
└─────────────────────────┘
```

This design gives:
- **Zero-cost access** (GET/REPLACE use direct pointer)
- **RAII semantics** (drop returns slot to freelist via vtable)
- **No TLS lookups** (vtable pointer embedded in Slot)

### Memory Layout

```
Slab (contiguous allocation):
┌──────────────────────────────────────────┐
│ SlotCell 0: [stamp: u64][value: T]       │
│ SlotCell 1: [stamp: u64][value: T]       │
│ ...                                       │
│ SlotCell N: [stamp: u64][value: T]       │
└──────────────────────────────────────────┘
```

### No Generational Indices

Keys are simple indices. This is intentional—see the [Key documentation](https://docs.rs/nexus-slab/latest/nexus_slab/struct.Key.html) for rationale.

**TL;DR**: Your data has authoritative external identifiers (exchange order IDs, database keys). You validate against those anyway. Generational indices add ~8 cycles to catch bugs that domain validation already catches.

## Thread Safety

`Allocator` and `Slot` are `!Send` and `!Sync`. Each allocator must only be used from the thread that created it.

```rust
use nexus_slab::Allocator;

// This won't compile:
// std::thread::spawn(move || {
//     let slot = alloc.new_slot(42);  // Error: Allocator is !Send
// });
```

## License

MIT OR Apache-2.0
