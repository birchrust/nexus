# SlotCell (Internal Reference)

The fundamental storage unit. A `repr(C)` union that overlays the value
with the freelist pointer — no extra memory per slot for bookkeeping.

## Layout

```
SlotCell<T> (union, repr(C)):
┌──────────────────────────────┐
│  next_free: *mut SlotCell<T> │  ← when vacant (on freelist)
│  ─── OR ───                  │
│  value: ManuallyDrop<        │  ← when occupied (allocated)
│           MaybeUninit<T>>    │
└──────────────────────────────┘
```

When a slot is vacant, the `next_free` pointer links it into the
freelist. When occupied, the same bytes hold the value. No wasted space.

## Transition

```
Vacant → Occupied:  write value into the union (overwrites next_free)
Occupied → Vacant:  drop value, write next_free pointer
```

Writing the value IS the transition. There's no separate "occupied" flag —
the `RawSlot` handle IS the proof of occupancy. If you hold a `RawSlot`,
the slot is occupied. When you free it, it's vacant.

## Accessor Methods

Fields are private. Access through methods:

| Method | Visibility | What |
|--------|-----------|------|
| `write_value(value)` | `pub` | Write value into the slot |
| `read_value() -> T` | `pub` | Move value out |
| `value_ref() -> &T` | `pub` | Borrow the value |
| `value_mut() -> &mut T` | `pub` | Mutably borrow the value |
| `set_next_free(ptr)` | `pub(crate)` | Set freelist pointer |
| `next_free() -> *mut Self` | `pub(crate)` | Read freelist pointer |

`pub` methods are needed by macros that expand at call sites in
downstream crates. `pub(crate)` methods are for internal freelist
management.

## Why a Union?

The alternative is a tagged enum (`Occupied(T)` / `Vacant(next)`). The
union saves the discriminant byte (which would add padding for alignment)
and avoids the branch on access. The `RawSlot` handle replaces the
discriminant — if you have the handle, the slot is occupied.
