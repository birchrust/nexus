# Patterns

## 1. WebSocket archival (off the hot path)

The feed handler parses a WebSocket frame, dispatches it to the
matching engine, and archives the raw bytes. Archival must not
slow down dispatch.

```rust
use nexus_logbuf::queue::spsc;

// 4 MiB archival buffer is plenty for most feeds.
let (mut archive_tx, mut archive_rx) = spsc::new(4 * 1024 * 1024);

// Feed handler (hot path):
fn on_frame(raw: &[u8], archive_tx: &mut spsc::Producer) {
    // 1. Dispatch to matching engine (not shown).
    // ...

    // 2. Archive the raw bytes. If the archive falls behind,
    //    drop rather than block the feed handler.
    if let Ok(mut claim) = archive_tx.try_claim(raw.len()) {
        claim.copy_from_slice(raw);
        claim.commit();
    } else {
        // Record the drop as a health metric.
        // archive_dropped.fetch_add(1, Relaxed);
    }
}

// Archival thread:
fn archival_loop(mut archive_rx: spsc::Consumer, mut file: std::fs::File) {
    use std::io::Write;
    loop {
        if let Some(rec) = archive_rx.try_claim() {
            let _ = file.write_all(&rec);
        } else {
            // Idle: yield or spin depending on latency budget.
            std::thread::yield_now();
        }
    }
}
```

Key points:
- Feed handler never blocks on archive I/O.
- Archive falls behind → drops, not stalls.
- Frame bytes flow through without allocation or
  serialization — they're already bytes.

## 2. Event sourcing journal

A trading system that journals every input event (market data,
orders, control messages) into a replay log.

```rust
use nexus_logbuf::channel::mpsc;
use std::thread;
use std::time::Duration;

let (journal_tx, mut journal_rx) = mpsc::new(16 * 1024 * 1024);

// Three input threads: market data, order entry, control plane.
// Each clones the sender.
let md_tx = journal_tx.clone();
thread::spawn(move || {
    let mut md_tx = md_tx;
    let event = b"\x01MD...";
    if let Ok(mut c) = md_tx.send(event.len()) {
        c.copy_from_slice(event);
        c.commit();
        md_tx.notify();
    }
});

// Journal writer thread:
thread::spawn(move || {
    loop {
        match journal_rx.recv(Some(Duration::from_millis(10))) {
            Ok(rec) => {
                // append_to_journal(&rec);
                let _ = &*rec;
            }
            Err(nexus_logbuf::channel::mpsc::RecvError::Timeout) => continue,
            Err(_) => break,
        }
    }
});

drop(journal_tx);
```

The high bit of the first payload byte could encode the event
type (market data, order, control). On replay, the same parser
handles any event.

## 3. FIX message archival

Each session generates FIX messages of varying lengths (20
bytes to 2 KiB). Pool-per-session plus an MPSC logbuf to a
single archival sink.

```rust
use nexus_logbuf::channel::mpsc;

let (fix_tx, _fix_rx) = mpsc::new(8 * 1024 * 1024);

fn on_fix_message(msg: &[u8], fix_tx: &mut mpsc::Sender) {
    if let Ok(mut claim) = fix_tx.try_send(msg.len()) {
        claim.copy_from_slice(msg);
        claim.commit();
        fix_tx.notify();
    } else {
        // Log a drop; consider per-session drop counters.
    }
}
# let _ = fix_tx;
```

For FIX you usually want to frame each record with a sequence
number so you can replay from any point. Prepend 8 bytes of
sequence before the raw FIX message in the same claim.

## 4. Structured binary logging

Not text logs — binary telemetry. Tracing spans, metrics deltas,
flame graph samples, etc. Very hot path, can't afford `println!`.

```rust
use nexus_logbuf::queue::spsc;

#[repr(C)]
#[derive(Clone, Copy)]
struct TraceEvent {
    ts_ns: u64,
    thread_id: u32,
    event_id: u16,
    _pad: u16,
    payload: [u8; 16],
}

let (mut tx, _rx) = spsc::new(1 * 1024 * 1024);

fn record_trace(tx: &mut spsc::Producer, ev: &TraceEvent) {
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (ev as *const TraceEvent) as *const u8,
            std::mem::size_of::<TraceEvent>(),
        )
    };
    if let Ok(mut claim) = tx.try_claim(bytes.len()) {
        claim.copy_from_slice(bytes);
        claim.commit();
    }
}
# let _ = tx;
```

For variable-length structured records, prefix each with a
type tag and length, or use a format like Cap'n Proto
(zero-copy encode into the claim buffer).

## 5. Pairing with nexus-net

`nexus-net` is sans-IO, which means it gives you callbacks with
`&[u8]` references to frame payloads. These are the perfect
input for logbuf archival.

```rust
// Pseudo-code illustrating the flow.
fn on_ws_message(payload: &[u8], archive_tx: &mut nexus_logbuf::queue::spsc::Producer) {
    // 1. Business logic with payload (still borrowed).
    // dispatch(payload);

    // 2. Archive the raw bytes. No copy beyond the archive.
    if let Ok(mut claim) = archive_tx.try_claim(payload.len()) {
        claim.copy_from_slice(payload);
        claim.commit();
    }
}
```

The full pipeline: `nexus-net` parses WebSocket frames zero-copy
→ handler dispatches the decoded message → logbuf archives the
raw bytes. No allocation anywhere.
