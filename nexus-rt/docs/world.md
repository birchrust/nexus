# World & Resources

The `World` is a typed key-value store. Each type gets one slot. Handlers
access resources by type — `Res<T>` for shared read, `ResMut<T>` for
exclusive write.

## WorldBuilder

All registration happens at setup time through `WorldBuilder`. Once
`build()` is called, the set of resources is sealed.

```rust
use nexus_rt::{WorldBuilder, Resource};

#[derive(Resource, Default)]
struct GameState { /* ... */ }

#[derive(Resource)]
struct Config { /* ... */ }

let mut wb = WorldBuilder::new();

// Register resources by type
wb.register(GameState::default());
wb.register(load_config());

// Install drivers (they register their own resources)
let timer = wb.install_driver(TimerInstaller::new(wheel));
let clock = wb.install_driver(RealtimeClockInstaller::default());

// Seal — no more registration
let mut world = wb.build();
```

### Why build-time registration?

Resources are stored as individually heap-allocated `ResourceCell<T>` values
with direct `NonNull<u8>` pointers. `ResourceId` is assigned at registration
time and is stable for the World's lifetime. O(1) access, single deref.

If resources could be added at runtime, the array might need to grow,
invalidating existing ResourceIds. The build-time seal prevents this.

## Accessing Resources

### In handlers — via parameter types

```rust
fn my_handler(
    config: Res<Config>,         // shared read
    mut state: ResMut<GameState>, // exclusive write
    clock: Res<Clock>,           // another shared read
) {
    if config.debug_mode {
        state.debug_count += 1;
    }
    let ts = clock.unix_nanos();
}
```

The framework resolves parameters at build time (`into_handler`). At
dispatch time, it provides references directly from the World — one
pointer dereference per parameter.

### Outside handlers — via World methods

```rust
// Read
let config = world.resource::<Config>();

// Write
let state = world.resource_mut::<GameState>();
state.counter += 1;

// By ResourceId (for drivers with pre-resolved IDs)
let clock = unsafe { world.get_mut::<Clock>(clock_id) };
```

## ResourceId

`ResourceId` is a pre-resolved handle to a specific resource slot. It's
a raw pointer internally — dereferencing it is O(1), no lookup.

Drivers use ResourceIds to avoid the type-based lookup on every poll:

```rust
// At install time:
let clock_id = world_builder.register(Clock::default()); // returns ResourceId

// At poll time:
let clock = unsafe { world.get_mut::<Clock>(clock_id) }; // one deref
```

The `unsafe` is required because the caller must guarantee the ResourceId
is valid and the type matches. Drivers get this guarantee from the
registration during `install()`.

## Resource Rules

1. **One per type** — `register::<T>(value)` stores one `T`. Registering
   the same type twice panics. If you need multiple instances of the
   same type, use a newtype wrapper.

2. **`Send + 'static`** — resources must be `Send` (can cross thread
   boundaries at setup) and `'static` (no borrows).

3. **No runtime addition** — all resources must be registered before
   `build()`. This is enforced by the type system — `World` doesn't
   expose a `register` method.

4. **Mutable access is exclusive** — `ResMut<T>` gives `&mut T`.
   Two handlers accessing `ResMut<T>` on the same type would conflict.
   The framework checks for conflicts at build time.

## Optional Resources

Handlers can declare optional dependencies:

```rust
fn my_handler(
    state: ResMut<MyState>,
    debug: Option<Res<DebugConfig>>,  // None if not registered
) {
    if let Some(debug) = debug {
        // DebugConfig was registered
    }
}
```

`Option<Res<T>>` resolves to `None` if the resource doesn't exist in the
World, rather than panicking.

## Local State

Handlers can have per-instance state via `Local<T>`:

```rust
fn my_handler(
    mut call_count: Local<u64>,  // per-handler, not shared
    state: Res<SharedState>,
) {
    *call_count += 1;
    // call_count is unique to this handler instance
}
```

`Local<T>` is stored in the handler's callback, not in the World. Each
handler instance gets its own copy. `T` must implement `Default`.
