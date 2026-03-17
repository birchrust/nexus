# BoxSlot & RcSlot

RAII and reference-counted handles for slab-allocated objects.

## BoxSlot — RAII Handle

`BoxSlot<T, A>` owns a slab slot and frees it on drop. Analogous to `Box<T>`
but backed by the slab instead of the global allocator.

```rust
// Create via macro allocator
let item = my_alloc::BoxSlot::try_new(MyType::default()).unwrap();

// Deref to the value
println!("{}", *item);

// Auto-frees on drop — no manual free() needed
drop(item);
```

### Converting Between Handle Types

```rust
// BoxSlot → RawSlot (take ownership, no auto-free)
let raw = item.into_slot();

// RawSlot → BoxSlot (re-wrap for RAII)
let item = unsafe { my_alloc::BoxSlot::from_slot(raw) };

// BoxSlot → owned value (extracts and frees slot)
let value = item.into_inner();

// BoxSlot → static reference (leaks the slot, value lives forever)
let static_ref: &'static MyType = item.leak();
```

### Replace

Swap the value in-place without reallocating:

```rust
let old_value = item.replace(MyType::new_value());
```

## RcSlot — Reference Counted

`RcSlot<T, A>` is a reference-counted handle. Multiple `RcSlot`s can
point to the same slab slot. The slot is freed when the last strong
reference drops.

```rust
let rc1 = my_alloc::RcSlot::new(MyType::default());
let rc2 = rc1.clone();  // both point to the same slot

drop(rc1);  // slot still alive (rc2 holds it)
drop(rc2);  // last ref — slot freed
```

### Weak References

```rust
let rc = my_alloc::RcSlot::new(MyType::default());
let weak = rc.downgrade();

// Upgrade returns Some if strong refs exist
assert!(weak.upgrade().is_some());

drop(rc);  // last strong ref gone
assert!(weak.upgrade().is_none());
```

### Raw Pointer API

For unsafe low-level usage (e.g., intrusive data structures):

```rust
// Get the raw pointer
let ptr = RcSlot::into_raw(rc);

// Reconstruct from raw pointer
let rc = unsafe { RcSlot::from_raw(ptr) };

// Manual refcount management
unsafe { RcSlot::<T, A>::increment_strong_count(ptr); }
unsafe { RcSlot::<T, A>::decrement_strong_count(ptr); }
```

### Identity Comparison

```rust
let a = my_alloc::RcSlot::new(42);
let b = a.clone();
let c = my_alloc::RcSlot::new(42);

assert!(RcSlot::ptr_eq(&a, &b));   // same slot
assert!(!RcSlot::ptr_eq(&a, &c));  // different slots, same value
```

## When to Use Which

| Handle | When |
|--------|------|
| `RawSlot` | Manual memory management, maximum control |
| `BoxSlot` | Single owner, RAII cleanup |
| `RcSlot` | Shared ownership (intrusive collections, caches) |
