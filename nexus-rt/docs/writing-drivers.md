# Writing Your Own Driver

This guide is for users implementing their own drivers — protocol
handlers, custom IO sources, exchange connectivity layers, etc. The
[drivers.md](drivers.md) doc covers the user-facing API. This one
covers the implementation patterns and the subtle traps you'll hit.

If you're just consuming existing drivers (timer, clock, IO), you can
skip this doc.

---

## What a Driver Is

A driver is anything that:

1. **Owns external state** that needs to be polled — sockets, hardware,
   timers, message queues, etc.
2. **Dispatches handlers** in response to that state changing.
3. **Stores its state in a World resource** so handlers can access it.

The standard shape:

```rust
// 1. The Installer carries config and registers resources at setup time.
pub struct MyDriverInstaller {
    config: MyConfig,
}

impl Installer for MyDriverInstaller {
    type Poller = MyDriverPoller;

    fn install(self, wb: &mut WorldBuilder) -> Self::Poller {
        // Register your driver's state as a World resource
        wb.register(MyDriverState::new(self.config));
        // Resolve the ResourceId for fast dispatch-time access
        let state_id = wb.registry().id::<MyDriverState>();
        MyDriverPoller { state_id }
    }
}

// 2. The Poller is the runtime handle. Cheap (just a ResourceId).
pub struct MyDriverPoller {
    state_id: ResourceId,
}

impl MyDriverPoller {
    pub fn poll(&mut self, world: &mut World) {
        let state = world.resource_mut::<MyDriverState>();
        // ... drive the external system, dispatch handlers
    }
}
```

That's the basic skeleton. Now let's talk about where it gets hard.

---

## The Self-Referential Dispatch Problem

This is the trap that bites everyone who writes a driver with dynamic
handler registration. Read it carefully — it's subtle and the compiler
won't help you.

### The setup

Your driver stores handlers in a HashMap (or Vec, or whatever):

```rust
pub struct MyDriverState {
    handlers: HashMap<u32, Box<dyn Handler<Message>>>,
}
```

When a message arrives, you look up the handler and dispatch it:

```rust
impl MyDriverPoller {
    pub fn poll(&mut self, world: &mut World) {
        let state = world.resource_mut::<MyDriverState>();
        for (key, msg) in incoming_messages() {
            if let Some(handler) = state.handlers.get_mut(&key) {
                handler.run(world, msg); // ← PROBLEM
            }
        }
    }
}
```

This won't compile. You're holding `&mut state.handlers` (which lives
inside `world.resource_mut::<MyDriverState>()`) AND you want to pass
`&mut World` to the handler. Two mutable borrows over overlapping
memory.

### Why it matters

Even if you could compile this (with `unsafe` casts), it would be UB if
the handler does anything reasonable. The most common case: the handler
needs to **register a new handler** in the same map.

```rust
fn admin_command_handler(world: &mut World, msg: AdminCommand) {
    // The user wants to register a new handler at runtime
    let state = world.resource_mut::<MyDriverState>();
    state.handlers.insert(msg.new_key, Box::new(my_new_handler));
}
```

That `state.handlers.insert()` is a mutable borrow of the same HashMap
the driver was iterating. If we'd kept the original `&mut handler` from
the iteration, we now have two mutable references to the same map.
HashMap reallocation could invalidate the original `&mut handler`. UB.

This isn't theoretical — it's the standard failure mode for any driver
that supports runtime handler registration. The classic case is
"administrative commands that register new subscriptions."

### The fix: take, dispatch, return

The correct pattern is to **temporarily move the handler out of the
map**, dispatch it (which can now freely borrow the rest of the world,
including the driver), then move it back in when done.

```rust
pub struct MyDriverState {
    // Note: Option so we can take() it
    handlers: HashMap<u32, Option<Box<dyn Handler<Message>>>>,
}

impl MyDriverPoller {
    pub fn poll(&mut self, world: &mut World) {
        // Collect keys upfront — we can't iterate while modifying
        let keys: Vec<u32> = {
            let state = world.resource::<MyDriverState>();
            state.handlers.keys().copied().collect()
        };

        for key in keys {
            // Take the handler out of the map, releasing the borrow
            let mut handler = {
                let state = world.resource_mut::<MyDriverState>();
                match state.handlers.get_mut(&key) {
                    Some(slot) => slot.take(),
                    None => None,
                }
            };

            if let Some(ref mut h) = handler {
                // Dispatch with full World access — the handler can now
                // freely borrow MyDriverState and modify state.handlers.
                let msg = next_message_for(key);
                h.run(world, msg);
            }

            // Put it back. The slot may have been replaced during dispatch
            // (e.g., the handler called insert() with the same key), so
            // we have to be careful.
            if let Some(h) = handler {
                let state = world.resource_mut::<MyDriverState>();
                match state.handlers.get_mut(&key) {
                    // Slot is still empty — restore our handler
                    Some(slot @ None) => *slot = Some(h),
                    // Slot was filled during dispatch — caller wins, drop ours
                    Some(Some(_)) => drop(h),
                    // Entry was removed during dispatch — drop ours
                    None => drop(h),
                }
            }
        }
    }
}
```

