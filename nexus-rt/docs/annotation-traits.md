# Compile-Time Configuration with Annotation Traits

Generic infrastructure code often needs to work with different concrete
strategies — handler storage, encoding formats, allocation policies —
chosen once at wire-up time with zero cost at dispatch time.

The **annotation trait** pattern solves this: a marker trait bundles an
associated type with factory methods, ZST implementations act as
compile-time selectors, and generic code parameterizes over the trait.
Monomorphization resolves everything — no vtable dispatch over the
configuration itself.

## The Pattern

### 1. Define the annotation trait

The trait carries the "what" (associated types) and the "how" (factory
methods). It must be `Send + 'static` so the resulting types can live in
[`World`].

```rust
use std::ops::DerefMut;

pub trait TimerConfig: Send + 'static {
    /// The concrete storage type for timer handlers.
    type Storage: DerefMut<Target = dyn Handler<Instant>> + Send + 'static;

    /// Wrap a concrete handler into the storage type.
    fn wrap(handler: impl Handler<Instant> + 'static) -> Self::Storage;
}
```

Key properties:

- **No `&self`** — `wrap()` is an associated function, not a method. The
  trait is never instantiated at runtime.
- **Associated type, not generic** — each config maps to exactly one
  storage type. This makes `Wheel<C::Storage>` a concrete `World`
  resource.

### 2. Implement with zero-sized markers

Each implementation is a ZST — no runtime data, just behavior:

```rust
pub struct BoxedTimers;

impl TimerConfig for BoxedTimers {
    type Storage = Box<dyn Handler<Instant>>;

    fn wrap(handler: impl Handler<Instant> + 'static) -> Self::Storage {
        Box::new(handler)
    }
}
```

Additional strategies follow the same shape:

```rust
/// Inline storage — no heap allocation, panics if handler exceeds buffer.
pub struct InlineTimers;

impl TimerConfig for InlineTimers {
    type Storage = FlatVirtual<Instant, B256>;

    fn wrap(handler: impl Handler<Instant> + 'static) -> Self::Storage {
        // ... inline buffer construction
    }
}

/// Inline with heap fallback — never panics.
pub struct FlexTimers;

impl TimerConfig for FlexTimers {
    type Storage = FlexVirtual<Instant, B256>;

    fn wrap(handler: impl Handler<Instant> + 'static) -> Self::Storage {
        // ... flex buffer construction
    }
}
```

### 3. Write generic code over the trait

Functions and types parameterized by `C: TimerConfig` work with any
storage strategy:

```rust
fn schedule_heartbeat<C: TimerConfig>(
    world: &mut World,
    handler: impl Handler<Instant> + 'static,
    deadline: Instant,
) {
    world.resource_mut::<Wheel<C::Storage>>()
        .schedule_forget(deadline, C::wrap(handler));
}
```

The caller picks the config:

```rust
// Heap-allocated handlers
schedule_heartbeat::<BoxedTimers>(&mut world, my_handler, deadline);

// Inline handlers — zero allocation
schedule_heartbeat::<InlineTimers>(&mut world, my_handler, deadline);
```

After monomorphization, each call site is fully concrete. No indirection,
no branching on the config.

### 4. Use defaults to keep simple cases simple

Default type parameters let callers omit the annotation when the common
case is good enough:

```rust
pub struct Periodic<C: TimerConfig = BoxedTimers> {
    inner: Option<C::Storage>,
    interval: Duration,
    _config: PhantomData<C>,
}
```

```rust
// Simple — uses BoxedTimers
let p = Periodic::boxed(handler, interval);

// Explicit — uses InlineTimers
let p = Periodic::<InlineTimers>::wrap(handler, interval);
```

## World Integration

The annotation's associated type determines the **resource type**
registered in `World`. Different configs produce different resources:

