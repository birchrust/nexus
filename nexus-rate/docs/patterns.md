# Patterns

## 1. Exchange order rate limit (GCRA)

CME-style: 500 new orders per second per session, with a small
burst allowance for the head of the session.

```rust
use nexus_rate::local::Gcra;
use std::time::{Duration, Instant};

pub struct Session {
    order_limiter: Gcra,
    // ... other session fields
}

impl Session {
    pub fn new(now: Instant) -> Self {
        Self {
            order_limiter: Gcra::builder()
                .rate(500)
                .period(Duration::from_secs(1))
                .burst(50)              // 50-order head burst
                .now(now)
                .build()
                .unwrap(),
        }
    }

    pub fn submit_order(&mut self, now: Instant) -> Result<(), &'static str> {
        if !self.order_limiter.try_acquire(1, now) {
            return Err("rate limited");
        }
        // ... actually send the order ...
        Ok(())
    }

    pub fn on_reject(&mut self, now: Instant) {
        // Exchange rejected the order → rebate capacity.
        self.order_limiter.release(1, now);
    }
}
```

The limiter lives inside the session (per-thread, single-owner),
so we use `local::Gcra` — no atomics, ~5-10 cycles per check.

## 2. API client throttling (Token Bucket)

Binance-style: weighted REST requests, 1200 weight/minute,
bucket capacity 1200.

```rust
use nexus_rate::local::TokenBucket;
use std::time::{Duration, Instant};

let mut rest_limiter = TokenBucket::builder()
    .rate(1200)
    .period(Duration::from_secs(60))
    .burst(1200)                 // full minute of burst
    .build()
    .unwrap();

enum Endpoint {
    Ticker,      // weight 1
    Depth,       // weight 5
    AccountInfo, // weight 10
    AllOrders,   // weight 40
}

fn endpoint_weight(e: Endpoint) -> u64 {
    match e {
        Endpoint::Ticker => 1,
        Endpoint::Depth => 5,
        Endpoint::AccountInfo => 10,
        Endpoint::AllOrders => 40,
    }
}

fn call(limiter: &mut TokenBucket, ep: Endpoint) -> bool {
    let now = Instant::now();
    let weight = endpoint_weight(ep);
    if limiter.try_acquire(weight, now) {
        // ... make the HTTP call ...
        true
    } else {
        false
    }
}
# let _ = (rest_limiter, call);
```

Weighted cost is the key feature here. One limiter handles all
endpoints; each call passes its weight.

## 3. Per-client rate limit (Sliding Window)

Your service exposes a REST API. Each client is limited to 100
requests per minute, enforced as an exact count within a
sliding window.

```rust
use nexus_rate::local::SlidingWindow;
use std::collections::HashMap;
use std::time::{Duration, Instant};

struct ClientLimiter {
    limiters: HashMap<u64, SlidingWindow>,
}

impl ClientLimiter {
    fn new() -> Self {
        Self { limiters: HashMap::new() }
    }

    fn check(&mut self, client_id: u64, now: Instant) -> bool {
        let limiter = self.limiters.entry(client_id).or_insert_with(|| {
            SlidingWindow::builder()
                .window(Duration::from_secs(60))
                .sub_windows(10)
                .limit(100)
                .now(now)
                .build()
                .unwrap()
        });
        limiter.try_acquire(1, now)
    }
}
```

One limiter per client, all living on the same thread that
handles that client's requests. No sync, no atomics, O(1) per
check.

If the client count grows unbounded, periodically evict idle
entries (e.g., clients that haven't made a request in 10
minutes).

## 4. Global shared limit via dedicated thread

You have a single API key across many workers, and the
exchange limits are per-key. You want one limiter across all
workers, but you don't want the contention of `sync::Gcra`.

```rust
use nexus_rate::local::Gcra;
use nexus_queue::mpsc;
use std::thread;
use std::time::{Duration, Instant};

struct Request;
struct Response { allowed: bool }

let (req_tx, req_rx) = mpsc::ring_buffer::<Request>(1024);
let (resp_tx, _resp_rx) = mpsc::ring_buffer::<Response>(1024);

let limiter_thread = thread::spawn(move || {
    let mut limiter = Gcra::builder()
        .rate(500).period(Duration::from_secs(1)).burst(50)
        .build().unwrap();

    while let Some(_req) = req_rx.pop() {
        let allowed = limiter.try_acquire(1, Instant::now());
        let _ = resp_tx.push(Response { allowed });
        # break;
    }
});

// Workers:
// let _ = req_tx.push(Request);
// let resp = resp_rx.pop();

# drop(req_tx);
# limiter_thread.join().unwrap();
```

The dedicated limiter thread serializes access without atomics
on the hot path. Each check is one queue round-trip — more
expensive than a local check but more predictable under load
than atomic contention.

## 5. Weighted request fan-in

Orders with different costs (limit orders = weight 1,
multi-leg options = weight 5, algorithmic orders = weight 2).

```rust
use nexus_rate::local::Gcra;
use std::time::{Duration, Instant};

enum OrderKind { Limit, Algo, MultiLeg }

fn weight(k: OrderKind) -> u64 {
    match k {
        OrderKind::Limit => 1,
        OrderKind::Algo => 2,
        OrderKind::MultiLeg => 5,
    }
}

let mut limiter = Gcra::builder()
    .rate(1000).period(Duration::from_secs(1)).burst(100)
    .build().unwrap();

fn submit(limiter: &mut Gcra, kind: OrderKind) -> bool {
    limiter.try_acquire(weight(kind), Instant::now())
}
# let _ = submit(&mut limiter, OrderKind::Limit);
```

Weighted request handling is built into the `cost` parameter —
no special configuration needed.

## 6. Rate-limit aware backoff

When rate limited, you want to know **how long** to wait before
retrying. GCRA provides `time_until_allowed`:

```rust
use nexus_rate::local::Gcra;
use std::time::{Duration, Instant};

let mut limiter = Gcra::builder()
    .rate(10).period(Duration::from_secs(1))
    .build().unwrap();

fn try_with_backoff(limiter: &mut Gcra) {
    let now = Instant::now();
    if limiter.try_acquire(1, now) {
        // proceed
        return;
    }
    let wait = limiter.time_until_allowed(1, now);
    // sleep or schedule a timer for `wait`
    let _ = wait;
}
# try_with_backoff(&mut limiter);
```

This beats blind exponential backoff — you know exactly when
the limiter will admit you.