The key insight: while the handler is OUT of the map (replaced with
`None`), the map is in a temporarily-empty state for that key. The
handler being dispatched can do anything to the map — including insert
the same key — without aliasing. When dispatch completes, we restore
the handler if the slot is still vacant, or drop it if the user already
filled it during dispatch.

### Policy decisions

When you put the handler back, you have to decide what happens if the
user replaced it during dispatch:

**Option A: Caller wins.** The user's replacement is intentional. Drop
your old handler, leave theirs. This is what the example above does.
Use this when "register a new handler" is a normal operation.

**Option B: First-write wins.** The original handler is the canonical
one. Reject the user's replacement (or queue it for later). Use this
when handlers are long-lived and replacements are exceptional.

**Option C: Panic.** Mid-dispatch replacement is a logic error. Use
this in debug builds to catch bugs early.

The reactor system (see [reactors.md](reactors.md)) uses **Option C
implicitly** by deferring removals via `DeferredRemovals` — the user
queues a removal, but the actual map mutation happens at the end of
the cycle, after dispatch is complete. This avoids the take/return
dance entirely.

---

## Alternative: Deferred Operations

If the take/return pattern feels fragile, use the deferred-operations
approach: collect mutations into a queue during dispatch, apply them at
the end of the cycle.

```rust
pub struct MyDriverState {
    handlers: HashMap<u32, Box<dyn Handler<Message>>>,
    pending_inserts: Vec<(u32, Box<dyn Handler<Message>>)>,
    pending_removes: Vec<u32>,
}

impl MyDriverState {
    /// Called by handlers during dispatch. Doesn't mutate `handlers`.
    pub fn register(&mut self, key: u32, handler: Box<dyn Handler<Message>>) {
        self.pending_inserts.push((key, handler));
    }

    pub fn deregister(&mut self, key: u32) {
        self.pending_removes.push(key);
    }

    /// Called by the poller after the dispatch loop completes.
    fn apply_pending(&mut self) {
        for key in self.pending_removes.drain(..) {
            self.handlers.remove(&key);
        }
        for (key, handler) in self.pending_inserts.drain(..) {
            self.handlers.insert(key, handler);
        }
    }
}
```

The poller still needs the take/return pattern to dispatch handlers
without aliasing the map, but the user-facing API is much simpler:
`state.register(key, handler)` always works, no error cases.

The downside: a handler that registers a new handler won't see it fire
in the same poll cycle — the new handler is queued, not active yet.
For most use cases (admin commands, dynamic subscriptions) this is the
correct behavior.

The reactor system uses this pattern via `DeferredRemovals`. See
[reactors.md](reactors.md) for the canonical example.

---

## Alternative: Index-Based Dispatch

If your handlers are addressed by integer index (not arbitrary keys),
you can avoid the HashMap aliasing problem entirely by storing handlers
in a `Vec` and using `mem::replace` to swap a slot:

```rust
pub struct MyDriverState {
    handlers: Vec<Option<Box<dyn Handler<Message>>>>,
}

impl MyDriverPoller {
    pub fn poll(&mut self, world: &mut World) {
        let len = world.resource::<MyDriverState>().handlers.len();

        for idx in 0..len {
            // Swap out: replace slot with None, get the handler
            let mut handler = {
                let state = world.resource_mut::<MyDriverState>();
                std::mem::replace(&mut state.handlers[idx], None)
            };

            if let Some(ref mut h) = handler {
                h.run(world, next_message_for(idx));
            }

            // Swap back: only if slot is still empty
            if let Some(h) = handler {
                let state = world.resource_mut::<MyDriverState>();
                if state.handlers[idx].is_none() {
                    state.handlers[idx] = Some(h);
                }
            }
        }
    }
}
```

`Vec<Option<Box<dyn Handler>>>` won't reallocate on `insert` (it's
just an index assignment), so the take/return is bounded to a single
slot. The user can still grow the vec during dispatch, but that
happens through `push` which goes to a new index — doesn't affect any
slot we're currently iterating.

