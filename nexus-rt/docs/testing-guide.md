# Testing Handlers and Pipelines

nexus-rt provides `TestHarness` and `TestTimerDriver` for unit-testing
handlers, pipelines, and timer-driven logic. The design goal: tests are
deterministic, fast, and don't need a real event loop.

---

## TestHarness — The Basics

`TestHarness` wraps a `World` and gives you a single method:
`dispatch(handler, event)`. Each dispatch advances the world's sequence
number, mimicking what a real event loop does.

```rust
use nexus_rt::{
    Handler, IntoHandler, Res, ResMut, Resource, TestHarness, WorldBuilder,
};

#[derive(Resource, Default)]
struct Counter(u64);

fn increment(mut c: ResMut<Counter>, _event: ()) {
    c.0 += 1;
}

#[test]
fn handler_increments_counter() {
    let mut wb = WorldBuilder::new();
    wb.register(Counter::default());

    let mut harness = TestHarness::new(wb);
    let mut handler = increment.into_handler(harness.registry());

    harness.dispatch(&mut handler, ());
    harness.dispatch(&mut handler, ());
    harness.dispatch(&mut handler, ());

    assert_eq!(harness.world().resource::<Counter>().0, 3);
}
```

**Why TestHarness instead of constructing a World directly?**
- Auto-advances the sequence number, so handlers using `Seq` see realistic values
- Provides a stable place to add test utilities later
- Documents intent — this is a test, not production code

---

## Asserting on Resources

After dispatching, read resources from `harness.world()`:

```rust
#[test]
fn order_pipeline_updates_book() {
    let mut wb = WorldBuilder::new();
    wb.register(OrderBook::default());
    wb.register(RiskLimits { max_qty: 1000 });

    let mut harness = TestHarness::new(wb);
    let mut pipeline = build_order_pipeline(harness.registry());

    harness.dispatch(&mut pipeline, Order { qty: 100, price: 50.0 });

    let book = harness.world().resource::<OrderBook>();
    assert_eq!(book.bid_levels.len(), 1);
    assert_eq!(book.bid_levels[0].qty, 100);
}
```

For mutable access (e.g., to set up state mid-test):

```rust
harness.world_mut().resource_mut::<OrderBook>().clear();
```

---

## Dispatching Many Events

`dispatch_many` takes any iterable:

```rust
let events = vec![
    Order { qty: 100, price: 50.0 },
    Order { qty: 200, price: 51.0 },
    Order { qty: 150, price: 49.5 },
];

harness.dispatch_many(&mut pipeline, events);

assert_eq!(harness.world().resource::<OrderBook>().total_volume(), 450);
```

Each event advances the sequence number, so handlers see seq=1, 2, 3...

---

## Testing with Optional Resources

Handlers that take `Option<Res<T>>` should be tested both with and
without the resource registered:

```rust
fn maybe_log(logger: Option<Res<Logger>>, event: Order) {
    if let Some(log) = logger {
        log.info(&format!("order: {:?}", event));
    }
}

#[test]
fn maybe_log_without_logger() {
    let wb = WorldBuilder::new();
    let mut harness = TestHarness::new(wb);
    let mut handler = maybe_log.into_handler(harness.registry());

    // Should not panic even though Logger isn't registered
    harness.dispatch(&mut handler, Order { qty: 100, price: 50.0 });
}

#[test]
fn maybe_log_with_logger() {
    let mut wb = WorldBuilder::new();
    wb.register(Logger::default());

    let mut harness = TestHarness::new(wb);
    let mut handler = maybe_log.into_handler(harness.registry());

    harness.dispatch(&mut handler, Order { qty: 100, price: 50.0 });
    assert_eq!(harness.world().resource::<Logger>().messages.len(), 1);
}
```

---

## Testing Sequence Number Behavior

Handlers using `Seq` or `SeqMut` need to see realistic sequence
progression. `TestHarness` handles this automatically:

```rust
fn record_seq(seq: Seq, mut log: ResMut<Vec<i64>>, _event: ()) {
    log.push(seq.get().as_i64());
}

#[test]
fn handler_sees_advancing_sequence() {
    let mut wb = WorldBuilder::new();
    wb.register(Vec::<i64>::new());

    let mut harness = TestHarness::new(wb);
    let mut h = record_seq.into_handler(harness.registry());

    harness.dispatch(&mut h, ());
    harness.dispatch(&mut h, ());
    harness.dispatch(&mut h, ());

    let log = harness.world().resource::<Vec<i64>>();
    assert_eq!(log, &[1, 2, 3]);
}
```

For deterministic replay tests, you can manually set the sequence:

```rust
harness.world_mut().set_sequence(Sequence::from_i64(1000));
```

---

## TestTimerDriver — Virtual Time

Real timers depend on `Instant::now()` advancing. In tests you want full
control: advance time exactly N milliseconds, fire only the expected
timers, no sleeping. Requires the `timer` feature.

