# Patterns

## Dirty-set tracking in an event loop

An event loop processes many components per tick. Some components change
each tick, most don't. Rather than re-running everything, track which
components became dirty this tick and only process those.

```rust
use nexus_notify::{local::LocalNotify, Token, Events};

pub struct Engine {
    dirty: LocalNotify,
    events: Events,
    // ... component storage
}

impl Engine {
    pub fn mark_dirty(&mut self, token: Token) {
        self.dirty.mark(token);
    }

    pub fn tick(&mut self) {
        self.dirty.poll(&mut self.events);
        for evt in self.events.as_slice() {
            self.process(evt.index());
        }
    }

    fn process(&mut self, _idx: usize) { /* ... */ }
}
```

Duplicate marks within a tick are free — the second `mark` just sees the
flag already set.

## IO thread → strategy thread wakeups

The IO thread parses market data into conflation slots and notifies the
strategy thread that *something* changed.

```rust
use nexus_notify::{event_queue, Token, Events};

const MAX_SESSIONS: usize = 256;

let (notifier, poller) = event_queue(MAX_SESSIONS);
let mut events = Events::with_capacity(MAX_SESSIONS);

// IO thread:
let notifier_a = notifier.clone();
std::thread::spawn(move || {
    loop {
        let session_idx = read_ws_frame();  // pseudo
        // ... update the conflation slot for this session ...
        notifier_a.notify(Token::new(session_idx)).unwrap();
    }
});

// Strategy thread:
loop {
    events.clear();
    poller.poll(&mut events);
    for evt in events.as_slice() {
        process_session(evt.index());
    }
}
# fn read_ws_frame() -> usize { 0 }
# fn process_session(_: usize) {}
```

The strategy thread is woken up per-session exactly once per tick even
if the IO thread fired 50 updates for the same session in the meantime.

## Budgeted dispatch for fair-share

When many tokens are ready at once, dispatch in chunks to keep tail
latency bounded and prevent any single tick from monopolizing the event
loop.

```rust
use nexus_notify::{event_queue, Token, Events};

let (notifier, poller) = event_queue(4096);
let mut events = Events::with_capacity(4096);

const MAX_PER_TICK: usize = 32;

loop {
    events.clear();
    poller.poll_limit(&mut events, MAX_PER_TICK);
    for evt in events.as_slice() {
        handle(evt.index());
    }
    // ... other work in this tick ...
    do_other_work();
}
# fn handle(_: usize) {}
# fn do_other_work() {}
```

Because `poll_limit` preserves FIFO order, older tokens drain first — no
starvation.

## Pairing with `nexus-slot` for "latest data + wakeup"

The common idiom: producer writes the latest value into a
[slot](../../nexus-slot), then notifies a token. Consumer wakes up and
reads the slot. Multiple writes between polls → one wakeup, one read of
the most recent value.

```rust
use nexus_notify::{event_queue, Token, Events};
use nexus_slot::spsc::slot;

#[derive(Copy, Clone, Default)]
struct Quote { bid: f64, ask: f64, seq: u64 }

let (mut quote_w, mut quote_r) = slot::<Quote>();
let (notifier, poller) = event_queue(1);
let mut events = Events::with_capacity(1);
let tok = Token::new(0);

// Producer:
std::thread::spawn(move || {
    for seq in 0.. {
        quote_w.write(Quote { bid: 100.0, ask: 100.05, seq });
        notifier.notify(tok).ok();
    }
});

// Consumer:
loop {
    events.clear();
    poller.poll(&mut events);
    if !events.is_empty() {
        if let Some(q) = quote_r.read() {
            process(q);
        }
    }
}
# fn process(_: Quote) {}
```

The consumer may process far fewer quotes than the producer generates —
that's the point. The slot gives it the *latest* quote; the notify
guarantees it wakes up when one is available.

## Signal coalescing for expensive work

An external signal (reload config, rebuild index, reconcile state) that
shouldn't run more than once per polling cycle, even if requested many
times:

```rust
use nexus_notify::{event_queue, Token, Events};

#[repr(usize)]
enum Signal {
    ReloadConfig   = 0,
    RebuildIndex   = 1,
    ReconcileState = 2,
}

let (notifier, poller) = event_queue(3);
let mut events = Events::with_capacity(3);

// Any thread can request:
notifier.notify(Token::new(Signal::ReloadConfig as usize)).unwrap();
notifier.notify(Token::new(Signal::ReloadConfig as usize)).unwrap();  // dedup
notifier.notify(Token::new(Signal::RebuildIndex as usize)).unwrap();

// Control thread handles at most once per tick:
events.clear();
poller.poll(&mut events);
for evt in events.as_slice() {
    match evt.index() {
        0 => reload_config(),
        1 => rebuild_index(),
        2 => reconcile_state(),
        _ => {}
    }
}
# fn reload_config() {} fn rebuild_index() {} fn reconcile_state() {}
```

Two notifications for `ReloadConfig` collapse into one call. Three
different signals fire three times. Perfect coalescing with zero
per-signal scheduling machinery.
