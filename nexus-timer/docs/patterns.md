# Patterns

## Request timeouts

The classic timer-wheel workload: every outgoing request gets a deadline,
most are cancelled when the response arrives, the survivors fire as
timeouts.

```rust
use std::time::{Duration, Instant};
use nexus_timer::{Wheel, TimerHandle};
use std::collections::HashMap;

pub struct RequestTracker {
    wheel: Wheel<RequestId>,
    pending: HashMap<RequestId, TimerHandle<RequestId>>,
}
# #[derive(Clone, Copy, Hash, PartialEq, Eq)] pub struct RequestId(u64);

impl RequestTracker {
    pub fn new(now: Instant) -> Self {
        Self {
            wheel: Wheel::unbounded(4096, now),
            pending: HashMap::new(),
        }
    }

    pub fn start(&mut self, id: RequestId, timeout_at: Instant) {
        let handle = self.wheel.schedule(timeout_at, id);
        self.pending.insert(id, handle);
    }

    /// Response arrived before deadline — cancel the timer.
    pub fn complete(&mut self, id: RequestId) {
        if let Some(handle) = self.pending.remove(&id) {
            self.wheel.cancel(handle);
        }
    }

    /// Periodic poll — returns IDs whose timers fired.
    pub fn poll(&mut self, now: Instant, fired: &mut Vec<RequestId>) {
        let start = fired.len();
        self.wheel.poll(now, fired);
        for id in &fired[start..] {
            self.pending.remove(id);
        }
    }
}
```

The hot path here is `start` + `complete` — both are O(1). Timeouts
(`poll` + fire) are the exception.

## Exchange heartbeats

Heartbeats are periodic fire-and-forget timers. Use `schedule_forget` and
let them fire naturally.

```rust
use std::time::{Duration, Instant};
use nexus_timer::Wheel;

pub struct Heartbeat;

pub fn schedule_heartbeat(wheel: &mut Wheel<Heartbeat>, now: Instant) {
    wheel.schedule_forget(now + Duration::from_secs(10), Heartbeat);
}

// In your poll loop:
fn on_heartbeat_tick<F: FnMut()>(mut send: F) {
    send();
}
```

For *recurring* heartbeats, re-schedule from the fire handler:

```rust
# use std::time::{Duration, Instant};
# use nexus_timer::Wheel;
# pub struct Heartbeat;
fn tick(wheel: &mut Wheel<Heartbeat>, _fired: Heartbeat, now: Instant) {
    // Do the heartbeat work...
    send_heartbeat();

    // Re-arm for the next interval
    wheel.schedule_forget(now + Duration::from_secs(10), Heartbeat);
}
# fn send_heartbeat() {}
```

## Deadline-driven event loop

Use `next_deadline` to compute how long to sleep between polls:

```rust
use std::time::{Duration, Instant};
use nexus_timer::Wheel;

fn event_loop_step<T>(wheel: &mut Wheel<T>, fired: &mut Vec<T>) -> Duration {
    let now = Instant::now();
    wheel.poll(now, fired);

    match wheel.next_deadline() {
        Some(next) => next.saturating_duration_since(Instant::now()),
        None       => Duration::from_millis(100),  // idle
    }
}
```

Caller can then sleep (or epoll-wait) for the returned duration before the
next iteration. This gives you accurate wakeups without constant polling.

## Budgeted fire cap for bounded tail latency

If you have thousands of timers firing in a burst, unbounded poll can spike
your event-loop iteration time. Cap it with `poll_with_limit`:

```rust
use std::time::Instant;
use nexus_timer::Wheel;

const MAX_TIMERS_PER_TICK: usize = 32;

fn drain<T>(wheel: &mut Wheel<T>, now: Instant, buf: &mut Vec<T>) {
    let fired = wheel.poll_with_limit(now, MAX_TIMERS_PER_TICK, buf);
    if fired == MAX_TIMERS_PER_TICK {
        // Hit the budget — remaining timers will fire on the next iteration
        // with the *same* `now`, preserving the fair-share property.
    }
}
```

The next call to `poll_with_limit` with the same `now` resumes where the
previous call stopped, so you don't starve any slot.

## Cancellable retries

Combine `reschedule` with a retry counter for exponential-backoff
reconnects:

```rust
use std::time::{Duration, Instant};
use nexus_timer::{Wheel, TimerHandle};

pub struct ReconnectState {
    pub handle: TimerHandle<ConnectionId>,
    pub attempts: u32,
}
# #[derive(Clone, Copy)] pub struct ConnectionId(u64);

pub fn bump_retry(
    wheel: &mut Wheel<ConnectionId>,
    state: ReconnectState,
    now: Instant,
) -> ReconnectState {
    let delay_ms = 100u64 << state.attempts.min(10);  // cap at 100s
    let handle = wheel.reschedule(
        state.handle,
        now + Duration::from_millis(delay_ms),
    );
    ReconnectState { handle, attempts: state.attempts + 1 }
}
```

`reschedule` is cheaper than `cancel` + `schedule` because it doesn't
construct a new entry or touch the allocator.
