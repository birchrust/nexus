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
use nexus_slab::create_allocator;

// Define an allocator for your type
create_allocator!(order_alloc, Order);

// Initialize at startup (once per thread)
order_alloc::init().bounded(1024).build();

// Insert returns an 8-byte RAII Slot
let slot = order_alloc::insert(Order::new());
assert_eq!(slot.price, 100);

// Modify through the slot
slot.get_mut().quantity = 50;

// Slot auto-deallocates on drop
drop(slot);
assert_eq!(order_alloc::len(), 0);
```

## Performance

All measurements in CPU cycles. See [BENCHMARKS.md](./BENCHMARKS.md) for methodology.

### Macro API vs slab crate (p50)

| Operation | Slot API | Key-based | slab crate | Notes |
|-----------|----------|-----------|------------|-------|
| GET | **2** | 3 | 3 | Direct pointer, no lookup |
| GET (hot) | **1** | - | 2 | ILP - CPU pipelines loads |
| GET_MUT | **2** | 2 | 3 | Direct pointer |
| INSERT | 8 | - | **4** | +4 cycles TLS overhead |
| REMOVE | 4 | - | **3** | TLS overhead |
| REPLACE | **2** | - | 4 | Direct pointer, no lookup |
| CONTAINS | **2** | 3 | 2 | slot.is_valid() fastest |

**Key insight**: The TLS lookup adds ~4 cycles to INSERT/REMOVE, but access operations (GET/REPLACE) have zero overhead because `Slot` caches the pointer. For access-heavy workloads, this is a net win.

### Full Lifecycle Cost

| | Direct API | Macro API | Delta |
|---|---|---|---|
| INSERT | 7 | 11 | +4 |
| GET | 2 | 2 | 0 |
| REMOVE | 8 | 5 | -3 |
| **Total** | **17** | **18** | **+1** |

One cycle per object lifecycle for the ergonomics of a global allocator pattern.

### vs Box (Isolation Advantage)

The killer feature: **slab is isolated from the global allocator**. In production, `Box::new()` shares malloc with everything else. Your slab is yours alone.

| Size | Box p50 | Slab p50 | Box p99.99 | Slab p99.99 |
|------|---------|----------|------------|-------------|
| 64B | 15 | **10** | 6003 | **108** |
| 256B | 17 | 17 | 184 | **82** |
| 4096B | 133 | **71** | 419 | **367** |

**Tail latency**: At 64B, Box worst-case is **55x worse** (6003 vs 108 cycles). This is the difference between hitting malloc contention/syscalls vs a simple freelist pop.

See [BENCHMARKS.md](./BENCHMARKS.md) for full methodology and stress test results.

## Use Cases

### Node-Based Data Structures

```rust
use nexus_slab::{create_allocator, Key};

struct Node {
    value: i32,
    next: Key,  // 4 bytes, not 8 for Option<Box<Node>>
    prev: Key,
}

create_allocator!(node_alloc, Node);

fn build_list() {
    node_alloc::init().bounded(1000).build();

    let head = node_alloc::insert(Node {
        value: 1,
        next: Key::NONE,
        prev: Key::NONE,
    });

    // Keys are stable - safe to store in other nodes
    let head_key = head.key();

    let tail = node_alloc::insert(Node {
        value: 2,
        next: Key::NONE,
        prev: head_key,
    });

    head.get_mut().next = tail.key();
}
```

### Stable Memory Addresses

```rust
create_allocator!(buffer_alloc, [u8; 4096]);

fn get_stable_buffer() -> *const u8 {
    buffer_alloc::init().bounded(100).build();

    let slot = buffer_alloc::insert([0u8; 4096]);
    let ptr = slot.as_ptr() as *const u8;

    // Pointer remains valid as long as slot exists
    // No reallocation, no movement

    let _ = slot.leak();  // Keep alive, return key for later cleanup
    ptr
}
```

### Key Serialization

Keys can be converted to/from `u32` for external storage:

```rust
let slot = my_alloc::insert(value);
let key = slot.leak();