```rust
use nexus_rt::{
    timer::{TimerInstaller, TimerWheel},
    TestTimerDriver, TestHarness, WorldBuilder,
};
use nexus_timer::WheelBuilder;
use std::time::{Duration, Instant};

#[test]
fn timer_fires_after_advance() {
    let mut wb = WorldBuilder::new();
    let wheel = WheelBuilder::default().unbounded(64).build(Instant::now());
    let poller = wb.install_driver(TimerInstaller::new(wheel));

    let mut harness = TestHarness::new(wb);
    let mut timer = TestTimerDriver::new(poller);

    // Schedule a timer for 100ms from now
    let fire_at = timer.now() + Duration::from_millis(100);
    let handler: Box<dyn Handler<()>> = Box::new(my_callback.into_handler(harness.registry()));
    harness.world_mut()
        .resource_mut::<TimerWheel>()
        .schedule_forget(fire_at, handler);

    // Before the deadline: nothing fires
    assert_eq!(timer.poll(harness.world_mut()), 0);

    // Advance 50ms: still nothing
    timer.advance(Duration::from_millis(50));
    assert_eq!(timer.poll(harness.world_mut()), 0);

    // Advance another 60ms (total 110ms): fires
    timer.advance(Duration::from_millis(60));
    assert_eq!(timer.poll(harness.world_mut()), 1);
}
```

`TestTimerDriver` methods:
- `now() -> Instant` — current virtual time
- `advance(duration)` — move virtual time forward
- `set_now(instant)` — jump to a specific time
- `poll(world)` — fire all expired timers, returns count fired
- `next_deadline(world) -> Option<Instant>` — what's pending
- `len(world) -> usize` — how many timers are scheduled

---

## Testing Pipelines

Pipelines are handlers — same approach:

```rust
#[test]
fn pipeline_filters_invalid_orders() {
    let mut wb = WorldBuilder::new();
    wb.register(OrderLog::default());

    let mut harness = TestHarness::new(wb);
    let mut pipeline = PipelineBuilder::<RawOrder>::new()
        .then(parse, harness.registry())
        .guard(is_valid, harness.registry())
        .dispatch(record);

    // Valid order — should be recorded
    harness.dispatch(&mut pipeline, RawOrder::valid());
    assert_eq!(harness.world().resource::<OrderLog>().count(), 1);

    // Invalid order — should be filtered out
    harness.dispatch(&mut pipeline, RawOrder::invalid());
    assert_eq!(harness.world().resource::<OrderLog>().count(), 1);
}
```

---

## Testing Callbacks (Owned State)

Callbacks have per-instance state. You can construct one for the test
and dispatch directly:

```rust
use nexus_rt::IntoCallback;

struct SessionCounter {
    session_id: u32,
    events: u64,
}

fn count_session_event(
    ctx: &mut SessionCounter,
    _event: u64,
) {
    ctx.events += 1;
}

#[test]
fn callback_accumulates_state() {
    let wb = WorldBuilder::new();
    let mut harness = TestHarness::new(wb);

    let mut callback = count_session_event.into_callback(
        SessionCounter { session_id: 42, events: 0 },
        harness.registry(),
    );

    harness.dispatch(&mut callback, 1);
    harness.dispatch(&mut callback, 2);
    harness.dispatch(&mut callback, 3);

    // Inspect ctx — accessible because Callback owns it
    assert_eq!(callback.ctx.events, 3);
    assert_eq!(callback.ctx.session_id, 42);
}
```

The `callback.ctx` field is public — read it directly after dispatching
to verify state changes.

---

## Testing with Local State

Handlers with `Local<T>` parameters work the same way. Local state is
per-handler-instance, so create one handler per test:

```rust
fn dedup_sequential(
    mut last: Local<Option<i64>>,
    mut output: ResMut<Vec<i64>>,
    event: i64,
) {
    if last.as_ref() != Some(&event) {
        output.push(event);
        *last = Some(event);
    }
}

#[test]
fn dedup_filters_consecutive_duplicates() {
    let mut wb = WorldBuilder::new();
    wb.register(Vec::<i64>::new());

    let mut harness = TestHarness::new(wb);
    let mut handler = dedup_sequential.into_handler(harness.registry());

    harness.dispatch_many(&mut handler, vec![1, 1, 2, 2, 2, 3, 1]);

    assert_eq!(harness.world().resource::<Vec<i64>>(), &vec![1, 2, 3, 1]);
}
```

---

## Deterministic Replay

Combine `TestHarness` with `set_sequence` and `TestTimerDriver` for
fully deterministic replays:

```rust
#[test]
fn replay_deterministic_session() {
    let mut wb = WorldBuilder::new();
    // ... register all resources

    let mut harness = TestHarness::new(wb);
    let mut handler = build_session_handler(harness.registry());

    // Replay a recorded sequence
    let recorded_events = load_test_fixture("session_001.events");
    for (seq, event) in recorded_events {
        harness.world_mut().set_sequence(Sequence::from_i64(seq));
        handler.run(harness.world_mut(), event);
    }

    // Final state should match the snapshot
    let final_state = harness.world().resource::<SessionState>();
    assert_eq!(final_state, &load_test_snapshot("session_001.snapshot"));
}
```

This is how you build replay-driven regression tests: record real
sessions in production, replay them in CI, assert on final state.

---

## What Not to Test with TestHarness

`TestHarness` is for unit testing handler logic. It does NOT:
- Drive IO drivers (no real sockets, no mio)
- Run an event loop
- Test concurrency (single-threaded)
- Test the full poll loop integration

For those, use integration tests with a real `Runtime` (see
[poll-loop.md](poll-loop.md)) or use `nexus-async-rt`'s test utilities
when async behavior is involved.

---

## See Also

- [handlers.md](handlers.md) — Writing the handlers being tested
- [pipelines.md](pipelines.md) — Pipeline construction
- [callbacks.md](callbacks.md) — Stateful callbacks
- [poll-loop.md](poll-loop.md) — Integration testing with real drivers
