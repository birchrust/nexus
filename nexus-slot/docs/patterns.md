# Patterns

## Latest top-of-book from an exchange feed

The canonical use case. The market data thread parses a firehose of
updates; the strategy thread only cares about the current best bid/ask.

```rust
use nexus_slot::spsc::{slot, Reader, Writer};

#[derive(Copy, Clone, Default)]
pub struct TopOfBook {
    pub bid_px: f64,
    pub bid_qty: f64,
    pub ask_px: f64,
    pub ask_qty: f64,
    pub seq: u64,
}

pub struct MarketDataProducer { w: Writer<TopOfBook> }
pub struct StrategyConsumer    { r: Reader<TopOfBook> }

pub fn channel() -> (MarketDataProducer, StrategyConsumer) {
    let (w, r) = slot::<TopOfBook>();
    (MarketDataProducer { w }, StrategyConsumer { r })
}

impl MarketDataProducer {
    pub fn on_update(&mut self, tob: TopOfBook) {
        self.w.write(tob);  // always succeeds
    }
}

impl StrategyConsumer {
    pub fn latest(&mut self) -> Option<TopOfBook> {
        self.r.read()
    }
}
```

The strategy thread never sees partial updates, never applies backpressure
to the feed, and is always looking at the freshest quote.

## Fanout to multiple strategies

Multiple strategies want the same book snapshot:

```rust
use nexus_slot::spmc::shared_slot;
# #[derive(Copy, Clone, Default)] pub struct TopOfBook { pub bid_px: f64, pub ask_px: f64, pub seq: u64 }

let (mut writer, reader) = shared_slot::<TopOfBook>();

let strat_a = reader.clone();
let strat_b = reader.clone();
let strat_c = reader;  // original goes to the last consumer

// Each strategy sees every new value exactly once.
```

## Pair with notify for "latest + wakeup"

A slot tells you the latest value. A notify token tells you *when* a new
value is ready. Together they give you an event-driven "wake me when
there's fresh data, then read the latest":

```rust
use nexus_slot::spsc::slot;
use nexus_notify::{event_queue, Token};
# #[derive(Copy, Clone, Default)] pub struct Quote { pub bid: f64, pub ask: f64 }

let (mut q_writer, mut q_reader)   = slot::<Quote>();
let (notifier, poller) = event_queue(1);
let tok = Token::new(0);

// Producer:
std::thread::spawn(move || {
    q_writer.write(Quote { bid: 100.0, ask: 100.05 });
    notifier.notify(tok).unwrap();
});

// Consumer:
let mut events = nexus_notify::Events::with_capacity(1);
poller.poll(&mut events);
if !events.is_empty() {
    if let Some(latest) = q_reader.read() {
        // handle latest
    }
}
```

The notify token gives the consumer a wakeup without having to spin on
`has_update()`. The slot guarantees it sees the *latest* quote on wakeup,
not an arbitrary historical one.

## Configuration broadcast

A config thread wants to publish settings updates to one or more workers.
Workers read at their own pace — they don't need every intermediate edit,
just the latest committed settings.

```rust
use nexus_slot::spmc::shared_slot;

#[derive(Copy, Clone, Default)]
pub struct RiskConfig {
    pub max_position: f64,
    pub max_order_qty: f64,
    pub kill_switch: bool,
    pub version: u64,
}

let (mut config_writer, config_reader) = shared_slot::<RiskConfig>();
let worker_a_cfg = config_reader.clone();
let worker_b_cfg = config_reader.clone();

// Operator pushes a new config
config_writer.write(RiskConfig { max_position: 1_000_000.0, kill_switch: false, version: 1, ..Default::default() });

// Each worker re-reads at the top of its iteration
fn worker_step(r: &mut nexus_slot::spmc::SharedReader<RiskConfig>) {
    if let Some(cfg) = r.read() {
        // apply new settings
        let _ = cfg;
    }
    // ... do work with current settings ...
}
```

Workers running at different speeds all converge on the latest
configuration without coordination.

## Gauges and health snapshots

Internal metrics exported to a monitoring thread:

```rust
use nexus_slot::spsc::slot;

#[derive(Copy, Clone, Default)]
pub struct Gauges {
    pub orders_inflight: u32,
    pub ws_msgs_per_sec: f32,
    pub event_loop_p99_ns: u64,
}

let (mut gauge_w, mut gauge_r) = slot::<Gauges>();

// Hot loop writes occasionally
gauge_w.write(Gauges { orders_inflight: 42, ws_msgs_per_sec: 8_000.0, event_loop_p99_ns: 3_100 });

// Monitor thread reads when it wakes up
if let Some(snap) = gauge_r.read() {
    // export to prometheus, write to log, etc.
}
```

The hot loop pays essentially nothing for observability — a single
seqlock write. The monitor thread sees a consistent snapshot.