// Serialize
let raw: u32 = key.into_raw();
store_somewhere(raw);

// Deserialize
let raw = load_from_somewhere();
let key = Key::from_raw(raw);

// Access (caller must ensure validity)
let value = unsafe { my_alloc::get_unchecked(key) };
```

**Warning**: Keys are simple indices with no generation counter. If you store keys externally (databases, wire protocols), you must ensure the key is still valid before use. For wire protocols, prefer authoritative external identifiers (exchange order IDs, database primary keys) and use the slab key only for internal indexing.

## API

### Allocator Module (generated by `create_allocator!`)

| Function | Returns | Description |
|----------|---------|-------------|
| `init()` | `Builder` | Start configuring the allocator |
| `insert(value)` | `Slot` | Insert, panics if full |
| `try_insert(value)` | `Option<Slot>` | Insert, returns None if full |
| `contains_key(key)` | `bool` | Check if key is valid |
| `get_unchecked(key)` | `&'static T` | Get by key (unsafe) |
| `get_unchecked_mut(key)` | `&'static mut T` | Get mut by key (unsafe) |
| `len()` / `capacity()` | `usize` | Slot counts |
| `is_empty()` | `bool` | Check if empty |
| `is_initialized()` | `bool` | Check if init() was called |
| `shutdown()` | `Result<(), SlotsRemaining>` | Shutdown (must be empty) |

### Slot (8 bytes)

| Method | Returns | Description |
|--------|---------|-------------|
| `get()` | `&T` | Borrow the value |
| `get_mut()` | `&mut T` | Mutably borrow the value |
| `replace(value)` | `T` | Swap value, return old |
| `into_inner()` | `T` | Remove and return value |
| `key()` | `Key` | Get the key for this slot |
| `leak()` | `Key` | Keep alive, return key |
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
// Bounded: fixed capacity, returns None when full
my_alloc::init()
    .bounded(1024)
    .build();

// Unbounded: grows by adding chunks (no copying)
my_alloc::init()
    .unbounded()
    .chunk_capacity(4096)  // slots per chunk
    .capacity(10_000)      // pre-allocate
    .build();
```

## Bounded vs Unbounded

| | Bounded | Unbounded |
|---|---------|-----------|
| Growth | Fixed capacity | Adds chunks |
| Full behavior | Returns `None` | Always succeeds |
| Tail latency | Deterministic | +2-4 cycles chunk lookup |
| Use case | Known capacity | Unknown/variable load |

Use **bounded** when capacity is known—it's faster and fully deterministic.

Use **unbounded** when you need overflow headroom without `Vec` reallocation spikes.

## Architecture

### Slot Design

Each `Slot` is **8 bytes** (single pointer). The VTable for slab operations is stored in thread-local storage:

```
Slot (8 bytes):
┌─────────────────────────┐
│ *mut SlotCell<T>        │  ← Direct pointer to value
└─────────────────────────┘

TLS (per allocator):
┌─────────────────────────┐
│ *const VTable<T>        │  ← Cached for fast access
└─────────────────────────┘
```

This design gives:
- **8-byte handles** (vs 16+ for pointer+vtable designs)
- **Zero-cost access** (GET/REPLACE don't touch TLS)
- **RAII semantics** (drop returns slot to freelist)

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

Each thread has its own allocator instance. The allocator is `!Send` and `!Sync`.

**Do not store `Slot` in `thread_local!`**. Rust drops stack variables before TLS, so stack slots drop correctly. But if both `Slot` and the slab are in TLS, drop order is unspecified.

## Direct API (Advanced)

For cases where the macro API doesn't fit (multiple slabs, dynamic creation), use the direct API:

```rust
use nexus_slab::bounded::BoundedSlab;

let slab = BoundedSlab::with_capacity(1024);
let slot = slab.insert(42).unwrap();
assert_eq!(*slot.get(), 42);
```

See the [bounded](https://docs.rs/nexus-slab/latest/nexus_slab/bounded/) and [unbounded](https://docs.rs/nexus-slab/latest/nexus_slab/unbounded/) modules.

## License

MIT OR Apache-2.0