```rust
// BoxedTimers → registers Wheel<Box<dyn Handler<Instant>>>
let mut timer: TimerPoller = builder.install_driver(TimerInstaller::new(256));

// InlineTimers → registers Wheel<FlatVirtual<Instant, B256>>
let mut timer: TimerPoller<FlatVirtual<Instant, B256>> =
    builder.install_driver(TimerInstaller::<FlatVirtual<Instant, B256>>::new(256));
```

This is compile-time checked — scheduling a `BoxedTimers` handler into
an `InlineTimers` wheel is a type error.

## Composing with Other Patterns

### With `Handler` / `IntoHandler`

Handlers are created normally and wrapped by the config:

```rust
fn on_timeout(mut state: ResMut<AppState>, now: Instant) {
    state.last_heartbeat = now;
}

let handler = on_timeout.into_handler(world.registry());
let periodic = Periodic::<BoxedTimers>::wrap(handler, Duration::from_secs(1));
world.resource_mut::<TimerWheel>()
    .schedule_forget(Instant::now(), Box::new(periodic));
```

### With `Callback` / `IntoCallback`

Callbacks carry owned context. The same pattern applies:

```rust
fn reconnect(
    ctx: &mut ReconnectState,
    mut wheel: ResMut<TimerWheel>,
    reg: RegistryRef,
    now: Instant,
) {
    ctx.attempts += 1;
    if ctx.attempts < ctx.max_retries {
        let next = reconnect.into_callback(ctx.clone(), &reg);
        wheel.schedule_forget(now + ctx.backoff(), Box::new(next));
    }
}
```

### With Templates

[`HandlerTemplate`] resolves parameter state once, then stamps out
handlers cheaply. Combine with an annotation to control storage:

```rust
fn tick(mut counter: ResMut<u64>, _now: Instant) {
    *counter += 1;
}

let template = HandlerTemplate::new(tick, world.registry());
for deadline in deadlines {
    let handler = template.stamp();
    let periodic = Periodic::<BoxedTimers>::wrap(handler, interval);
    world.resource_mut::<TimerWheel>()
        .schedule_forget(deadline, Box::new(periodic));
}
```

## Writing Your Own Annotation Trait

The timer config pattern generalizes. Any time you have:

1. **Generic infrastructure** that shouldn't hardcode a strategy
2. **A small set of strategies** that differ in associated types
3. **Wire-up time selection** — the choice is made once, not per-call

Define your own annotation trait.

### Example: Log Sink Configuration

A logging driver that writes structured events. The sink format is
configurable at wire-up time:

```rust
/// Annotation trait for log sink format selection.
pub trait LogSinkConfig: Send + 'static {
    /// The concrete sink type stored as a World resource.
    type Sink: LogSink + Send + 'static;

    /// Create a sink with the given capacity.
    fn create(capacity: usize) -> Self::Sink;
}

pub trait LogSink {
    fn write(&mut self, record: &LogRecord);
    fn flush(&mut self);
}
```

Implementations:

```rust
pub struct JsonLogs;

impl LogSinkConfig for JsonLogs {
    type Sink = JsonSink;

    fn create(capacity: usize) -> Self::Sink {
        JsonSink::with_capacity(capacity)
    }
}

pub struct BinaryLogs;

impl LogSinkConfig for BinaryLogs {
    type Sink = BinarySink;

    fn create(capacity: usize) -> Self::Sink {
        BinarySink::with_capacity(capacity)
    }
}
```

Generic code:

```rust
fn emit_log<C: LogSinkConfig>(world: &mut World, record: &LogRecord) {
    world.resource_mut::<C::Sink>().write(record);
}

// At wire-up:
builder.register::<JsonSink>(JsonLogs::create(4096));
// or
builder.register::<BinarySink>(BinaryLogs::create(4096));
```

### Checklist

When designing an annotation trait:

- [ ] Trait is `Send + 'static` (resources must be sendable to `World`)
- [ ] Associated types are fully concrete (no nested generics that leak)
- [ ] Factory methods are associated functions, not `&self` methods
- [ ] Implementations are ZSTs — `PhantomData` if you need type params
- [ ] Default type parameter on generic structs for the common case
- [ ] Different configs produce different resource types in `World`