This is faster than the HashMap pattern (no hash computation, no
reallocation risk) and the implementation is simpler. Use it when your
handler keys are dense integers (e.g., file descriptors, mio tokens,
slot indices).

---

## Alternative: Split State

If the driver's state can be split into "handlers" and "everything
else," store them in **separate** World resources. The handler can take
`&mut OtherState` while the driver iterates `&mut HandlerMap` — no
aliasing because they're different allocations.

```rust
// Two resources, registered separately
wb.register(MyDriverHandlers { map: HashMap::new() });
wb.register(MyDriverConfig { ..., counters: 0 });

// In the poller:
fn poll(&mut self, world: &mut World) {
    let handlers = world.resource_mut::<MyDriverHandlers>();
    for (key, handler) in handlers.map.iter_mut() {
        // handler.run takes &mut World, but it can't access
        // MyDriverHandlers (we hold the only &mut to it).
        // It CAN access MyDriverConfig because that's a separate resource.
        handler.run(world, next_message_for(*key));  // ← still aliases!
    }
}
```

Wait — that still doesn't work. `&mut World` is too broad; it covers
ALL resources including `MyDriverHandlers`. You'd need a way to pass a
"World minus this resource" reference, which Rust can't express.

So split state alone doesn't fix the problem. You still need the
take/return or deferred-operations pattern. But splitting state is a
useful complement: it lets the dispatched handler access driver
metadata (counters, config) without aliasing the handler map itself.

---

## The Easy Path: Don't Allow Mid-Dispatch Registration

If your driver doesn't need to support handler registration at runtime,
none of this matters. Register handlers at setup time only:

```rust
pub struct MyDriverInstaller {
    handlers: HashMap<u32, Box<dyn Handler<Message>>>,
}

impl MyDriverInstaller {
    pub fn add_handler(mut self, key: u32, handler: impl Handler<Message> + 'static) -> Self {
        self.handlers.insert(key, Box::new(handler));
        self
    }
}

impl Installer for MyDriverInstaller {
    type Poller = MyDriverPoller;
    fn install(self, wb: &mut WorldBuilder) -> Self::Poller {
        wb.register(MyDriverState { handlers: self.handlers });
        MyDriverPoller { /* ... */ }
    }
}
```

At setup, the user calls `installer.add_handler(...).add_handler(...).build()`.
All handlers are present before `install()`. After install, the handler
set is frozen. The poller iterates the map without worrying about
mid-dispatch mutation because there's no API to mutate it.

This is the simplest design and it covers many use cases. Only pick the
take/return or deferred-operations pattern if you genuinely need
runtime handler registration (admin commands, dynamic subscriptions,
session management).

---

## Wakers and Notification

If your driver needs to wake other parts of the system (a task is
ready, a deadline expired), you have two options:

### Direct dispatch

The driver calls handlers synchronously during its own `poll()`. This
is what the timer driver does — `TimerPoller::poll()` fires expired
handlers in-line. Simple, low-latency, but the handlers run on the
driver's poll thread.

### Notification + deferred dispatch

The driver marks a flag or pushes to a queue, and a separate poller
later picks up the notification and dispatches. This is what the
reactor system does — `mark()` is O(1) and just sets a bit.
`drain_pending()` collects the marked tokens and the user's poll loop
dispatches them.

Use direct dispatch when latency matters and the handler set is small.
Use notification + deferred dispatch when you have many handlers, the
notification source is on a critical path, or you need to dedup
multiple wakes into one dispatch.

---

## Summary: Picking a Pattern

| Need | Pattern |
|------|---------|
| Static handler set, registered at setup only | Frozen map in installer |
| Runtime registration, dense integer keys | Vec<Option<Box>> with mem::replace |
| Runtime registration, arbitrary keys | HashMap with take/return |
| Many concurrent registrations during dispatch | Deferred operations queue |
| O(1) wake from anywhere, batched dispatch | Notification flag + drain |

The take/return pattern is the workhorse — most drivers can use it.
Deferred operations are the cleanest when you need many mid-dispatch
mutations. The frozen-map approach is the simplest if you can get away
with it.

---

## See Also

- [drivers.md](drivers.md) — Using existing drivers (timer, clock, IO)
- [reactors.md](reactors.md) — Interest-based dispatch with deferred removal
- [handlers.md](handlers.md) — The Handler trait the driver dispatches
- [poll-loop.md](poll-loop.md) — How drivers compose into your event loop
